//! Editable Bober identity loaded from `data/SOUL.md`.
//!
//! The checked-in [`DEFAULT_SOUL`] is seeded into the appliance data directory
//! on first start. Administrators can edit the copy without rebuilding, and a
//! daemon restart picks it up.

use std::path::Path;

/// Default personality shipped with the repository.
pub const DEFAULT_SOUL: &str = include_str!("../../../SOUL.md");

/// Load `data/SOUL.md`, seeding it from [`DEFAULT_SOUL`] when missing or empty.
///
/// # Errors
/// Returns the filesystem error when the data directory is not writable.
pub fn load_or_seed(data_dir: &Path) -> std::io::Result<String> {
    let path = data_dir.join("SOUL.md");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    match std::fs::read_to_string(&path) {
        Ok(contents) if !contents.trim().is_empty() => Ok(contents),
        Ok(_) => {
            std::fs::write(&path, DEFAULT_SOUL)?;
            Ok(DEFAULT_SOUL.to_owned())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            std::fs::write(&path, DEFAULT_SOUL)?;
            Ok(DEFAULT_SOUL.to_owned())
        }
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_soul_has_the_identity_and_voice_rules() {
        assert!(DEFAULT_SOUL.contains("Bober"));
        assert!(DEFAULT_SOUL.contains("Nada de emojis"));
        assert!(DEFAULT_SOUL.contains("timer.create"));
    }

    #[test]
    fn seeding_and_loading_round_trips() {
        let directory = std::env::temp_dir().join(format!("hs-soul-{}", uuid::Uuid::new_v4()));
        let soul = load_or_seed(&directory).expect("seed");
        assert!(soul.contains("Bober"));
        assert!(directory.join("SOUL.md").is_file());
        std::fs::write(directory.join("SOUL.md"), "Custom soul").expect("write");
        assert_eq!(load_or_seed(&directory).expect("load"), "Custom soul");
        let _ = std::fs::remove_dir_all(&directory);
    }
}
