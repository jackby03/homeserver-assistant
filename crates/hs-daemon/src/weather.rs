//! Current weather lookup for the kiosk (Open-Meteo, no API key).
//!
//! The response is read-only and cached by the caller; the only network call is
//! made when a user-facing weather surface asks for it.

use std::time::Duration;

use serde::Serialize;

/// Current conditions for one city.
#[derive(Debug, Clone, Serialize)]
pub struct WeatherSnapshot {
    /// Resolved city name.
    pub city: String,
    /// Temperature in Celsius.
    pub temperature_c: f64,
    /// Apparent temperature in Celsius.
    pub apparent_c: f64,
    /// Relative humidity percentage.
    pub humidity: f64,
    /// Precipitation in millimetres.
    pub precipitation_mm: f64,
    /// WMO weather code.
    pub weather_code: i64,
    /// Short Spanish description.
    pub description: String,
}

impl WeatherSnapshot {
    /// One-line reply used by the assistant.
    #[must_use]
    pub fn sentence(&self) -> String {
        let mut reply = format!(
            "En {} hay {:.1} grados Celsius, con sensación de {:.1} grados y humedad del {:.0} por ciento. {}.",
            self.city, self.temperature_c, self.apparent_c, self.humidity, self.description
        );
        if self.precipitation_mm > 0.0 {
            reply.push_str(&format!(
                " Hay {:.1} milímetros de precipitación.",
                self.precipitation_mm
            ));
        }
        reply
    }
}

/// Fetch current weather for `city` (Lima when absent).
///
/// # Errors
/// Returns a user-facing Spanish message when the city is unknown or the
/// upstream service cannot be reached.
pub async fn current(city: Option<&str>) -> Result<WeatherSnapshot, String> {
    let requested = city
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("Lima");
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|_| "No pude preparar la consulta del clima.".to_owned())?;

    let (name, latitude, longitude) = if requested.eq_ignore_ascii_case("lima") {
        ("Lima".to_owned(), -12.04318_f64, -77.02824_f64)
    } else {
        geocode(&client, requested).await.ok_or_else(|| {
            format!("No encontré la ciudad «{requested}». Prueba con otro nombre.")
        })?
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
    let body = client
        .get("https://api.open-meteo.com/v1/forecast")
        .query(&params)
        .send()
        .await
        .map_err(|_| format!("No pude consultar el clima de {name}. Revisa la conexión."))?
        .json::<serde_json::Value>()
        .await
        .map_err(|_| format!("El servicio del clima devolvió datos inválidos para {name}."))?;
    let current = body
        .get("current")
        .ok_or_else(|| format!("No recibí datos actuales del clima para {name}."))?;
    let temperature = current
        .get("temperature_2m")
        .and_then(|value| value.as_f64())
        .ok_or_else(|| format!("No pude leer la temperatura actual de {name}."))?;
    Ok(WeatherSnapshot {
        city: name,
        temperature_c: temperature,
        apparent_c: current
            .get("apparent_temperature")
            .and_then(|value| value.as_f64())
            .unwrap_or(temperature),
        humidity: current
            .get("relative_humidity_2m")
            .and_then(|value| value.as_f64())
            .unwrap_or(0.0),
        precipitation_mm: current
            .get("precipitation")
            .and_then(|value| value.as_f64())
            .unwrap_or(0.0),
        weather_code: current
            .get("weather_code")
            .and_then(|value| value.as_i64())
            .unwrap_or(0),
        description: description(
            current
                .get("weather_code")
                .and_then(|value| value.as_i64())
                .unwrap_or(0),
        )
        .to_owned(),
    })
}

async fn geocode(client: &reqwest::Client, city: &str) -> Option<(String, f64, f64)> {
    let params = [
        ("name".to_owned(), city.to_owned()),
        ("count".to_owned(), "1".to_owned()),
        ("language".to_owned(), "es".to_owned()),
        ("format".to_owned(), "json".to_owned()),
    ];
    let body = client
        .get("https://geocoding-api.open-meteo.com/v1/search")
        .query(&params)
        .send()
        .await
        .ok()?
        .json::<serde_json::Value>()
        .await
        .ok()?;
    let result = body.get("results")?.as_array()?.first()?;
    Some((
        result.get("name")?.as_str()?.to_owned(),
        result.get("latitude")?.as_f64()?,
        result.get("longitude")?.as_f64()?,
    ))
}

fn description(code: i64) -> &'static str {
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

#[cfg(test)]
mod tests {
    use super::description;

    #[test]
    fn weather_descriptions_are_spanish() {
        assert_eq!(description(0), "El cielo está despejado");
        assert_eq!(description(61), "Hay lluvia");
        assert_eq!(description(95), "Hay tormenta");
    }
}
