//! Embedded database (redb) and typed persistence helpers.
//!
//! The appliance stores a small amount of data: administrators, sessions,
//! profiles, deployed apps, LLM providers, workflows, audit entries and
//! encrypted secrets. redb is a pure-Rust embedded key/value store with
//! transactional writes, which keeps the ISO free of native database
//! dependencies and makes tests fast and hermetic.
//!
//! All values are JSON encoded behind the typed helpers; keys are strings
//! (UUIDs, usernames, secret names). The concrete encoding is an implementation
//! detail that can be migrated through the `meta` table.

use std::path::Path;

use chrono::Utc;
use redb::ReadableTable;
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::error::{CoreError, CoreResult};
use crate::models::{
    AppRecord, AuditEntry, Profile, ProviderRecord, SessionRecord, User, Workflow,
};

type TableDef = redb::TableDefinition<'static, &'static str, &'static [u8]>;

const USERS: TableDef = redb::TableDefinition::new("users");
const SESSIONS: TableDef = redb::TableDefinition::new("sessions");
const SECRETS: TableDef = redb::TableDefinition::new("secrets");
const PROFILES: TableDef = redb::TableDefinition::new("profiles");
const PROVIDERS: TableDef = redb::TableDefinition::new("providers");
const WORKFLOWS: TableDef = redb::TableDefinition::new("workflows");
const APPS: TableDef = redb::TableDefinition::new("apps");
const AUDIT: TableDef = redb::TableDefinition::new("audit");
const META: TableDef = redb::TableDefinition::new("meta");

const ALL_TABLES: [TableDef; 9] = [
    USERS, SESSIONS, SECRETS, PROFILES, PROVIDERS, WORKFLOWS, APPS, AUDIT, META,
];

const AUDIT_SEQUENCE_KEY: &str = "audit_sequence";

/// Logical tables exposed by [`Database`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Table {
    /// Administrator accounts.
    Users,
    /// Active and revoked sessions.
    Sessions,
    /// Encrypted secrets (never plaintext).
    Secrets,
    /// Household profiles.
    Profiles,
    /// LLM provider configuration.
    Providers,
    /// Node-based workflows.
    Workflows,
    /// Deployed applications.
    Apps,
    /// Audit trail.
    Audit,
    /// Internal metadata.
    Meta,
}

impl Table {
    fn definition(self) -> TableDef {
        match self {
            Self::Users => USERS,
            Self::Sessions => SESSIONS,
            Self::Secrets => SECRETS,
            Self::Profiles => PROFILES,
            Self::Providers => PROVIDERS,
            Self::Workflows => WORKFLOWS,
            Self::Apps => APPS,
            Self::Audit => AUDIT,
            Self::Meta => META,
        }
    }
}

/// Embedded, transactional key/value database.
pub struct Database {
    inner: redb::Database,
}

impl std::fmt::Debug for Database {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Database").finish_non_exhaustive()
    }
}

impl Database {
    /// Open (or create) the database at `path`.
    ///
    /// # Errors
    /// Returns [`CoreError::Storage`] when the file cannot be opened.
    pub fn open(path: &Path) -> CoreResult<Self> {
        if let Some(parent) = path.parent() {
            crate::state::fsutil::ensure_dir(parent)?;
        }
        let inner = redb::Database::create(path)?;
        let db = Self { inner };
        db.ensure_tables()?;
        Ok(db)
    }

    /// Open a transient in-memory database. Used by tests and by
    /// `hs-daemon --dev` when no data directory is available.
    ///
    /// # Errors
    /// Returns [`CoreError::Storage`] when the backend cannot be created.
    pub fn open_in_memory() -> CoreResult<Self> {
        let backend = redb::backends::InMemoryBackend::new();
        let inner = redb::Database::builder().create_with_backend(backend)?;
        let db = Self { inner };
        db.ensure_tables()?;
        Ok(db)
    }

    fn ensure_tables(&self) -> CoreResult<()> {
        let write = self.inner.begin_write()?;
        {
            for table in ALL_TABLES {
                // Opening the table in a write transaction creates it.
                let _ = write.open_table(table)?;
            }
        }
        write.commit()?;
        Ok(())
    }

    // -- raw helpers --------------------------------------------------------

    fn put_raw(&self, table: Table, key: &str, bytes: &[u8]) -> CoreResult<()> {
        let write = self.inner.begin_write()?;
        {
            let mut handle = write.open_table(table.definition())?;
            handle.insert(key, bytes)?;
        }
        write.commit()?;
        Ok(())
    }

    fn get_raw(&self, table: Table, key: &str) -> CoreResult<Option<Vec<u8>>> {
        let read = self.inner.begin_read()?;
        let handle = read.open_table(table.definition())?;
        Ok(handle.get(key)?.map(|guard| guard.value().to_vec()))
    }

    fn delete_raw(&self, table: Table, key: &str) -> CoreResult<bool> {
        let write = self.inner.begin_write()?;
        let removed = {
            let mut handle = write.open_table(table.definition())?;
            let existed = handle.remove(key)?.is_some();
            existed
        };
        write.commit()?;
        Ok(removed)
    }

    fn list_raw(&self, table: Table) -> CoreResult<Vec<(String, Vec<u8>)>> {
        let read = self.inner.begin_read()?;
        let handle = read.open_table(table.definition())?;
        let mut entries = Vec::new();
        for item in handle.iter()? {
            let (key, value) = item?;
            entries.push((key.value().to_owned(), value.value().to_vec()));
        }
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(entries)
    }

    /// Store a serializable value under `key`.
    ///
    /// # Errors
    /// Returns [`CoreError::Storage`] on database or serialization failure.
    pub fn put<T: Serialize>(&self, table: Table, key: &str, value: &T) -> CoreResult<()> {
        let bytes = serde_json::to_vec(value)?;
        self.put_raw(table, key, &bytes)
    }

    /// Load a value by key.
    ///
    /// # Errors
    /// Returns [`CoreError::Storage`] on database or deserialization failure.
    pub fn get<T: DeserializeOwned>(&self, table: Table, key: &str) -> CoreResult<Option<T>> {
        match self.get_raw(table, key)? {
            Some(bytes) => {
                let value = serde_json::from_slice(&bytes).map_err(|err| {
                    CoreError::Storage(format!(
                        "corrupted record `{key}` in table {:?}: {err}",
                        table
                    ))
                })?;
                Ok(Some(value))
            }
            None => Ok(None),
        }
    }

    /// List every value in a table.
    ///
    /// # Errors
    /// Returns [`CoreError::Storage`] on database or deserialization failure.
    pub fn list<T: DeserializeOwned>(&self, table: Table) -> CoreResult<Vec<T>> {
        let mut values = Vec::new();
        for (_, bytes) in self.list_raw(table)? {
            let value = serde_json::from_slice(&bytes).map_err(|err| {
                CoreError::Storage(format!("corrupted record in table {table:?}: {err}"))
            })?;
            values.push(value);
        }
        Ok(values)
    }

    /// Delete a key, returning whether it existed.
    ///
    /// # Errors
    /// Returns [`CoreError::Storage`] on database failure.
    pub fn delete(&self, table: Table, key: &str) -> CoreResult<bool> {
        self.delete_raw(table, key)
    }

    /// Number of records in a table.
    ///
    /// # Errors
    /// Returns [`CoreError::Storage`] on database failure.
    pub fn count(&self, table: Table) -> CoreResult<usize> {
        Ok(self.list_raw(table)?.len())
    }

    // -- typed convenience helpers -----------------------------------------

    /// Insert or replace a user.
    ///
    /// # Errors
    /// Returns [`CoreError`] on storage failure.
    pub fn save_user(&self, user: &User) -> CoreResult<()> {
        self.put(Table::Users, &user.id.to_string(), user)
    }

    /// Find a user by id.
    ///
    /// # Errors
    /// Returns [`CoreError`] on storage failure.
    pub fn user_by_id(&self, id: uuid::Uuid) -> CoreResult<Option<User>> {
        self.get(Table::Users, &id.to_string())
    }

    /// Find a user by login name (case-insensitive).
    ///
    /// # Errors
    /// Returns [`CoreError`] on storage failure.
    pub fn user_by_username(&self, username: &str) -> CoreResult<Option<User>> {
        let normalized = username.trim().to_lowercase();
        Ok(self
            .list::<User>(Table::Users)?
            .into_iter()
            .find(|user| user.username == normalized))
    }

    /// List all users.
    ///
    /// # Errors
    /// Returns [`CoreError`] on storage failure.
    pub fn list_users(&self) -> CoreResult<Vec<User>> {
        self.list(Table::Users)
    }

    /// Delete a user by id.
    ///
    /// # Errors
    /// Returns [`CoreError`] on storage failure.
    pub fn delete_user(&self, id: uuid::Uuid) -> CoreResult<bool> {
        self.delete(Table::Users, &id.to_string())
    }

    /// Save a session.
    ///
    /// # Errors
    /// Returns [`CoreError`] on storage failure.
    pub fn save_session(&self, session: &SessionRecord) -> CoreResult<()> {
        self.put(Table::Sessions, &session.id.to_string(), session)
    }

    /// Load a session.
    ///
    /// # Errors
    /// Returns [`CoreError`] on storage failure.
    pub fn session(&self, id: uuid::Uuid) -> CoreResult<Option<SessionRecord>> {
        self.get(Table::Sessions, &id.to_string())
    }

    /// Delete a session.
    ///
    /// # Errors
    /// Returns [`CoreError`] on storage failure.
    pub fn delete_session(&self, id: uuid::Uuid) -> CoreResult<bool> {
        self.delete(Table::Sessions, &id.to_string())
    }

    /// Remove expired or revoked sessions.
    ///
    /// # Errors
    /// Returns [`CoreError`] on storage failure.
    pub fn purge_sessions(&self, now: chrono::DateTime<chrono::Utc>) -> CoreResult<usize> {
        let mut removed = 0;
        for session in self.list::<SessionRecord>(Table::Sessions)? {
            if !session.is_active(now) && self.delete_session(session.id)? {
                removed += 1;
            }
        }
        Ok(removed)
    }

    /// Insert or replace a profile.
    ///
    /// # Errors
    /// Returns [`CoreError`] on storage failure.
    pub fn save_profile(&self, profile: &Profile) -> CoreResult<()> {
        self.put(Table::Profiles, &profile.id.to_string(), profile)
    }

    /// Load a profile.
    ///
    /// # Errors
    /// Returns [`CoreError`] on storage failure.
    pub fn profile(&self, id: uuid::Uuid) -> CoreResult<Option<Profile>> {
        self.get(Table::Profiles, &id.to_string())
    }

    /// List all profiles.
    ///
    /// # Errors
    /// Returns [`CoreError`] on storage failure.
    pub fn list_profiles(&self) -> CoreResult<Vec<Profile>> {
        self.list(Table::Profiles)
    }

    /// Delete a profile.
    ///
    /// # Errors
    /// Returns [`CoreError`] on storage failure.
    pub fn delete_profile(&self, id: uuid::Uuid) -> CoreResult<bool> {
        self.delete(Table::Profiles, &id.to_string())
    }

    /// Insert or replace a provider record.
    ///
    /// # Errors
    /// Returns [`CoreError`] on storage failure.
    pub fn save_provider(&self, provider: &ProviderRecord) -> CoreResult<()> {
        self.put(Table::Providers, &provider.id.to_string(), provider)
    }

    /// Load a provider record.
    ///
    /// # Errors
    /// Returns [`CoreError`] on storage failure.
    pub fn provider(&self, id: uuid::Uuid) -> CoreResult<Option<ProviderRecord>> {
        self.get(Table::Providers, &id.to_string())
    }

    /// List provider records.
    ///
    /// # Errors
    /// Returns [`CoreError`] on storage failure.
    pub fn list_providers(&self) -> CoreResult<Vec<ProviderRecord>> {
        self.list(Table::Providers)
    }

    /// Delete a provider record.
    ///
    /// # Errors
    /// Returns [`CoreError`] on storage failure.
    pub fn delete_provider(&self, id: uuid::Uuid) -> CoreResult<bool> {
        self.delete(Table::Providers, &id.to_string())
    }

    /// Insert or replace a workflow.
    ///
    /// # Errors
    /// Returns [`CoreError`] on storage failure.
    pub fn save_workflow(&self, workflow: &Workflow) -> CoreResult<()> {
        self.put(Table::Workflows, &workflow.id.to_string(), workflow)
    }

    /// Load a workflow.
    ///
    /// # Errors
    /// Returns [`CoreError`] on storage failure.
    pub fn workflow(&self, id: uuid::Uuid) -> CoreResult<Option<Workflow>> {
        self.get(Table::Workflows, &id.to_string())
    }

    /// List workflows.
    ///
    /// # Errors
    /// Returns [`CoreError`] on storage failure.
    pub fn list_workflows(&self) -> CoreResult<Vec<Workflow>> {
        self.list(Table::Workflows)
    }

    /// Delete a workflow.
    ///
    /// # Errors
    /// Returns [`CoreError`] on storage failure.
    pub fn delete_workflow(&self, id: uuid::Uuid) -> CoreResult<bool> {
        self.delete(Table::Workflows, &id.to_string())
    }

    /// Insert or replace an app record.
    ///
    /// # Errors
    /// Returns [`CoreError`] on storage failure.
    pub fn save_app(&self, app: &AppRecord) -> CoreResult<()> {
        self.put(Table::Apps, &app.id.to_string(), app)
    }

    /// Load an app record.
    ///
    /// # Errors
    /// Returns [`CoreError`] on storage failure.
    pub fn app(&self, id: uuid::Uuid) -> CoreResult<Option<AppRecord>> {
        self.get(Table::Apps, &id.to_string())
    }

    /// List app records.
    ///
    /// # Errors
    /// Returns [`CoreError`] on storage failure.
    pub fn list_apps(&self) -> CoreResult<Vec<AppRecord>> {
        self.list(Table::Apps)
    }

    /// Delete an app record.
    ///
    /// # Errors
    /// Returns [`CoreError`] on storage failure.
    pub fn delete_app(&self, id: uuid::Uuid) -> CoreResult<bool> {
        self.delete(Table::Apps, &id.to_string())
    }

    /// Append an audit entry, assigning the next sequence number.
    ///
    /// # Errors
    /// Returns [`CoreError`] on storage failure.
    pub fn append_audit(&self, mut entry: AuditEntry) -> CoreResult<AuditEntry> {
        let write = self.inner.begin_write()?;
        let sequence = {
            let meta = write.open_table(META)?;
            let next = match meta.get(AUDIT_SEQUENCE_KEY)? {
                Some(guard) => {
                    let bytes = guard.value();
                    serde_json::from_slice::<u64>(bytes).unwrap_or(0) + 1
                }
                None => 1,
            };
            next
        };
        entry.seq = sequence;
        if entry.timestamp == chrono::DateTime::<Utc>::MIN_UTC {
            entry.timestamp = Utc::now();
        }
        {
            let mut meta = write.open_table(META)?;
            let encoded = serde_json::to_vec(&sequence)?;
            meta.insert(AUDIT_SEQUENCE_KEY, encoded.as_slice())?;

            let mut audit = write.open_table(AUDIT)?;
            let key = format!("{sequence:020}");
            let encoded = serde_json::to_vec(&entry)?;
            audit.insert(key.as_str(), encoded.as_slice())?;
        }
        write.commit()?;
        Ok(entry)
    }

    /// List the most recent audit entries (newest first).
    ///
    /// # Errors
    /// Returns [`CoreError`] on storage failure.
    pub fn list_audit(&self, limit: usize) -> CoreResult<Vec<AuditEntry>> {
        let entries = self.list_raw(Table::Audit)?;
        let mut result = Vec::with_capacity(limit.min(entries.len()));
        for (_, bytes) in entries.into_iter().rev().take(limit) {
            let entry: AuditEntry = serde_json::from_slice(&bytes)
                .map_err(|err| CoreError::Storage(format!("corrupted audit entry: {err}")))?;
            result.push(entry);
        }
        Ok(result)
    }

    /// Store a free-form metadata value.
    ///
    /// # Errors
    /// Returns [`CoreError`] on storage failure.
    pub fn set_meta<T: Serialize>(&self, key: &str, value: &T) -> CoreResult<()> {
        self.put(Table::Meta, key, value)
    }

    /// Read a free-form metadata value.
    ///
    /// # Errors
    /// Returns [`CoreError`] on storage failure.
    pub fn meta<T: DeserializeOwned>(&self, key: &str) -> CoreResult<Option<T>> {
        self.get(Table::Meta, key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{AuditOutcome, Avatar, ProfileRole, Role, Tone};
    use chrono::Utc;
    use uuid::Uuid;

    fn user(username: &str) -> User {
        User {
            id: Uuid::new_v4(),
            username: username.to_owned(),
            display_name: username.to_owned(),
            password_hash: "$argon2id$fake".to_owned(),
            role: Role::Admin,
            disabled: false,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_login_at: None,
        }
    }

    #[test]
    fn users_roundtrip_and_lookup() {
        let db = Database::open_in_memory().unwrap();
        let admin = user("admin");
        db.save_user(&admin).unwrap();
        assert_eq!(db.count(Table::Users).unwrap(), 1);
        assert_eq!(db.user_by_id(admin.id).unwrap().unwrap(), admin);
        assert_eq!(db.user_by_username("ADMIN").unwrap().unwrap().id, admin.id);
        assert!(db.delete_user(admin.id).unwrap());
        assert!(db.user_by_id(admin.id).unwrap().is_none());
    }

    #[test]
    fn profiles_roundtrip() {
        let db = Database::open_in_memory().unwrap();
        let profile = Profile {
            id: Uuid::new_v4(),
            name: "Mamá".to_owned(),
            role: ProfileRole::Adult,
            avatar: Avatar::default(),
            language: "es".to_owned(),
            tone: Tone::Friendly,
            allowed_tools: vec!["music.play".to_owned()],
            face_enrolled: false,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        db.save_profile(&profile).unwrap();
        assert_eq!(db.profile(profile.id).unwrap().unwrap(), profile);
        assert_eq!(db.list_profiles().unwrap().len(), 1);
        assert!(db.delete_profile(profile.id).unwrap());
    }

    #[test]
    fn audit_sequence_is_monotonic_and_newest_first() {
        let db = Database::open_in_memory().unwrap();
        for index in 0..3 {
            let entry = AuditEntry {
                seq: 0,
                timestamp: Utc::now(),
                actor: "system".to_owned(),
                action: format!("test.action.{index}"),
                target: None,
                outcome: AuditOutcome::Success,
                ip: None,
                details: None,
            };
            let stored = db.append_audit(entry).unwrap();
            assert_eq!(stored.seq, index + 1);
        }
        let entries = db.list_audit(2).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].action, "test.action.2");
        assert_eq!(entries[1].action, "test.action.1");
    }

    #[test]
    fn persistence_survives_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.redb");
        let user_id;
        {
            let db = Database::open(&path).unwrap();
            let admin = user("admin");
            user_id = admin.id;
            db.save_user(&admin).unwrap();
        }
        let db = Database::open(&path).unwrap();
        assert_eq!(db.user_by_id(user_id).unwrap().unwrap().username, "admin");
    }

    #[test]
    fn generic_tables_support_any_serializable_type() {
        let db = Database::open_in_memory().unwrap();
        db.set_meta("boot_count", &7u64).unwrap();
        assert_eq!(db.meta::<u64>("boot_count").unwrap(), Some(7));
    }
}
