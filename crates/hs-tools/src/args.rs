//! Argument extraction helpers shared by the built-in tools.
//!
//! Every helper returns [`ToolError::InvalidArguments`] so the registry and the
//! workflow engine can distinguish bad model output from adapter failures.

use serde_json::{Map, Value};

use crate::error::{ToolError, ToolResult};

/// Coerce tool arguments into an object, accepting `null` as "no arguments".
pub(crate) fn object(arguments: Value) -> ToolResult<Map<String, Value>> {
    match arguments {
        Value::Null => Ok(Map::new()),
        Value::Object(map) => Ok(map),
        _ => Err(invalid("arguments must be a JSON object")),
    }
}

/// Read a required string argument.
pub(crate) fn required_string(map: &Map<String, Value>, key: &str) -> ToolResult<String> {
    let Some(value) = map.get(key) else {
        return Err(invalid(&format!("missing required argument `{key}`")));
    };
    value
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| invalid(&format!("argument `{key}` must be a string")))
}

/// Read an optional JSON value.
pub(crate) fn optional_value(map: &Map<String, Value>, key: &str) -> Option<Value> {
    map.get(key).cloned()
}

/// Read a required integer argument constrained to `min..=max`.
pub(crate) fn required_integer_in_range(
    map: &Map<String, Value>,
    key: &str,
    min: u8,
    max: u8,
) -> ToolResult<u8> {
    let Some(value) = map.get(key) else {
        return Err(invalid(&format!("missing required argument `{key}`")));
    };
    let message = || {
        invalid(&format!(
            "argument `{key}` must be an integer between {min} and {max}"
        ))
    };
    let number = value.as_u64().ok_or_else(message)?;
    if number < u64::from(min) || number > u64::from(max) {
        return Err(message());
    }
    u8::try_from(number).map_err(|_| message())
}

/// Validate a runtime instance name and return it.
pub(crate) fn instance_name(map: &Map<String, Value>, key: &str) -> ToolResult<String> {
    let name = required_string(map, key)?;
    hs_core::validation::instance_name(&name)
        .map_err(|error| ToolError::InvalidArguments(error.to_string()))?;
    Ok(name)
}

/// Validate a Home Assistant domain/service identifier.
pub(crate) fn ha_identifier(map: &Map<String, Value>, key: &str) -> ToolResult<String> {
    let value = required_string(map, key)?;
    let valid = !value.is_empty()
        && value.len() <= 64
        && value.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_'
        });
    if valid {
        Ok(value)
    } else {
        Err(invalid(&format!(
            "argument `{key}` must be a Home Assistant identifier (lowercase letters, digits or underscore)"
        )))
    }
}

/// Validate an absolute HTTP(S) URL and return it.
pub(crate) fn http_url(map: &Map<String, Value>, key: &str) -> ToolResult<String> {
    let url = required_string(map, key)?;
    hs_core::validation::http_url(&url)
        .map_err(|error| ToolError::InvalidArguments(error.to_string()))?;
    Ok(url)
}

fn invalid(message: &str) -> ToolError {
    ToolError::InvalidArguments(message.to_owned())
}
