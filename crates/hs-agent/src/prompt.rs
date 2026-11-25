//! Profile-aware system prompt construction.

use hs_core::models::{Profile, Tone};

/// Build the system prompt for a chat turn with `profile`.
///
/// The prompt carries the profile name, role, language and tone so a single
/// configured provider can serve the whole family, plus the tool-use rules and
/// the safety instructions defined by the appliance.
#[must_use]
pub fn build_system_prompt(profile: &Profile) -> String {
    let language = match profile.language.as_str() {
        "es" => "Always answer in Spanish (es), using a natural household register.",
        _ => "Always answer in English (en).",
    };
    let tone = tone_instruction(profile.tone);

    format!(
        "You are the household assistant of HomeServer Assistant OS, a local-first home server appliance.\n\
         You are speaking with {name}, whose household role is {role}.\n\
         {language}\n\
         Conversational tone: {tone}.\n\
         \n\
         Tool use:\n\
         - Tools are provided as function calls, already filtered to the ones this profile is allowed to use.\n\
         - Every call is re-checked against the profile permissions before execution.\n\
         - Call a tool only when it is needed to answer or act. Never invent tool names, arguments or results.\n\
         - If a tool is denied or fails, explain the problem to the user instead of retrying the same call.\n\
         \n\
         Safety:\n\
         - Never reveal API keys, tokens, passwords or biometric data, and never ask the user for them.\n\
         - Do not perform destructive or irreversible actions without explicit confirmation from the user.\n\
         - Keep answers short enough for a wall kiosk; ask a short clarifying question when a request is ambiguous.",
        name = profile.name,
        role = profile.role.id(),
        language = language,
        tone = tone
    )
}

fn tone_instruction(tone: Tone) -> &'static str {
    match tone {
        Tone::Friendly => "warm, helpful and conversational",
        Tone::Calm => "calm, brief and neutral",
        Tone::Playful => "playful and encouraging, suitable for a child",
        Tone::Formal => "formal, precise and concise",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use hs_core::models::{Avatar, ProfileRole};
    use uuid::Uuid;

    fn profile(name: &str, language: &str, tone: Tone) -> Profile {
        Profile {
            id: Uuid::new_v4(),
            name: name.to_owned(),
            role: ProfileRole::Adult,
            avatar: Avatar::default(),
            language: language.to_owned(),
            tone,
            allowed_tools: vec!["music.play".to_owned()],
            face_enrolled: false,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn prompt_contains_profile_identity() {
        let prompt = build_system_prompt(&profile("Mamá", "es", Tone::Friendly));
        assert!(prompt.contains("Mamá"));
        assert!(prompt.contains("adult"));
        assert!(prompt.contains("Spanish"));
        assert!(prompt.contains("warm, helpful and conversational"));
        assert!(prompt.contains("permissions"));
        assert!(prompt.contains("Never reveal API keys"));
    }

    #[test]
    fn prompt_uses_english_for_english_profiles() {
        let prompt = build_system_prompt(&profile("Alex", "en", Tone::Formal));
        assert!(prompt.contains("English"));
        assert!(prompt.contains("formal, precise and concise"));
        assert!(!prompt.contains("Spanish"));
    }

    #[test]
    fn unknown_language_falls_back_to_english() {
        let prompt = build_system_prompt(&profile("Guest", "fr", Tone::Calm));
        assert!(prompt.contains("English"));
        assert!(prompt.contains("calm, brief and neutral"));
    }
}
