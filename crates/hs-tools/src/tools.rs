//! The built-in tool set exposed to the assistant and the workflow engine.
//!
//! Each tool is a thin, stateless adapter around one [`crate::ports`] trait
//! method. Argument validation happens here, before any adapter call, so an
//! invalid model request can never reach the host.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{Duration, Local, NaiveTime, TimeZone};
use hs_core::models::Profile;
use serde_json::{json, Value};

use crate::args;
use crate::error::{ToolError, ToolResult};
use crate::ports::ToolPorts;
use crate::registry::Tool;

const NO_ARGUMENTS_SCHEMA: &str = "object";

fn no_arguments() -> Value {
    json!({
        "type": NO_ARGUMENTS_SCHEMA,
        "properties": {},
        "required": [],
        "additionalProperties": false
    })
}

fn instance_argument_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "name": {
                "type": "string",
                "description": "Runtime instance name (lowercase letters, digits and hyphens)."
            }
        },
        "required": ["name"],
        "additionalProperties": false
    })
}

struct SystemStatus;

#[async_trait]
impl Tool for SystemStatus {
    fn name(&self) -> &'static str {
        "system.status"
    }

    fn description(&self) -> &'static str {
        "Return an overview of the appliance: CPU, memory, disks, temperature, battery, network and services."
    }

    fn parameters(&self) -> Value {
        no_arguments()
    }

    async fn execute(&self, ports: &ToolPorts, arguments: Value) -> ToolResult<Value> {
        let _ = args::object(arguments)?;
        ports.system.overview().await.map_err(ToolError::Upstream)
    }
}

struct RuntimeListInstances;

#[async_trait]
impl Tool for RuntimeListInstances {
    fn name(&self) -> &'static str {
        "runtime.list_instances"
    }

    fn description(&self) -> &'static str {
        "List the containers and virtual machines managed by the runtime, including their state and addresses."
    }

    fn parameters(&self) -> Value {
        no_arguments()
    }

    async fn execute(&self, ports: &ToolPorts, arguments: Value) -> ToolResult<Value> {
        let _ = args::object(arguments)?;
        let instances = ports
            .runtime
            .list_instances()
            .await
            .map_err(ToolError::Upstream)?;
        let count = instances.len();
        Ok(json!({ "instances": instances, "count": count }))
    }
}

struct RuntimeStartInstance;

#[async_trait]
impl Tool for RuntimeStartInstance {
    fn name(&self) -> &'static str {
        "runtime.start_instance"
    }

    fn description(&self) -> &'static str {
        "Start a stopped container or virtual machine."
    }

    fn parameters(&self) -> Value {
        instance_argument_schema()
    }

    async fn execute(&self, ports: &ToolPorts, arguments: Value) -> ToolResult<Value> {
        let map = args::object(arguments)?;
        let name = args::instance_name(&map, "name")?;
        ports
            .runtime
            .start_instance(&name)
            .await
            .map_err(ToolError::Upstream)?;
        Ok(json!({ "status": "started", "instance": name }))
    }
}

struct RuntimeStopInstance;

#[async_trait]
impl Tool for RuntimeStopInstance {
    fn name(&self) -> &'static str {
        "runtime.stop_instance"
    }

    fn risk(&self) -> crate::registry::ToolRisk {
        crate::registry::ToolRisk::Destructive
    }

    fn description(&self) -> &'static str {
        "Stop a running container or virtual machine."
    }

    fn parameters(&self) -> Value {
        instance_argument_schema()
    }

    async fn execute(&self, ports: &ToolPorts, arguments: Value) -> ToolResult<Value> {
        let map = args::object(arguments)?;
        let name = args::instance_name(&map, "name")?;
        ports
            .runtime
            .stop_instance(&name)
            .await
            .map_err(ToolError::Upstream)?;
        Ok(json!({ "status": "stopped", "instance": name }))
    }
}

struct HomeAssistantListEntities;

#[async_trait]
impl Tool for HomeAssistantListEntities {
    fn name(&self) -> &'static str {
        "homeassistant.list_entities"
    }

    fn description(&self) -> &'static str {
        "List the Home Assistant entities with their current state and friendly name."
    }

    fn parameters(&self) -> Value {
        no_arguments()
    }

    async fn execute(&self, ports: &ToolPorts, arguments: Value) -> ToolResult<Value> {
        let _ = args::object(arguments)?;
        let entities = ports
            .domotics
            .list_entities()
            .await
            .map_err(ToolError::Upstream)?;
        let count = entities.len();
        Ok(json!({ "entities": entities, "count": count }))
    }
}

struct HomeAssistantCallService;

#[async_trait]
impl Tool for HomeAssistantCallService {
    fn name(&self) -> &'static str {
        "homeassistant.call_service"
    }

    fn description(&self) -> &'static str {
        "Call a Home Assistant service, for example domain `light` with service `turn_on`."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "domain": {
                    "type": "string",
                    "description": "Service domain, e.g. `light`."
                },
                "service": {
                    "type": "string",
                    "description": "Service name, e.g. `turn_on`."
                },
                "data": {
                    "type": "object",
                    "description": "Service payload (entity ids, attributes).",
                    "additionalProperties": true
                }
            },
            "required": ["domain", "service"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, ports: &ToolPorts, arguments: Value) -> ToolResult<Value> {
        let map = args::object(arguments)?;
        let domain = args::ha_identifier(&map, "domain")?;
        let service = args::ha_identifier(&map, "service")?;
        let data = match map.get("data") {
            None | Some(Value::Null) => Value::Object(serde_json::Map::new()),
            Some(value @ Value::Object(_)) => value.clone(),
            Some(_) => {
                return Err(ToolError::InvalidArguments(
                    "argument `data` must be a JSON object".to_owned(),
                ));
            }
        };
        ports
            .domotics
            .call_service(&domain, &service, data)
            .await
            .map_err(ToolError::Upstream)?;
        Ok(json!({ "status": "called", "domain": domain, "service": service }))
    }
}

struct MusicPlay;

#[async_trait]
impl Tool for MusicPlay {
    fn name(&self) -> &'static str {
        "music.play"
    }

    fn description(&self) -> &'static str {
        "Start or resume music playback on the household audio sink."
    }

    fn parameters(&self) -> Value {
        no_arguments()
    }

    async fn execute(&self, ports: &ToolPorts, arguments: Value) -> ToolResult<Value> {
        let _ = args::object(arguments)?;
        ports.audio.play().await.map_err(ToolError::Upstream)?;
        Ok(json!({ "status": "playing" }))
    }
}

struct MusicPause;

#[async_trait]
impl Tool for MusicPause {
    fn name(&self) -> &'static str {
        "music.pause"
    }

    fn description(&self) -> &'static str {
        "Pause music playback on the household audio sink."
    }

    fn parameters(&self) -> Value {
        no_arguments()
    }

    async fn execute(&self, ports: &ToolPorts, arguments: Value) -> ToolResult<Value> {
        let _ = args::object(arguments)?;
        ports.audio.pause().await.map_err(ToolError::Upstream)?;
        Ok(json!({ "status": "paused" }))
    }
}

struct MusicSetVolume;

#[async_trait]
impl Tool for MusicSetVolume {
    fn name(&self) -> &'static str {
        "music.set_volume"
    }

    fn description(&self) -> &'static str {
        "Set the playback volume as a percentage between 0 and 100."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "percent": {
                    "type": "integer",
                    "description": "Target volume percentage (0-100).",
                    "minimum": 0,
                    "maximum": 100
                }
            },
            "required": ["percent"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, ports: &ToolPorts, arguments: Value) -> ToolResult<Value> {
        let map = args::object(arguments)?;
        let percent = args::required_integer_in_range(&map, "percent", 0, 100)?;
        ports
            .audio
            .set_volume(percent)
            .await
            .map_err(ToolError::Upstream)?;
        Ok(json!({ "status": "volume_set", "volume": percent }))
    }
}

struct HttpWebhook;

#[async_trait]
impl Tool for HttpWebhook {
    fn name(&self) -> &'static str {
        "http.webhook"
    }

    fn description(&self) -> &'static str {
        "Send a JSON payload to an outbound HTTP(S) webhook."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "Absolute http(s) webhook URL."
                },
                "payload": {
                    "description": "JSON payload delivered to the webhook.",
                    "type": ["object", "array", "string", "number", "boolean", "null"]
                }
            },
            "required": ["url"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, ports: &ToolPorts, arguments: Value) -> ToolResult<Value> {
        let map = args::object(arguments)?;
        let url = args::http_url(&map, "url")?;
        let payload = args::optional_value(&map, "payload").unwrap_or_else(|| json!({}));
        let status_code = ports
            .webhooks
            .post(&url, payload)
            .await
            .map_err(ToolError::Upstream)?;
        Ok(json!({ "status": "delivered", "status_code": status_code }))
    }
}

fn memory_key(map: &serde_json::Map<String, Value>) -> ToolResult<String> {
    let key = args::required_string(map, "key")?;
    let key = key.trim();
    if key.is_empty() || key.len() > 64 {
        return Err(ToolError::InvalidArguments(
            "key must be between 1 and 64 characters".to_owned(),
        ));
    }
    Ok(key.to_owned())
}

struct MemoryRemember;

#[async_trait]
impl Tool for MemoryRemember {
    fn name(&self) -> &'static str {
        "memory.remember"
    }

    fn description(&self) -> &'static str {
        "Remember a durable fact or preference for this household profile (e.g. a birthday, a favourite drink, an access preference)."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "key": {
                    "type": "string",
                    "description": "Short stable key, e.g. cumpleanos_mama."
                },
                "value": {
                    "type": "string",
                    "description": "The fact to remember."
                }
            },
            "required": ["key", "value"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _ports: &ToolPorts, _arguments: Value) -> ToolResult<Value> {
        Err(ToolError::InvalidArguments(
            "memory tools require the active profile".to_owned(),
        ))
    }

    async fn execute_for_profile(
        &self,
        ports: &ToolPorts,
        arguments: Value,
        profile: &Profile,
    ) -> ToolResult<Value> {
        let map = args::object(arguments)?;
        let key = memory_key(&map)?;
        let value = args::required_string(&map, "value")?;
        let value = value.trim();
        if value.is_empty() || value.chars().count() > 2_000 {
            return Err(ToolError::InvalidArguments(
                "value must be between 1 and 2000 characters".to_owned(),
            ));
        }
        let entry = ports
            .memory
            .remember(&profile.id.to_string(), &key, value)
            .await
            .map_err(ToolError::Upstream)?;
        Ok(json!({ "status": "remembered", "key": entry.key, "value": entry.value }))
    }
}

struct MemoryRecall;

#[async_trait]
impl Tool for MemoryRecall {
    fn name(&self) -> &'static str {
        "memory.recall"
    }

    fn description(&self) -> &'static str {
        "Recall durable facts and preferences stored for this household profile, optionally filtered by a search term."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Optional case-insensitive search term."
                }
            },
            "required": [],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _ports: &ToolPorts, _arguments: Value) -> ToolResult<Value> {
        Err(ToolError::InvalidArguments(
            "memory tools require the active profile".to_owned(),
        ))
    }

    async fn execute_for_profile(
        &self,
        ports: &ToolPorts,
        arguments: Value,
        profile: &Profile,
    ) -> ToolResult<Value> {
        let map = args::object(arguments)?;
        let query =
            args::optional_value(&map, "query").and_then(|value| value.as_str().map(str::to_owned));
        let entries = ports
            .memory
            .recall(&profile.id.to_string(), query.as_deref())
            .await
            .map_err(ToolError::Upstream)?;
        let count = entries.len();
        Ok(json!({ "memories": entries, "count": count }))
    }
}

struct MemoryForget;

#[async_trait]
impl Tool for MemoryForget {
    fn name(&self) -> &'static str {
        "memory.forget"
    }

    fn description(&self) -> &'static str {
        "Delete a stored memory by its key."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "key": { "type": "string", "description": "Memory key to delete." }
            },
            "required": ["key"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _ports: &ToolPorts, _arguments: Value) -> ToolResult<Value> {
        Err(ToolError::InvalidArguments(
            "memory tools require the active profile".to_owned(),
        ))
    }

    async fn execute_for_profile(
        &self,
        ports: &ToolPorts,
        arguments: Value,
        profile: &Profile,
    ) -> ToolResult<Value> {
        let map = args::object(arguments)?;
        let key = memory_key(&map)?;
        let forgotten = ports
            .memory
            .forget(&profile.id.to_string(), &key)
            .await
            .map_err(ToolError::Upstream)?;
        Ok(json!({ "forgotten": forgotten, "key": key }))
    }
}

fn timer_label(map: &serde_json::Map<String, Value>) -> ToolResult<String> {
    let label = args::required_string(map, "label")?;
    let label = label.trim();
    if label.is_empty() || label.chars().count() > 200 {
        return Err(ToolError::InvalidArguments(
            "label must be between 1 and 200 characters".to_owned(),
        ));
    }
    Ok(label.to_owned())
}

fn timer_seconds(map: &serde_json::Map<String, Value>) -> ToolResult<u64> {
    if let Some(seconds) = map.get("seconds").and_then(Value::as_u64) {
        if !(10..=86_400).contains(&seconds) {
            return Err(ToolError::InvalidArguments(
                "seconds must be between 10 and 86400".to_owned(),
            ));
        }
        return Ok(seconds);
    }
    let at = map
        .get("at")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ToolError::InvalidArguments("missing `seconds` or `at`".to_owned()))?;
    let now = Local::now();
    if let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(at) {
        let millis = parsed
            .with_timezone(&Local)
            .signed_duration_since(now)
            .num_milliseconds();
        if millis <= 0 {
            return Err(ToolError::InvalidArguments(
                "`at` must be in the future".to_owned(),
            ));
        }
        return Ok(u64::try_from((millis + 999) / 1_000)
            .unwrap_or(86_400)
            .clamp(10, 86_400));
    }
    let time = NaiveTime::parse_from_str(at, "%H:%M")
        .or_else(|_| NaiveTime::parse_from_str(at, "%H:%M:%S"))
        .map_err(|_| {
            ToolError::InvalidArguments(
                "`at` must be HH:MM (24-hour) or an RFC3339 timestamp".to_owned(),
            )
        })?;
    let mut due = now.date_naive().and_time(time);
    if due <= now.naive_local() {
        due += Duration::days(1);
    }
    let due = Local
        .from_local_datetime(&due)
        .single()
        .ok_or_else(|| ToolError::InvalidArguments("invalid local time".to_owned()))?;
    let millis = due.signed_duration_since(now).num_milliseconds().max(0);
    let seconds = u64::try_from((millis + 999) / 1_000).unwrap_or(86_400);
    Ok(seconds.clamp(10, 86_400))
}

struct TimerCreate;

#[async_trait]
impl Tool for TimerCreate {
    fn name(&self) -> &'static str {
        "timer.create"
    }

    fn description(&self) -> &'static str {
        "Create a reminder. Pass seconds from now, or at='HH:MM' in the appliance local time shown in the prompt (for example 'recuérdame sacar la basura a las 19:00')."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "label": {
                    "type": "string",
                    "description": "What to remind the user about."
                },
                "seconds": {
                    "type": "integer",
                    "description": "Relative delay in seconds from now (10 to 86400).",
                    "minimum": 10,
                    "maximum": 86400
                },
                "at": {
                    "type": "string",
                    "description": "Absolute local time as HH:MM (24-hour) or an RFC3339 timestamp. Use the current time from the system prompt."
                }
            },
            "required": ["label"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _ports: &ToolPorts, _arguments: Value) -> ToolResult<Value> {
        Err(ToolError::InvalidArguments(
            "timer tools require the active profile".to_owned(),
        ))
    }

    async fn execute_for_profile(
        &self,
        ports: &ToolPorts,
        arguments: Value,
        profile: &Profile,
    ) -> ToolResult<Value> {
        let map = args::object(arguments)?;
        let label = timer_label(&map)?;
        let seconds = timer_seconds(&map)?;
        let entry = ports
            .timers
            .create(&profile.id.to_string(), &label, seconds)
            .await
            .map_err(ToolError::Upstream)?;
        Ok(json!({
            "status": "scheduled",
            "id": entry.id,
            "label": entry.label,
            "due_at": entry.due_at,
        }))
    }
}

struct TimerList;

#[async_trait]
impl Tool for TimerList {
    fn name(&self) -> &'static str {
        "timer.list"
    }

    fn description(&self) -> &'static str {
        "List the reminders that are still pending for this profile."
    }

    fn parameters(&self) -> Value {
        no_arguments()
    }

    async fn execute(&self, _ports: &ToolPorts, _arguments: Value) -> ToolResult<Value> {
        Err(ToolError::InvalidArguments(
            "timer tools require the active profile".to_owned(),
        ))
    }

    async fn execute_for_profile(
        &self,
        ports: &ToolPorts,
        arguments: Value,
        profile: &Profile,
    ) -> ToolResult<Value> {
        let _ = args::object(arguments)?;
        let timers = ports
            .timers
            .list(&profile.id.to_string())
            .await
            .map_err(ToolError::Upstream)?;
        let count = timers.len();
        Ok(json!({ "timers": timers, "count": count }))
    }
}

struct TimerCancel;

#[async_trait]
impl Tool for TimerCancel {
    fn name(&self) -> &'static str {
        "timer.cancel"
    }

    fn description(&self) -> &'static str {
        "Cancel a pending reminder by its id."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "Timer id returned by timer.create." }
            },
            "required": ["id"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _ports: &ToolPorts, _arguments: Value) -> ToolResult<Value> {
        Err(ToolError::InvalidArguments(
            "timer tools require the active profile".to_owned(),
        ))
    }

    async fn execute_for_profile(
        &self,
        ports: &ToolPorts,
        arguments: Value,
        profile: &Profile,
    ) -> ToolResult<Value> {
        let map = args::object(arguments)?;
        let id = args::required_string(&map, "id")?;
        let cancelled = ports
            .timers
            .cancel(&profile.id.to_string(), &id)
            .await
            .map_err(ToolError::Upstream)?;
        Ok(json!({ "cancelled": cancelled, "id": id }))
    }
}

struct NewsLatest;

#[async_trait]
impl Tool for NewsLatest {
    fn name(&self) -> &'static str {
        "news.latest"
    }

    fn description(&self) -> &'static str {
        "Read current headlines from a Spanish-language news feed, optionally filtered by topic. Always cite the source and date; never invent news."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Optional topic, person, place or keyword."
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of headlines (1-8).",
                    "minimum": 1,
                    "maximum": 8
                }
            },
            "required": [],
            "additionalProperties": false
        })
    }

    async fn execute(&self, ports: &ToolPorts, arguments: Value) -> ToolResult<Value> {
        let map = args::object(arguments)?;
        let query = args::optional_value(&map, "query").and_then(|value| match value {
            Value::String(value) => {
                let trimmed = value.trim();
                (!trimmed.is_empty()).then(|| trimmed.to_owned())
            }
            _ => None,
        });
        let limit = map
            .get("limit")
            .and_then(Value::as_u64)
            .map(|value| usize::try_from(value).unwrap_or(5))
            .unwrap_or(5)
            .clamp(1, 8);
        let items = ports
            .news
            .latest(query.as_deref(), limit)
            .await
            .map_err(ToolError::Upstream)?;
        let count = items.len();
        Ok(json!({ "items": items, "count": count }))
    }
}

fn external_date(map: &serde_json::Map<String, Value>, key: &str) -> ToolResult<String> {
    let value = args::required_string(map, key)?;
    chrono::DateTime::parse_from_rfc3339(&value).map_err(|_| {
        ToolError::InvalidArguments(format!("`{key}` must be an RFC3339 timestamp"))
    })?;
    Ok(value)
}

struct CalendarListEvents;

#[async_trait]
impl Tool for CalendarListEvents {
    fn name(&self) -> &'static str {
        "calendar.list_events"
    }

    fn description(&self) -> &'static str {
        "List upcoming events from the configured calendar. Requires the admin to enable the Composio integration."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "time_min": { "type": "string", "description": "Optional RFC3339 lower bound." },
                "time_max": { "type": "string", "description": "Optional RFC3339 upper bound." },
                "max_results": { "type": "integer", "minimum": 1, "maximum": 50 }
            },
            "required": [],
            "additionalProperties": false
        })
    }

    async fn execute(&self, ports: &ToolPorts, arguments: Value) -> ToolResult<Value> {
        let map = args::object(arguments)?;
        let mut payload = serde_json::Map::new();
        if let Some(value) = map.get("time_min") {
            payload.insert("time_min".to_owned(), value.clone());
        }
        if let Some(value) = map.get("time_max") {
            payload.insert("time_max".to_owned(), value.clone());
        }
        if let Some(value) = map.get("max_results") {
            payload.insert("max_results".to_owned(), value.clone());
        }
        ports
            .external
            .execute("calendar.list", Value::Object(payload))
            .await
            .map_err(ToolError::Upstream)
    }
}

struct CalendarCreateEvent;

#[async_trait]
impl Tool for CalendarCreateEvent {
    fn name(&self) -> &'static str {
        "calendar.create_event"
    }

    fn description(&self) -> &'static str {
        "Create a calendar event. The agent must ask for confirmation first."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "summary": { "type": "string", "description": "Event title." },
                "start": { "type": "string", "description": "Start time as RFC3339." },
                "end": { "type": "string", "description": "End time as RFC3339." },
                "description": { "type": "string", "description": "Optional details." }
            },
            "required": ["summary", "start", "end"],
            "additionalProperties": false
        })
    }

    fn risk(&self) -> crate::registry::ToolRisk {
        crate::registry::ToolRisk::Write
    }

    async fn execute(&self, ports: &ToolPorts, arguments: Value) -> ToolResult<Value> {
        let map = args::object(arguments)?;
        let summary = args::required_string(&map, "summary")?;
        let start = external_date(&map, "start")?;
        let end = external_date(&map, "end")?;
        let description = args::optional_value(&map, "description")
            .and_then(|value| value.as_str().map(str::to_owned));
        ports
            .external
            .execute(
                "calendar.create",
                json!({
                    "summary": summary,
                    "start": start,
                    "end": end,
                    "description": description,
                }),
            )
            .await
            .map_err(ToolError::Upstream)
    }
}

struct EmailSend;

#[async_trait]
impl Tool for EmailSend {
    fn name(&self) -> &'static str {
        "email.send"
    }

    fn description(&self) -> &'static str {
        "Send an email through the configured mailbox. The agent must ask for explicit confirmation first."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "to": { "type": "string", "description": "Recipient email address." },
                "subject": { "type": "string", "description": "Email subject." },
                "body": { "type": "string", "description": "Plain-text email body." }
            },
            "required": ["to", "subject", "body"],
            "additionalProperties": false
        })
    }

    fn risk(&self) -> crate::registry::ToolRisk {
        crate::registry::ToolRisk::Destructive
    }

    async fn execute(&self, ports: &ToolPorts, arguments: Value) -> ToolResult<Value> {
        let map = args::object(arguments)?;
        let to = args::required_string(&map, "to")?;
        if !to.contains('@') || to.len() > 320 {
            return Err(ToolError::InvalidArguments(
                "`to` must be a valid email address".to_owned(),
            ));
        }
        let subject = args::required_string(&map, "subject")?;
        let body = args::required_string(&map, "body")?;
        ports
            .external
            .execute(
                "email.send",
                json!({ "to": to, "subject": subject, "body": body }),
            )
            .await
            .map_err(ToolError::Upstream)
    }
}

/// Instantiate every built-in tool.
pub(crate) fn builtin_tools() -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(SystemStatus),
        Arc::new(RuntimeListInstances),
        Arc::new(RuntimeStartInstance),
        Arc::new(RuntimeStopInstance),
        Arc::new(HomeAssistantListEntities),
        Arc::new(HomeAssistantCallService),
        Arc::new(MusicPlay),
        Arc::new(MusicPause),
        Arc::new(MusicSetVolume),
        Arc::new(HttpWebhook),
        Arc::new(MemoryRemember),
        Arc::new(MemoryRecall),
        Arc::new(MemoryForget),
        Arc::new(TimerCreate),
        Arc::new(TimerList),
        Arc::new(TimerCancel),
        Arc::new(NewsLatest),
        Arc::new(CalendarListEvents),
        Arc::new(CalendarCreateEvent),
        Arc::new(EmailSend),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{fake_ports, profile_with_tools, CallCounts};
    use crate::{ToolRegistry, ToolRisk};

    fn registry() -> (Arc<ToolRegistry>, Arc<CallCounts>) {
        let (ports, counts, _webhook_payload) = fake_ports();
        (ToolRegistry::with_ports(ports), counts)
    }

    fn all_tools() -> Vec<String> {
        vec![
            "system.status".to_owned(),
            "runtime.list_instances".to_owned(),
            "runtime.start_instance".to_owned(),
            "runtime.stop_instance".to_owned(),
            "homeassistant.list_entities".to_owned(),
            "homeassistant.call_service".to_owned(),
            "music.play".to_owned(),
            "music.pause".to_owned(),
            "music.set_volume".to_owned(),
            "http.webhook".to_owned(),
            "memory.remember".to_owned(),
            "memory.recall".to_owned(),
            "memory.forget".to_owned(),
            "timer.create".to_owned(),
            "timer.list".to_owned(),
            "timer.cancel".to_owned(),
            "news.latest".to_owned(),
            "calendar.list_events".to_owned(),
            "calendar.create_event".to_owned(),
            "email.send".to_owned(),
        ]
    }

    #[tokio::test]
    async fn every_builtin_tool_has_a_schema() {
        let (registry, _counts) = registry();
        let descriptors = registry.list();
        assert_eq!(descriptors.len(), 20);
        let names: Vec<String> = descriptors.iter().map(|tool| tool.name.clone()).collect();
        let mut expected = all_tools();
        expected.sort();
        assert_eq!(names, expected);
        for descriptor in descriptors {
            assert!(!descriptor.description.is_empty());
            assert_eq!(descriptor.parameters["type"], "object");
        }
    }

    #[tokio::test]
    async fn memory_tools_are_profile_scoped() {
        let (registry, _counts) = registry();
        let profile = profile_with_tools(&["memory.remember", "memory.recall", "memory.forget"]);
        registry
            .invoke(
                &profile,
                "memory.remember",
                json!({"key": "cafe", "value": "sin azúcar"}),
            )
            .await
            .expect("remember");
        let recalled = registry
            .invoke(&profile, "memory.recall", json!({"query": "cafe"}))
            .await
            .expect("recall");
        assert_eq!(recalled.output["count"], 1);
        assert_eq!(recalled.output["memories"][0]["value"], "sin azúcar");
        let forgotten = registry
            .invoke(&profile, "memory.forget", json!({"key": "cafe"}))
            .await
            .expect("forget");
        assert_eq!(forgotten.output["forgotten"], true);
        let empty = registry
            .invoke(&profile, "memory.recall", json!({}))
            .await
            .expect("recall");
        assert_eq!(empty.output["count"], 0);
    }

    #[tokio::test]
    async fn external_tools_are_allowlisted_and_risk_classified() {
        let (registry, _counts) = registry();
        assert_eq!(registry.risk("calendar.list_events"), Some(ToolRisk::Read));
        assert_eq!(
            registry.risk("calendar.create_event"),
            Some(ToolRisk::Write)
        );
        assert_eq!(registry.risk("email.send"), Some(ToolRisk::Destructive));
        assert_eq!(registry.risk("news.latest"), Some(ToolRisk::Read));
        assert_eq!(registry.risk("does.not.exist"), None);
    }

    #[tokio::test]
    async fn external_tools_only_reach_the_named_capabilities() {
        let (registry, _counts) = registry();
        let profile = profile_with_tools(&[
            "calendar.list_events",
            "calendar.create_event",
            "email.send",
        ]);
        let calendar = registry
            .invoke(&profile, "calendar.list_events", json!({"max_results": 5}))
            .await
            .expect("calendar list");
        assert_eq!(calendar.output["capability"], "calendar.list");
        assert_eq!(calendar.output["arguments"]["max_results"], 5);

        let email = registry
            .invoke(
                &profile,
                "email.send",
                json!({"to": "ana@example.com", "subject": "Hola", "body": "Texto"}),
            )
            .await
            .expect("email");
        assert_eq!(email.output["capability"], "email.send");
        assert_eq!(email.output["arguments"]["to"], "ana@example.com");
    }

    #[tokio::test]
    async fn news_tool_returns_feed_items() {
        let (registry, _counts) = registry();
        let profile = profile_with_tools(&["news.latest"]);
        let result = registry
            .invoke(
                &profile,
                "news.latest",
                json!({"query": "Perú", "limit": 2}),
            )
            .await
            .expect("news");
        assert_eq!(result.output["count"], 2);
        assert!(result.output["items"][0]["link"]
            .as_str()
            .is_some_and(|link| link.starts_with("https://")));
    }

    #[tokio::test]
    async fn timer_tools_create_list_and_cancel() {
        let (registry, _counts) = registry();
        let profile = profile_with_tools(&["timer.create", "timer.list", "timer.cancel"]);
        let created = registry
            .invoke(
                &profile,
                "timer.create",
                json!({"label": "sacar la basura", "seconds": 60}),
            )
            .await
            .expect("create");
        assert_eq!(created.output["status"], "scheduled");
        let id = created.output["id"].as_str().expect("id").to_owned();
        let listed = registry
            .invoke(&profile, "timer.list", json!({}))
            .await
            .expect("list");
        assert_eq!(listed.output["count"], 1);
        let cancelled = registry
            .invoke(&profile, "timer.cancel", json!({"id": id}))
            .await
            .expect("cancel");
        assert_eq!(cancelled.output["cancelled"], true);
        let empty = registry
            .invoke(&profile, "timer.list", json!({}))
            .await
            .expect("list");
        assert_eq!(empty.output["count"], 0);
    }

    #[tokio::test]
    async fn happy_path_calls_reach_the_adapters() {
        let (registry, counts) = registry();
        let profile = profile_with_tools(&all_tools());
        let calls: Vec<(&str, Value)> = vec![
            ("system.status", json!({})),
            ("runtime.list_instances", json!({})),
            ("runtime.start_instance", json!({"name": "home-assistant"})),
            ("runtime.stop_instance", json!({"name": "home-assistant"})),
            ("homeassistant.list_entities", json!({})),
            (
                "homeassistant.call_service",
                json!({"domain": "light", "service": "turn_on", "data": {"entity_id": "light.kitchen"}}),
            ),
            ("music.play", json!({})),
            ("music.pause", json!({})),
            ("music.set_volume", json!({"percent": 42})),
            (
                "http.webhook",
                json!({"url": "https://hooks.example.test/notify", "payload": {"text": "hola"}}),
            ),
        ];
        for (tool, arguments) in calls {
            let outcome = registry
                .invoke(&profile, tool, arguments)
                .await
                .unwrap_or_else(|error| panic!("{tool} failed: {error}"));
            assert_eq!(outcome.tool, tool);
            assert!(outcome.output.is_object() || outcome.output.is_null());
        }
        let [runtime, domotics, audio, system, webhooks] = counts.snapshot();
        assert_eq!(runtime, 3);
        assert_eq!(domotics, 2);
        assert_eq!(audio, 3);
        assert_eq!(system, 1);
        assert_eq!(webhooks, 1);
    }

    #[tokio::test]
    async fn argument_validation_happens_before_any_port_call() {
        let (registry, counts) = registry();
        let profile = profile_with_tools(&all_tools());
        let invalid_calls: Vec<(&str, Value)> = vec![
            ("system.status", json!("not-an-object")),
            ("runtime.start_instance", json!({})),
            ("runtime.start_instance", json!({"name": "Bad Name"})),
            ("runtime.stop_instance", json!({"name": 7})),
            ("homeassistant.call_service", json!({"service": "turn_on"})),
            (
                "homeassistant.call_service",
                json!({"domain": "Light", "service": "turn_on"}),
            ),
            (
                "homeassistant.call_service",
                json!({"domain": "light", "service": "turn_on", "data": "nope"}),
            ),
            ("music.set_volume", json!({"percent": 101})),
            ("music.set_volume", json!({"percent": -1})),
            ("music.set_volume", json!({"percent": "loud"})),
            ("http.webhook", json!({"url": "not-a-url"})),
            ("http.webhook", json!({})),
        ];
        for (tool, arguments) in invalid_calls {
            let error = registry
                .invoke(&profile, tool, arguments)
                .await
                .expect_err("invalid arguments must be rejected");
            assert!(
                matches!(error, ToolError::InvalidArguments(_)),
                "{tool}: expected InvalidArguments, got {error:?}"
            );
        }
        let [runtime, domotics, audio, _system, webhooks] = counts.snapshot();
        assert_eq!(runtime, 0);
        assert_eq!(domotics, 0);
        assert_eq!(audio, 0);
        assert_eq!(webhooks, 0);
    }
}
