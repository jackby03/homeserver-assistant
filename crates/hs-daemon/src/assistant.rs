//! Local, offline execution of Spanish assistant intents.
//!
//! The kiosk/voice flow first calls [`try_local`]: everyday commands are answered
//! here without an LLM, network or provider latency. Anything not recognized
//! returns `None` and falls through to the LLM agent.
//!
//! Security: state-changing intents require the same tool permissions as the
//! agent (`profile.may_use_tool`); a denied action still answers locally instead
//! of reaching the LLM.

use chrono::{Datelike, Local, Timelike};
use hs_core::assistant::{normalize, parse_local_intent, LocalIntent};
use hs_core::models::Profile;
use std::time::Duration;

use crate::state::AppState;

/// A locally produced assistant answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalReply {
    /// Text to show and speak.
    pub reply: String,
    /// Intent id for auditing.
    pub intent: &'static str,
    /// Whether the action was refused due to profile permissions.
    pub denied: bool,
}

const JOKES: &[&str] = &[
    "¿Por qué los ordenadores nunca tienen frío? Porque tienen Windows.",
    "Un byte entra en un bar y pide una cerveza. El camarero le dice: ¿de 8 o de 16?",
    "¿Qué le dice un bit a otro? Nos vemos en el bus.",
    "No entiendo por qué la impresora me mira mal... será que tiene papel atascado.",
];

const CAPABILITIES: &str = "Puedo responder localmente, sin internet ni IA: \
hora y fecha, el clima actual de Lima o de otra ciudad, subir o bajar el volumen, \
poner o pausar música, encender o apagar luces, activar escenas, cálculos sencillos, \
chistes y preguntas sobre mí. Si me preguntas algo más complejo, uso el modelo de IA configurado.";

/// Execute a local intent if the utterance matches one.
pub async fn try_local(state: &AppState, text: &str, profile: &Profile) -> Option<LocalReply> {
    let intent = parse_local_intent(text)?;
    let assistant_name = state.config.snapshot().voice.assistant_name;
    let reply = match &intent {
        LocalIntent::Greeting => format!("Hola, {}. ¿En qué te puedo ayudar?", profile.name),
        LocalIntent::Identity => format!(
            "Soy {assistant_name}, el asistente local de HomeServer Assistant OS. \
La mayoría de órdenes básicas las resuelvo aquí mismo, sin enviar nada a internet."
        ),
        LocalIntent::Capabilities => CAPABILITIES.to_owned(),
        LocalIntent::Thanks => format!("De nada, {}.", profile.name),
        LocalIntent::Farewell => format!("Hasta luego, {}.", profile.name),
        LocalIntent::Weather { city } => weather_reply(city.as_deref()).await,
        LocalIntent::Wellbeing => format!(
            "Estoy muy bien, gracias por preguntar, {}. ¿Y tú cómo estás?",
            profile.name
        ),
        LocalIntent::Time => {
            let now = Local::now();
            format!("Son las {:02}:{:02}.", now.hour(), now.minute())
        }
        LocalIntent::Date => {
            let now = Local::now();
            let weekdays = [
                "lunes",
                "martes",
                "miércoles",
                "jueves",
                "viernes",
                "sábado",
                "domingo",
            ];
            let months = [
                "enero",
                "febrero",
                "marzo",
                "abril",
                "mayo",
                "junio",
                "julio",
                "agosto",
                "septiembre",
                "octubre",
                "noviembre",
                "diciembre",
            ];
            let weekday = weekdays
                .get(now.weekday().num_days_from_monday() as usize)
                .copied()
                .unwrap_or("hoy");
            let month = months.get(now.month0() as usize).copied().unwrap_or("mes");
            format!(
                "Hoy es {weekday}, {} de {month} de {}.",
                now.day(),
                now.year()
            )
        }
        LocalIntent::VolumeSet(_) | LocalIntent::VolumeStep(_) | LocalIntent::Mute => {
            if denied(profile, &intent) {
                return Some(denied_reply(&intent));
            }
            let current = state.kiosk.snapshot().music.volume;
            let target = match &intent {
                LocalIntent::VolumeSet(percent) => *percent,
                LocalIntent::VolumeStep(step) => (i32::from(current) + i32::from(*step))
                    .clamp(0, 100)
                    .try_into()
                    .unwrap_or(current),
                LocalIntent::Mute => 0,
                _ => current,
            };
            if let Err(error) = state.system.set_volume(target) {
                format!("No pude cambiar el volumen: {error}.")
            } else {
                let mut music = state.kiosk.snapshot().music;
                music.volume = target;
                state.kiosk.set_music(music);
                if matches!(intent, LocalIntent::Mute) {
                    "He silenciado el sonido.".to_owned()
                } else {
                    format!("Volumen al {target} por ciento.")
                }
            }
        }
        LocalIntent::MusicPlay | LocalIntent::MusicPause => {
            if denied(profile, &intent) {
                return Some(denied_reply(&intent));
            }
            let Some(entity_id) = first_media_player(state).await else {
                return Some(LocalReply {
                    reply: "No hay ningún reproductor de música configurado. \
Configura Home Assistant o Music Assistant para usar este comando."
                        .to_owned(),
                    intent: intent.id(),
                    denied: false,
                });
            };
            let service = if matches!(intent, LocalIntent::MusicPlay) {
                "media_play"
            } else {
                "media_pause"
            };
            match state
                .ha
                .call_service(
                    "media_player",
                    service,
                    serde_json::json!({ "entity_id": entity_id }),
                )
                .await
            {
                Ok(()) => {
                    let mut music = state.kiosk.snapshot().music;
                    music.playing = matches!(intent, LocalIntent::MusicPlay);
                    state.kiosk.set_music(music);
                    if matches!(intent, LocalIntent::MusicPlay) {
                        "Reproduciendo música.".to_owned()
                    } else {
                        "Música en pausa.".to_owned()
                    }
                }
                Err(error) => format!("No pude controlar la música: {error}."),
            }
        }
        LocalIntent::MusicStatus => {
            let music = state.kiosk.snapshot().music;
            if !music.playing {
                "No hay música sonando ahora mismo.".to_owned()
            } else {
                match (&music.track, &music.artist) {
                    (Some(track), Some(artist)) => format!(
                        "Suena «{track}» de {artist}, al {} por ciento.",
                        music.volume
                    ),
                    (Some(track), None) => {
                        format!("Suena «{track}» al {} por ciento.", music.volume)
                    }
                    _ => format!("Música reproduciéndose al {} por ciento.", music.volume),
                }
            }
        }
        LocalIntent::Joke => {
            let index = Local::now().timestamp().unsigned_abs() as usize % JOKES.len();
            JOKES
                .get(index)
                .copied()
                .unwrap_or("No me sé ningún chiste.")
                .to_owned()
        }
        LocalIntent::Math { expression, result } => {
            format!("{expression} es {result}.")
        }
        LocalIntent::LightsOn(target) | LocalIntent::LightsOff(target) => {
            if denied(profile, &intent) {
                return Some(denied_reply(&intent));
            }
            let turn_on = matches!(intent, LocalIntent::LightsOn(_));
            lights_reply(state, target.as_deref(), turn_on).await
        }
        LocalIntent::SceneActivate(name) => {
            if denied(profile, &intent) {
                return Some(denied_reply(&intent));
            }
            scene_reply(state, name).await
        }
        LocalIntent::LightsStatus => {
            if denied(profile, &intent) {
                return Some(denied_reply(&intent));
            }
            lights_status_reply(state).await
        }
    };
    Some(LocalReply {
        reply,
        intent: intent.id(),
        denied: false,
    })
}

fn denied(profile: &Profile, intent: &LocalIntent) -> bool {
    intent
        .required_tool()
        .is_some_and(|tool| !profile.may_use_tool(tool))
}

fn denied_reply(intent: &LocalIntent) -> LocalReply {
    let what = match intent {
        LocalIntent::VolumeSet(_) | LocalIntent::VolumeStep(_) | LocalIntent::Mute => {
            "cambiar el volumen"
        }
        LocalIntent::MusicPlay | LocalIntent::MusicPause => "controlar la música",
        LocalIntent::LightsOn(_) | LocalIntent::LightsOff(_) | LocalIntent::LightsStatus => {
            "controlar las luces"
        }
        LocalIntent::SceneActivate(_) => "activar escenas",
        _ => "ejecutar esa acción",
    };
    LocalReply {
        reply: format!(
            "Tu perfil no tiene permiso para {what}. Un administrador puede habilitarlo en Perfiles."
        ),
        intent: intent.id(),
        denied: true,
    }
}

async fn weather_reply(city: Option<&str>) -> String {
    let requested = city
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("Lima");
    let client = match reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(10))
        .build()
    {
        Ok(client) => client,
        Err(_) => return "No pude preparar la consulta del clima.".to_owned(),
    };

    let (name, latitude, longitude) = if requested.eq_ignore_ascii_case("lima") {
        ("Lima".to_owned(), -12.04318_f64, -77.02824_f64)
    } else {
        match geocode_city(&client, requested).await {
            Some(location) => location,
            None => {
                return format!(
                    "No encontré la ciudad «{requested}». Prueba con otro nombre o revisa la conexión."
                );
            }
        }
    };

    let params = [
        ("latitude".to_owned(), latitude.to_string()),
        ("longitude".to_owned(), longitude.to_string()),
        (
            "current".to_owned(),
            "temperature_2m,relative_humidity_2m,apparent_temperature,weather_code,precipitation"
                .to_owned(),
        ),
        ("timezone".to_owned(), "auto".to_owned()),
    ];
    let response = match client
        .get("https://api.open-meteo.com/v1/forecast")
        .query(&params)
        .send()
        .await
    {
        Ok(response) => response,
        Err(_) => {
            return format!(
                "No pude consultar el clima de {name} ahora mismo. Revisa la conexión a internet."
            );
        }
    };
    let body = match response.json::<serde_json::Value>().await {
        Ok(body) => body,
        Err(_) => {
            return format!("El servicio del clima devolvió una respuesta inválida para {name}.");
        }
    };
    let Some(current) = body.get("current") else {
        return format!("No recibí datos actuales del clima para {name}.");
    };
    let Some(temperature) = current
        .get("temperature_2m")
        .and_then(|value| value.as_f64())
    else {
        return format!("No pude leer la temperatura actual de {name}.");
    };
    let apparent = current
        .get("apparent_temperature")
        .and_then(|value| value.as_f64())
        .unwrap_or(temperature);
    let humidity = current
        .get("relative_humidity_2m")
        .and_then(|value| value.as_f64())
        .unwrap_or(0.0);
    let weather_code = current
        .get("weather_code")
        .and_then(|value| value.as_i64())
        .unwrap_or(0);
    let precipitation = current
        .get("precipitation")
        .and_then(|value| value.as_f64())
        .unwrap_or(0.0);
    let description = weather_description(weather_code);
    let mut reply = format!(
        "En {name} hay {temperature:.1} grados Celsius, con sensación de {apparent:.1} grados \
y humedad del {humidity:.0} por ciento. {description}."
    );
    if precipitation > 0.0 {
        reply.push_str(&format!(
            " Hay {precipitation:.1} milímetros de precipitación."
        ));
    }
    reply
}

async fn geocode_city(client: &reqwest::Client, city: &str) -> Option<(String, f64, f64)> {
    let params = [
        ("name".to_owned(), city.to_owned()),
        ("count".to_owned(), "1".to_owned()),
        ("language".to_owned(), "es".to_owned()),
        ("format".to_owned(), "json".to_owned()),
    ];
    let response = client
        .get("https://geocoding-api.open-meteo.com/v1/search")
        .query(&params)
        .send()
        .await
        .ok()?;
    let body = response.json::<serde_json::Value>().await.ok()?;
    let result = body.get("results")?.as_array()?.first()?;
    let name = result.get("name")?.as_str()?.to_owned();
    let latitude = result.get("latitude")?.as_f64()?;
    let longitude = result.get("longitude")?.as_f64()?;
    Some((name, latitude, longitude))
}

fn weather_description(code: i64) -> &'static str {
    match code {
        0 => "El cielo está despejado",
        1 => "El cielo está mayormente despejado",
        2 => "El cielo está parcialmente nublado",
        3 => "El cielo está nublado",
        45 | 48 => "Hay niebla",
        51 | 53 | 55 => "Hay llovizna",
        56 | 57 => "Hay llovizna helada",
        61 | 63 | 65 => "Hay lluvia",
        66 | 67 => "Hay lluvia helada",
        71 | 73 | 75 => "Hay nieve",
        77 => "Hay granos de nieve",
        80..=82 => "Hay chubascos",
        85 | 86 => "Hay chubascos de nieve",
        95 => "Hay tormenta",
        96 | 99 => "Hay tormenta con granizo",
        _ => "Las condiciones son variables",
    }
}

async fn first_media_player(state: &AppState) -> Option<String> {
    let entities = state.ha.entities().await.ok()?;
    entities
        .into_iter()
        .find(|entity| entity.domain == "media_player")
        .map(|entity| entity.entity_id)
}

async fn lights_reply(state: &AppState, target: Option<&str>, turn_on: bool) -> String {
    let entities = match state.ha.entities().await {
        Ok(entities) => entities,
        Err(_) => {
            return "Home Assistant no está configurado; no puedo controlar las luces.".to_owned();
        }
    };
    let target = target.map(normalize);
    let lights: Vec<_> = entities
        .into_iter()
        .filter(|entity| entity.domain == "light")
        .filter(|entity| match &target {
            None => true,
            Some(target) => {
                normalize(&entity.name).contains(target)
                    || normalize(&entity.entity_id).contains(target)
            }
        })
        .collect();
    if lights.is_empty() {
        return match target {
            Some(target) => format!("No encontré ninguna luz llamada «{target}»."),
            None => "No hay luces configuradas en Home Assistant.".to_owned(),
        };
    }
    let ids: Vec<String> = lights
        .iter()
        .map(|entity| entity.entity_id.clone())
        .collect();
    let service = if turn_on { "turn_on" } else { "turn_off" };
    match state
        .ha
        .call_service("light", service, serde_json::json!({ "entity_id": ids }))
        .await
    {
        Ok(()) => {
            let names: Vec<&str> = lights.iter().map(|entity| entity.name.as_str()).collect();
            if turn_on {
                format!("He encendido {}.", names.join(", "))
            } else {
                format!("He apagado {}.", names.join(", "))
            }
        }
        Err(error) => format!("No pude cambiar las luces: {error}."),
    }
}

async fn scene_reply(state: &AppState, name: &str) -> String {
    let entities = match state.ha.entities().await {
        Ok(entities) => entities,
        Err(_) => {
            return "Home Assistant no está configurado; no puedo activar escenas.".to_owned();
        }
    };
    let wanted = normalize(name);
    let scene = entities
        .into_iter()
        .filter(|entity| entity.domain == "scene")
        .find(|entity| {
            normalize(&entity.name).contains(&wanted)
                || normalize(&entity.entity_id).contains(&wanted)
        });
    let Some(scene) = scene else {
        return format!("No encontré ninguna escena llamada «{name}».");
    };
    match state
        .ha
        .call_service(
            "scene",
            "turn_on",
            serde_json::json!({ "entity_id": scene.entity_id }),
        )
        .await
    {
        Ok(()) => format!("Escena «{}» activada.", scene.name),
        Err(error) => format!("No pude activar la escena: {error}."),
    }
}

async fn lights_status_reply(state: &AppState) -> String {
    let entities = match state.ha.entities().await {
        Ok(entities) => entities,
        Err(_) => {
            return "Home Assistant no está configurado; no puedo consultar las luces.".to_owned();
        }
    };
    let on: Vec<&str> = entities
        .iter()
        .filter(|entity| entity.domain == "light" && entity.state == "on")
        .map(|entity| entity.name.as_str())
        .collect();
    if on.is_empty() {
        "No hay ninguna luz encendida.".to_owned()
    } else {
        format!("Están encendidas: {}.", on.join(", "))
    }
}
