//! Local Spanish intent router for the voice/chat assistant.
//!
//! Siri/Alexa-style assistants answer most everyday requests locally: time,
//! date, volume, playback, lights/scenes, simple maths, identity and small talk.
//! This module is pure (no I/O, no async) so it is fully unit-testable; the
//! daemon executes the returned intent and only falls back to the LLM when no
//! local intent matches.
//!
//! Spanish is the primary language. Matching is accent/case/punctuation
//! insensitive.

/// Intent understood locally, without the LLM.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalIntent {
    /// "hola", "buenos días"...
    Greeting,
    /// "¿quién eres?", "¿cómo te llamas?"
    Identity,
    /// "¿qué puedes hacer?", "ayuda", "comandos"
    Capabilities,
    /// "gracias"
    Thanks,
    /// "adiós", "hasta luego"
    Farewell,
    /// Local clock time.
    Time,
    /// Local date.
    Date,
    /// Set the speaker volume to an absolute percentage.
    VolumeSet(u8),
    /// Raise (positive) or lower (negative) the volume by a step.
    VolumeStep(i16),
    /// Mute (volume 0).
    Mute,
    /// Start playback.
    MusicPlay,
    /// Pause playback.
    MusicPause,
    /// "¿qué está sonando?"
    MusicStatus,
    /// Tell a short joke.
    Joke,
    /// Turn lights on; target is the normalized device name, if any.
    LightsOn(Option<String>),
    /// Turn lights off.
    LightsOff(Option<String>),
    /// Activate a Home Assistant scene by name.
    SceneActivate(String),
    /// List lights that are currently on.
    LightsStatus,
    /// Safe arithmetic with a pre-computed result.
    Math { expression: String, result: i64 },
}

impl LocalIntent {
    /// Stable id used in audit logs and tests.
    #[must_use]
    pub fn id(&self) -> &'static str {
        match self {
            Self::Greeting => "greeting",
            Self::Identity => "identity",
            Self::Capabilities => "capabilities",
            Self::Thanks => "thanks",
            Self::Farewell => "farewell",
            Self::Time => "time",
            Self::Date => "date",
            Self::VolumeSet(_) => "volume_set",
            Self::VolumeStep(_) => "volume_step",
            Self::Mute => "mute",
            Self::MusicPlay => "music_play",
            Self::MusicPause => "music_pause",
            Self::MusicStatus => "music_status",
            Self::Joke => "joke",
            Self::LightsOn(_) => "lights_on",
            Self::LightsOff(_) => "lights_off",
            Self::SceneActivate(_) => "scene_activate",
            Self::LightsStatus => "lights_status",
            Self::Math { .. } => "math",
        }
    }

    /// Whether executing this intent changes device state (needs permission).
    #[must_use]
    pub fn is_action(&self) -> bool {
        matches!(
            self,
            Self::VolumeSet(_)
                | Self::VolumeStep(_)
                | Self::Mute
                | Self::MusicPlay
                | Self::MusicPause
                | Self::LightsOn(_)
                | Self::LightsOff(_)
                | Self::SceneActivate(_)
        )
    }

    /// Tool permission required to execute the intent, when it is an action.
    #[must_use]
    pub fn required_tool(&self) -> Option<&'static str> {
        match self {
            Self::VolumeSet(_) | Self::VolumeStep(_) | Self::Mute => Some("music.set_volume"),
            Self::MusicPlay => Some("music.play"),
            Self::MusicPause => Some("music.pause"),
            Self::LightsOn(_) | Self::LightsOff(_) | Self::SceneActivate(_) => {
                Some("homeassistant.call_service")
            }
            Self::LightsStatus => Some("homeassistant.list_entities"),
            _ => None,
        }
    }
}

/// Normalize text: lowercase, strip Spanish accents, drop punctuation.
#[must_use]
pub fn normalize(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    for character in text.chars() {
        let lowered = character.to_lowercase().next().unwrap_or(character);
        let mapped = match lowered {
            'á' | 'à' | 'ä' | 'â' => 'a',
            'é' | 'è' | 'ë' | 'ê' => 'e',
            'í' | 'ì' | 'ï' | 'î' => 'i',
            'ó' | 'ò' | 'ö' | 'ô' => 'o',
            'ú' | 'ù' | 'ü' | 'û' => 'u',
            'ñ' => 'n',
            other => other,
        };
        if mapped.is_ascii_alphanumeric()
            || mapped == ' '
            || mapped == '+'
            || mapped == '-'
            || mapped == '*'
            || mapped == '/'
            || mapped == '('
            || mapped == ')'
        {
            output.push(mapped);
        } else if mapped.is_whitespace()
            || mapped == ','
            || mapped == '.'
            || mapped == '?'
            || mapped == '!'
        {
            output.push(' ');
        }
    }
    output.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn words(text: &str) -> Vec<&str> {
    text.split_whitespace().collect()
}

fn contains(text: &str, needle: &str) -> bool {
    format!(" {text} ").contains(&format!(" {needle} "))
}

fn starts_with_any(text: &str, prefixes: &[&str]) -> bool {
    prefixes.iter().any(|prefix| text.starts_with(prefix))
}

fn extract_number(text: &str) -> Option<u8> {
    for token in words(text) {
        if let Ok(value) = token.parse::<u8>() {
            if value <= 100 {
                return Some(value);
            }
        }
    }
    None
}

fn strip_prefixes<'a>(text: &'a str, prefixes: &[&str]) -> &'a str {
    for prefix in prefixes {
        if let Some(rest) = text.strip_prefix(prefix) {
            return rest.trim();
        }
    }
    text.trim()
}

fn extract_target(text: &str) -> Option<String> {
    const PREFIXES: &[&str] = &[
        "enciende ",
        "encender ",
        "prende ",
        "prender ",
        "apaga ",
        "apagar ",
        "la luz ",
        "las luces ",
        "la luces ",
        "luz ",
        "luces ",
        "el ",
        "la ",
        "los ",
        "las ",
        "de ",
        "del ",
        "en ",
    ];
    let mut base = text.trim();
    loop {
        let mut changed = false;
        for prefix in PREFIXES {
            if let Some(rest) = base.strip_prefix(prefix) {
                base = rest.trim();
                changed = true;
                break;
            }
        }
        if !changed {
            break;
        }
    }
    let target = base
        .split_whitespace()
        .filter(|word| !matches!(*word, "de" | "del" | "en" | "la" | "el" | "los" | "las"))
        .collect::<Vec<_>>()
        .join(" ");
    let target = target.trim();
    if target.is_empty() || matches!(target, "luces" | "luz" | "todo" | "todas" | "todos") {
        None
    } else {
        Some(target.to_owned())
    }
}

fn is_greeting(text: &str) -> bool {
    if words(text).len() > 4 {
        return false;
    }
    starts_with_any(
        text,
        &[
            "hola",
            "buenos dias",
            "buenas tardes",
            "buenas noches",
            "hey",
            "oye",
            "saludos",
        ],
    )
}

/// Parse a Spanish utterance into a local intent.
///
/// Returns `None` when the request needs the LLM (weather, open questions,
/// general knowledge, complex home automation, ...).
#[must_use]
pub fn parse_local_intent(text: &str) -> Option<LocalIntent> {
    let text = normalize(text);
    if text.is_empty() {
        return None;
    }

    // Maths first: "¿cuánto es 2 más 2?".
    if contains(&text, "cuanto es")
        || contains(&text, "cuanto son")
        || starts_with_any(&text, &["calcula ", "calculame "])
    {
        if let Some((expression, result)) = evaluate_math(&text) {
            return Some(LocalIntent::Math { expression, result });
        }
    }

    if contains(&text, "que hora")
        || contains(&text, "que horas")
        || contains(&text, "horas son")
        || contains(&text, "dime la hora")
        || starts_with_any(&text, &["que hora", "que horas"])
        || words(&text)
            .iter()
            .any(|word| *word == "hora" || *word == "horas")
        || text == "hora"
    {
        return Some(LocalIntent::Time);
    }
    if contains(&text, "que dia es hoy")
        || contains(&text, "que dia estamos")
        || contains(&text, "fecha de hoy")
        || contains(&text, "dime la fecha")
    {
        return Some(LocalIntent::Date);
    }

    if contains(&text, "silencia")
        || contains(&text, "silencio")
        || text == "mute"
        || contains(&text, "quita el sonido")
    {
        return Some(LocalIntent::Mute);
    }
    if contains(&text, "volumen")
        || contains(&text, "sube el sonido")
        || contains(&text, "baja el sonido")
    {
        if let Some(percent) = extract_number(&text) {
            return Some(LocalIntent::VolumeSet(percent));
        }
        if contains(&text, "sube") || contains(&text, "aumenta") || contains(&text, "mas") {
            return Some(LocalIntent::VolumeStep(10));
        }
        if contains(&text, "baja") || contains(&text, "reduce") || contains(&text, "menos") {
            return Some(LocalIntent::VolumeStep(-10));
        }
    }

    if contains(&text, "pon musica")
        || contains(&text, "reproduce musica")
        || contains(&text, "play musica")
        || contains(&text, "pon la musica")
    {
        return Some(LocalIntent::MusicPlay);
    }
    if contains(&text, "pausa la musica")
        || contains(&text, "para la musica")
        || contains(&text, "pausa musica")
        || contains(&text, "deten la musica")
    {
        return Some(LocalIntent::MusicPause);
    }
    if contains(&text, "que musica suena")
        || contains(&text, "que esta sonando")
        || contains(&text, "que cancion suena")
    {
        return Some(LocalIntent::MusicStatus);
    }

    if contains(&text, "luces encendidas")
        || contains(&text, "que luces estan encendidas")
        || contains(&text, "luces estan prendidas")
    {
        return Some(LocalIntent::LightsStatus);
    }
    if contains(&text, "escena") {
        let scene = strip_prefixes(
            &text,
            &[
                "activa la escena ",
                "activar la escena ",
                "activa escena ",
                "activar escena ",
                "pon la escena ",
                "pon escena ",
            ],
        );
        if !scene.is_empty() {
            return Some(LocalIntent::SceneActivate(scene.to_owned()));
        }
    }
    if contains(&text, "apaga") && (contains(&text, "luz") || contains(&text, "luces")) {
        return Some(LocalIntent::LightsOff(extract_target(&text)));
    }
    if (contains(&text, "enciende") || contains(&text, "prende"))
        && (contains(&text, "luz") || contains(&text, "luces"))
    {
        return Some(LocalIntent::LightsOn(extract_target(&text)));
    }

    if contains(&text, "quien eres")
        || contains(&text, "como te llamas")
        || contains(&text, "cual es tu nombre")
        || contains(&text, "tu nombre")
    {
        return Some(LocalIntent::Identity);
    }
    if contains(&text, "que puedes hacer")
        || contains(&text, "que sabes hacer")
        || text == "ayuda"
        || contains(&text, "comandos")
    {
        return Some(LocalIntent::Capabilities);
    }
    if contains(&text, "gracias") {
        return Some(LocalIntent::Thanks);
    }
    if contains(&text, "adios")
        || contains(&text, "hasta luego")
        || contains(&text, "nos vemos")
        || contains(&text, "chao")
    {
        return Some(LocalIntent::Farewell);
    }
    if contains(&text, "chiste") {
        return Some(LocalIntent::Joke);
    }
    if is_greeting(&text) {
        return Some(LocalIntent::Greeting);
    }
    None
}

fn evaluate_math(text: &str) -> Option<(String, i64)> {
    let mut expression = strip_prefixes(
        text,
        &[
            "cuanto es ",
            "cuanto son ",
            "calcula ",
            "calculame ",
            "resultado de ",
        ],
    )
    .to_owned();
    for (from, to) in [
        (" multiplicado por ", " * "),
        (" dividido entre ", " / "),
        (" dividido por ", " / "),
        (" por ", " * "),
        (" entre ", " / "),
        (" mas ", " + "),
        (" menos ", " - "),
    ] {
        expression = expression.replace(from, to);
    }
    let expression = expression
        .chars()
        .filter(|character| {
            character.is_ascii_digit()
                || matches!(character, '+' | '-' | '*' | '/' | '(' | ')')
                || character.is_whitespace()
        })
        .collect::<String>();
    let expression = expression.split_whitespace().collect::<Vec<_>>().join(" ");
    if expression.is_empty()
        || !expression
            .chars()
            .any(|character| matches!(character, '+' | '-' | '*' | '/'))
    {
        return None;
    }
    let tokens: Vec<char> = expression.chars().filter(|c| !c.is_whitespace()).collect();
    let mut index = 0usize;
    let value = parse_sum(&tokens, &mut index)?;
    if index != tokens.len() {
        return None;
    }
    Some((expression, value))
}

fn parse_sum(tokens: &[char], index: &mut usize) -> Option<i64> {
    let mut value = parse_product(tokens, index)?;
    while *index < tokens.len() && matches!(tokens[*index], '+' | '-') {
        let operator = tokens[*index];
        *index += 1;
        let right = parse_product(tokens, index)?;
        value = if operator == '+' {
            value.checked_add(right)?
        } else {
            value.checked_sub(right)?
        };
    }
    Some(value)
}

fn parse_product(tokens: &[char], index: &mut usize) -> Option<i64> {
    let mut value = parse_factor(tokens, index)?;
    while *index < tokens.len() && matches!(tokens[*index], '*' | '/') {
        let operator = tokens[*index];
        *index += 1;
        let right = parse_factor(tokens, index)?;
        value = if operator == '*' {
            value.checked_mul(right)?
        } else {
            if right == 0 {
                return None;
            }
            value.checked_div(right)?
        };
    }
    Some(value)
}

fn parse_factor(tokens: &[char], index: &mut usize) -> Option<i64> {
    if *index >= tokens.len() {
        return None;
    }
    if tokens[*index] == '-' {
        *index += 1;
        return parse_factor(tokens, index).map(|value| -value);
    }
    if tokens[*index] == '(' {
        *index += 1;
        let value = parse_sum(tokens, index)?;
        if *index >= tokens.len() || tokens[*index] != ')' {
            return None;
        }
        *index += 1;
        return Some(value);
    }
    let start = *index;
    while *index < tokens.len() && tokens[*index].is_ascii_digit() {
        *index += 1;
    }
    if start == *index {
        return None;
    }
    tokens[start..*index]
        .iter()
        .collect::<String>()
        .parse::<i64>()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalization_removes_accents_and_punctuation() {
        assert_eq!(normalize("¿Qué hora es, Aura?"), "que hora es aura");
        assert_eq!(normalize("Mañana en el salón"), "manana en el salon");
    }

    #[test]
    fn basic_control_intents_are_recognized() {
        assert_eq!(parse_local_intent("Hola Aura"), Some(LocalIntent::Greeting));
        assert_eq!(parse_local_intent("¿Qué hora es?"), Some(LocalIntent::Time));
        assert_eq!(parse_local_intent("¡Qué horas!"), Some(LocalIntent::Time));
        assert_eq!(
            parse_local_intent("¿Qué horas son?"),
            Some(LocalIntent::Time)
        );
        assert_eq!(
            parse_local_intent("¿Qué día es hoy?"),
            Some(LocalIntent::Date)
        );
        assert_eq!(
            parse_local_intent("sube el volumen"),
            Some(LocalIntent::VolumeStep(10))
        );
        assert_eq!(
            parse_local_intent("pon el volumen al 35"),
            Some(LocalIntent::VolumeSet(35))
        );
        assert_eq!(parse_local_intent("silencia"), Some(LocalIntent::Mute));
        assert_eq!(
            parse_local_intent("pon música"),
            Some(LocalIntent::MusicPlay)
        );
        assert_eq!(
            parse_local_intent("pausa la música"),
            Some(LocalIntent::MusicPause)
        );
        assert_eq!(
            parse_local_intent("¿qué está sonando?"),
            Some(LocalIntent::MusicStatus)
        );
    }

    #[test]
    fn lights_and_scenes_are_recognized() {
        assert_eq!(
            parse_local_intent("enciende la luz del salón"),
            Some(LocalIntent::LightsOn(Some("salon".to_owned())))
        );
        assert_eq!(
            parse_local_intent("apaga las luces de la cocina"),
            Some(LocalIntent::LightsOff(Some("cocina".to_owned())))
        );
        assert_eq!(
            parse_local_intent("apaga las luces"),
            Some(LocalIntent::LightsOff(None))
        );
        assert_eq!(
            parse_local_intent("activa la escena película"),
            Some(LocalIntent::SceneActivate("pelicula".to_owned()))
        );
        assert_eq!(
            parse_local_intent("¿qué luces están encendidas?"),
            Some(LocalIntent::LightsStatus)
        );
    }

    #[test]
    fn math_is_evaluated_locally_with_precedence() {
        assert_eq!(
            parse_local_intent("¿cuánto es 2 más 2?"),
            Some(LocalIntent::Math {
                expression: "2 + 2".to_owned(),
                result: 4
            })
        );
        assert_eq!(
            parse_local_intent("calcula 2 + 3 * 4"),
            Some(LocalIntent::Math {
                expression: "2 + 3 * 4".to_owned(),
                result: 14
            })
        );
        assert_eq!(
            parse_local_intent("cuánto es 10 dividido entre 2"),
            Some(LocalIntent::Math {
                expression: "10 / 2".to_owned(),
                result: 5
            })
        );
        assert!(parse_local_intent("cuánto es 5 / 0").is_none());
    }

    #[test]
    fn meta_intents_and_fallback() {
        assert_eq!(
            parse_local_intent("¿quién eres?"),
            Some(LocalIntent::Identity)
        );
        assert_eq!(
            parse_local_intent("¿qué puedes hacer?"),
            Some(LocalIntent::Capabilities)
        );
        assert_eq!(parse_local_intent("gracias"), Some(LocalIntent::Thanks));
        assert_eq!(
            parse_local_intent("hasta luego"),
            Some(LocalIntent::Farewell)
        );
        assert_eq!(
            parse_local_intent("cuéntame un chiste"),
            Some(LocalIntent::Joke)
        );
        // Complex/general questions must fall through to the LLM.
        assert!(parse_local_intent("¿qué tiempo hará mañana en Lima?").is_none());
        assert!(parse_local_intent("explícame la teoría de la relatividad").is_none());
    }

    #[test]
    fn actions_require_the_right_tool() {
        assert_eq!(
            LocalIntent::VolumeSet(30).required_tool(),
            Some("music.set_volume")
        );
        assert_eq!(
            LocalIntent::LightsOn(None).required_tool(),
            Some("homeassistant.call_service")
        );
        assert!(LocalIntent::Time.required_tool().is_none());
        assert!(!LocalIntent::Time.is_action());
        assert!(LocalIntent::MusicPlay.is_action());
    }
}
