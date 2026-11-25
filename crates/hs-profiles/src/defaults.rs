//! Built-in household profiles seeded by the onboarding wizard.
//!
//! The four profiles cover the common appliance setup: the owner, a second
//! adult, a child and a guest. Each one gets a distinct avatar and a tool
//! allow-list that follows the least-privilege rule.

use crate::service::CreateProfileRequest;
use hs_core::models::{Avatar, AvatarShape, ProfileRole, Tone};

/// Full tool set granted to the household owner.
const OWNER_TOOLS: &[&str] = &[
    "system.status",
    "runtime.list_instances",
    "runtime.start_instance",
    "runtime.stop_instance",
    "homeassistant.list_entities",
    "homeassistant.call_service",
    "music.play",
    "music.pause",
    "music.set_volume",
    "http.webhook",
    "memory.remember",
    "memory.recall",
    "memory.forget",
    "timer.create",
    "timer.list",
    "timer.cancel",
];

/// Adults may do everything except stop running workloads.
const ADULT_TOOLS: &[&str] = &[
    "system.status",
    "runtime.list_instances",
    "runtime.start_instance",
    "homeassistant.list_entities",
    "homeassistant.call_service",
    "music.play",
    "music.pause",
    "music.set_volume",
    "http.webhook",
    "memory.remember",
    "memory.recall",
    "memory.forget",
    "timer.create",
    "timer.list",
    "timer.cancel",
];

/// Children get read-only status plus media control.
const CHILD_TOOLS: &[&str] = &[
    "system.status",
    "homeassistant.list_entities",
    "music.play",
    "music.pause",
    "music.set_volume",
];

/// Guests can only ask about entities and control the music.
const GUEST_TOOLS: &[&str] = &["homeassistant.list_entities", "music.play", "music.pause"];

fn tools(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

fn avatar(shape: AvatarShape, color: &str, eye_tilt_deg: f32) -> Avatar {
    Avatar {
        shape,
        color: color.to_owned(),
        eye_color: "#0b1220".to_owned(),
        eye_tilt_deg,
    }
}

/// Return the built-in profile requests, in display order.
pub(crate) fn default_profiles() -> Vec<CreateProfileRequest> {
    vec![
        CreateProfileRequest {
            name: "Principal".to_owned(),
            role: ProfileRole::Owner,
            avatar: avatar(AvatarShape::Squircle, "#8b5cf6", 0.0),
            language: "es".to_owned(),
            tone: Tone::Friendly,
            allowed_tools: tools(OWNER_TOOLS),
        },
        CreateProfileRequest {
            name: "Mamá".to_owned(),
            role: ProfileRole::Adult,
            avatar: avatar(AvatarShape::Cone, "#f472b6", 3.0),
            language: "es".to_owned(),
            tone: Tone::Friendly,
            allowed_tools: tools(ADULT_TOOLS),
        },
        CreateProfileRequest {
            name: "Hermano pequeño".to_owned(),
            role: ProfileRole::Child,
            avatar: avatar(AvatarShape::Cloud, "#34d399", -4.0),
            language: "es".to_owned(),
            tone: Tone::Playful,
            allowed_tools: tools(CHILD_TOOLS),
        },
        CreateProfileRequest {
            name: "Invitado".to_owned(),
            role: ProfileRole::Guest,
            avatar: avatar(AvatarShape::Droplet, "#fbbf24", 0.0),
            language: "es".to_owned(),
            tone: Tone::Calm,
            allowed_tools: tools(GUEST_TOOLS),
        },
    ]
}
