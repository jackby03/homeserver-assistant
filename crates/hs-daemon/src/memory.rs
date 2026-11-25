//! File-backed assistant memory, keyed by profile id.
//!
//! The store is intentionally small and transparent: one JSON file per profile
//! under `data/memory/`, written atomically. It never contains secrets because
//! the model is instructed not to store them, and it is excluded from provider
//! prompts except through the explicit `memory.recall` tool.

use std::path::PathBuf;

use async_trait::async_trait;
use hs_tools::{MemoryEntry, MemoryStore};

/// Persistent memory directory for the household profiles.
#[derive(Debug, Clone)]
pub struct FileMemoryStore {
    directory: PathBuf,
}

impl FileMemoryStore {
    /// Create the store rooted at `directory`.
    #[must_use]
    pub fn new(directory: PathBuf) -> Self {
        Self { directory }
    }

    fn path_for(&self, profile_id: &str) -> PathBuf {
        let safe: String = profile_id
            .chars()
            .filter(|character| character.is_ascii_alphanumeric() || *character == '-')
            .collect();
        self.directory.join(format!("{safe}.json"))
    }

    async fn load(&self, profile_id: &str) -> Result<Vec<MemoryEntry>, String> {
        let path = self.path_for(profile_id);
        match tokio::fs::read_to_string(&path).await {
            Ok(contents) => serde_json::from_str(&contents)
                .map_err(|error| format!("cannot parse memory file: {error}")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(error) => Err(format!("cannot read memory file: {error}")),
        }
    }

    async fn save(&self, profile_id: &str, entries: &[MemoryEntry]) -> Result<(), String> {
        tokio::fs::create_dir_all(&self.directory)
            .await
            .map_err(|error| format!("cannot create memory directory: {error}"))?;
        let path = self.path_for(profile_id);
        let temporary = path.with_extension("json.tmp");
        let contents = serde_json::to_vec_pretty(entries)
            .map_err(|error| format!("cannot serialize memory: {error}"))?;
        tokio::fs::write(&temporary, contents)
            .await
            .map_err(|error| format!("cannot write memory: {error}"))?;
        tokio::fs::rename(&temporary, &path)
            .await
            .map_err(|error| format!("cannot replace memory file: {error}"))
    }
}

#[async_trait]
impl MemoryStore for FileMemoryStore {
    async fn remember(
        &self,
        profile_id: &str,
        key: &str,
        value: &str,
    ) -> Result<MemoryEntry, String> {
        let mut entries = self.load(profile_id).await?;
        let entry = MemoryEntry {
            key: key.trim().to_owned(),
            value: value.trim().to_owned(),
        };
        if let Some(existing) = entries.iter_mut().find(|item| item.key == entry.key) {
            *existing = entry.clone();
        } else {
            entries.push(entry.clone());
        }
        self.save(profile_id, &entries).await?;
        Ok(entry)
    }

    async fn recall(
        &self,
        profile_id: &str,
        query: Option<&str>,
    ) -> Result<Vec<MemoryEntry>, String> {
        let entries = self.load(profile_id).await?;
        let needle = query.map(normalize);
        Ok(entries
            .into_iter()
            .filter(|entry| {
                needle.as_ref().is_none_or(|needle| {
                    needle.is_empty()
                        || normalize(&entry.key).contains(needle)
                        || normalize(&entry.value).contains(needle)
                })
            })
            .collect())
    }

    async fn forget(&self, profile_id: &str, key: &str) -> Result<bool, String> {
        let mut entries = self.load(profile_id).await?;
        let before = entries.len();
        entries.retain(|entry| entry.key != key);
        if entries.len() == before {
            return Ok(false);
        }
        self.save(profile_id, &entries).await?;
        Ok(true)
    }
}

fn normalize(value: &str) -> String {
    value
        .trim()
        .to_lowercase()
        .chars()
        .map(|character| match character {
            'á' | 'à' | 'ä' | 'â' => 'a',
            'é' | 'è' | 'ë' | 'ê' => 'e',
            'í' | 'ì' | 'ï' | 'î' => 'i',
            'ó' | 'ò' | 'ö' | 'ô' => 'o',
            'ú' | 'ù' | 'ü' | 'û' => 'u',
            'ñ' => 'n',
            '_' | '-' => ' ',
            other => other,
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn remembers_recalls_and_forgets_per_profile() {
        let directory = std::env::temp_dir().join(format!("hs-memory-{}", uuid::Uuid::new_v4()));
        let store = FileMemoryStore::new(directory.clone());
        store
            .remember("alice", "cafe", "sin azúcar")
            .await
            .expect("remember");
        store
            .remember("bob", "cafe", "con leche")
            .await
            .expect("remember");
        let alice = store.recall("alice", Some("azucar")).await.expect("recall");
        assert_eq!(alice.len(), 1);
        assert_eq!(alice[0].value, "sin azúcar");
        assert!(store.forget("alice", "cafe").await.expect("forget"));
        assert!(store
            .recall("alice", None)
            .await
            .expect("recall")
            .is_empty());
        assert_eq!(store.recall("bob", None).await.expect("recall").len(), 1);
        let _ = tokio::fs::remove_dir_all(&directory).await;
    }
}
