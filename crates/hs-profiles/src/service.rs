//! Profile CRUD, permission management and biometric enrollment.
//!
//! [`ProfileService`] is the only supported entry point for reading and
//! mutating household profiles. It owns all validation rules so every caller
//! (HTTP handlers, onboarding wizard, kiosk) applies the same constraints.

use std::sync::Arc;

use chrono::Utc;
use hs_core::models::{Avatar, Profile, ProfileRole, Tone};
use hs_core::state::{Database, SecretStore};
use hs_core::validation;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::biometric;
use crate::defaults;
use crate::error::{ProfileError, ProfileResult};

/// Maximum profile display name length, in Unicode scalar values.
pub const MAX_PROFILE_NAME_CHARS: usize = 48;

/// Request used to create a household profile.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateProfileRequest {
    /// Display name, e.g. `Mamá`. Must be unique (case-insensitive) and
    /// between 1 and [`MAX_PROFILE_NAME_CHARS`] characters.
    pub name: String,
    /// Family role that drove the default tool set.
    pub role: ProfileRole,
    /// Avatar appearance.
    pub avatar: Avatar,
    /// Preferred language (`es` or `en`).
    pub language: String,
    /// Conversational tone applied to the agent prompt.
    pub tone: Tone,
    /// Tool names this profile may use. Empty means no tools at all.
    pub allowed_tools: Vec<String>,
}

/// Partial update for an existing household profile.
///
/// Only the fields that are `Some` are applied; omitted fields keep their
/// current value.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UpdateProfileRequest {
    /// New display name.
    pub name: Option<String>,
    /// New family role.
    pub role: Option<ProfileRole>,
    /// New avatar appearance.
    pub avatar: Option<Avatar>,
    /// New preferred language (`es` or `en`).
    pub language: Option<String>,
    /// New conversational tone.
    pub tone: Option<Tone>,
    /// Replacement tool allow-list. Empty means no tools.
    pub allowed_tools: Option<Vec<String>>,
}

/// Service managing household profiles and their biometric embeddings.
#[derive(Debug)]
pub struct ProfileService {
    db: Arc<Database>,
    secrets: Arc<SecretStore>,
}

impl ProfileService {
    /// Create a profile service backed by `db` and the encrypted `secrets` store.
    #[must_use]
    pub fn new(db: Arc<Database>, secrets: Arc<SecretStore>) -> Self {
        Self { db, secrets }
    }

    /// List every profile ordered by creation time (oldest first).
    ///
    /// # Errors
    /// Returns [`ProfileError::Storage`] when the database cannot be read.
    pub fn list(&self) -> ProfileResult<Vec<Profile>> {
        let mut profiles: Vec<Profile> = self.db.list_profiles()?;
        profiles.sort_by(|left, right| {
            left.created_at
                .cmp(&right.created_at)
                .then_with(|| left.name.cmp(&right.name))
        });
        Ok(profiles)
    }

    /// Load a single profile.
    ///
    /// # Errors
    /// Returns [`ProfileError::NotFound`] when the profile does not exist and
    /// [`ProfileError::Storage`] when the database cannot be read.
    pub fn get(&self, id: Uuid) -> ProfileResult<Profile> {
        self.db.profile(id)?.ok_or(ProfileError::NotFound)
    }

    /// Create a profile from a validated request.
    ///
    /// # Errors
    /// Returns [`ProfileError::Validation`] for invalid fields or duplicate
    /// names and [`ProfileError::Storage`] when persistence fails.
    pub fn create(&self, request: CreateProfileRequest) -> ProfileResult<Profile> {
        let name = normalize_name(&request.name)?;
        request.avatar.validate()?;
        validation::language(&request.language)?;
        let allowed_tools = validate_tools(request.allowed_tools)?;
        self.ensure_unique_name(&name, None)?;

        let now = Utc::now();
        let profile = Profile {
            id: Uuid::new_v4(),
            name,
            role: request.role,
            avatar: request.avatar,
            language: request.language,
            tone: request.tone,
            allowed_tools,
            face_enrolled: false,
            created_at: now,
            updated_at: now,
        };
        self.db.save_profile(&profile)?;
        tracing::info!(profile_id = %profile.id, role = profile.role.id(), "profile created");
        Ok(profile)
    }

    /// Apply a partial update to an existing profile.
    ///
    /// # Errors
    /// Returns [`ProfileError::NotFound`] when the profile does not exist,
    /// [`ProfileError::Validation`] for invalid fields or duplicate names and
    /// [`ProfileError::Storage`] when persistence fails.
    pub fn update(&self, id: Uuid, request: UpdateProfileRequest) -> ProfileResult<Profile> {
        let mut profile = self.get(id)?;

        if let Some(name) = request.name {
            let name = normalize_name(&name)?;
            self.ensure_unique_name(&name, Some(id))?;
            profile.name = name;
        }
        if let Some(role) = request.role {
            profile.role = role;
        }
        if let Some(avatar) = request.avatar {
            avatar.validate()?;
            profile.avatar = avatar;
        }
        if let Some(language) = request.language {
            validation::language(&language)?;
            profile.language = language;
        }
        if let Some(tone) = request.tone {
            profile.tone = tone;
        }
        if let Some(allowed_tools) = request.allowed_tools {
            profile.allowed_tools = validate_tools(allowed_tools)?;
        }

        profile.updated_at = Utc::now();
        self.db.save_profile(&profile)?;
        Ok(profile)
    }

    /// Delete a profile and its encrypted biometric embedding.
    ///
    /// Returns `false` when the profile did not exist.
    ///
    /// # Errors
    /// Returns [`ProfileError::Storage`] when the database delete fails.
    pub fn delete(&self, id: Uuid) -> ProfileResult<bool> {
        let existed = self.db.delete_profile(id)?;
        if existed {
            if let Err(error) = self.secrets.delete(&biometric::secret_name(id)) {
                tracing::warn!(profile_id = %id, error = %error, "failed to remove the biometric secret of a deleted profile");
            }
            tracing::info!(profile_id = %id, "profile deleted");
        }
        Ok(existed)
    }

    /// Replace the tool allow-list of a profile.
    ///
    /// # Errors
    /// Returns [`ProfileError::NotFound`] when the profile does not exist,
    /// [`ProfileError::Validation`] for invalid tool names and
    /// [`ProfileError::Storage`] when persistence fails.
    pub fn set_allowed_tools(
        &self,
        id: Uuid,
        allowed_tools: Vec<String>,
    ) -> ProfileResult<Profile> {
        let mut profile = self.get(id)?;
        profile.allowed_tools = validate_tools(allowed_tools)?;
        profile.updated_at = Utc::now();
        self.db.save_profile(&profile)?;
        Ok(profile)
    }

    /// Enroll (or replace) the local face embedding of a profile.
    ///
    /// The embedding is validated, then stored encrypted through the secret
    /// store under `biometric.profile.<profile-id>`. All enrolled profiles must
    /// share one embedding dimension so comparisons stay meaningful.
    ///
    /// # Errors
    /// Returns [`ProfileError::NotFound`] when the profile does not exist,
    /// [`ProfileError::Validation`] for an invalid embedding or a dimension
    /// clash with another enrolled profile, and [`ProfileError::Storage`] when
    /// persistence or encryption fails.
    pub fn enroll_face(&self, id: Uuid, embedding: Vec<f32>) -> ProfileResult<Profile> {
        biometric::validate_embedding(&embedding)?;
        let mut profile = self.get(id)?;

        for other in self.list()? {
            if other.id == id || !other.face_enrolled {
                continue;
            }
            let Some(payload) = self.secrets.get(&biometric::secret_name(other.id))? else {
                tracing::warn!(
                    profile_id = %other.id,
                    "profile is marked as face enrolled but has no stored embedding"
                );
                continue;
            };
            let stored = biometric::decode_embedding(&payload)?;
            if stored.len() != embedding.len() {
                return Err(ProfileError::Validation(format!(
                    "embedding has {} dimensions but profile `{}` uses {}; all enrolled profiles must share one embedding size",
                    embedding.len(),
                    other.name,
                    stored.len()
                )));
            }
        }

        let encoded = biometric::encode_embedding(&embedding)?;
        self.secrets.set(&biometric::secret_name(id), &encoded)?;
        profile.face_enrolled = true;
        profile.updated_at = Utc::now();
        self.db.save_profile(&profile)?;
        tracing::info!(profile_id = %id, "face embedding enrolled");
        Ok(profile)
    }

    /// Remove the enrolled face embedding of a profile.
    ///
    /// # Errors
    /// Returns [`ProfileError::NotFound`] when the profile does not exist and
    /// [`ProfileError::Storage`] when persistence fails.
    pub fn remove_face(&self, id: Uuid) -> ProfileResult<Profile> {
        let mut profile = self.get(id)?;
        self.secrets.delete(&biometric::secret_name(id))?;
        profile.face_enrolled = false;
        profile.updated_at = Utc::now();
        self.db.save_profile(&profile)?;
        tracing::info!(profile_id = %id, "face embedding removed");
        Ok(profile)
    }

    /// Find the best matching enrolled profile for `embedding`.
    ///
    /// The cosine similarity of every enrolled profile is computed locally and
    /// the highest one that reaches `threshold` is returned. `threshold` must be
    /// between `0.0` and `1.0`; a higher value makes recognition stricter.
    ///
    /// # Errors
    /// Returns [`ProfileError::Validation`] for an invalid threshold or query
    /// embedding and [`ProfileError::Storage`] when the vault cannot be read.
    pub fn match_face(&self, embedding: &[f32], threshold: f32) -> ProfileResult<Option<Profile>> {
        if !(0.0..=1.0).contains(&threshold) {
            return Err(ProfileError::Validation(
                "threshold must be between 0.0 and 1.0".to_owned(),
            ));
        }
        biometric::validate_embedding(embedding)?;

        let mut best: Option<(f32, Profile)> = None;
        for profile in self.list()? {
            if !profile.face_enrolled {
                continue;
            }
            let secret_name = biometric::secret_name(profile.id);
            let Some(payload) = self.secrets.get(&secret_name)? else {
                tracing::warn!(
                    profile_id = %profile.id,
                    "profile is marked as face enrolled but has no stored embedding"
                );
                continue;
            };
            let stored = match biometric::decode_embedding(&payload) {
                Ok(stored) => stored,
                Err(error) => {
                    tracing::warn!(
                        profile_id = %profile.id,
                        error = %error,
                        "skipping unreadable biometric embedding"
                    );
                    continue;
                }
            };
            if stored.len() != embedding.len() {
                tracing::warn!(
                    profile_id = %profile.id,
                    expected = stored.len(),
                    received = embedding.len(),
                    "skipping biometric embedding with an unexpected dimension"
                );
                continue;
            }
            let Some(similarity) = biometric::cosine_similarity(embedding, &stored) else {
                continue;
            };
            if similarity >= threshold
                && best
                    .as_ref()
                    .is_none_or(|(best_score, _)| similarity > *best_score)
            {
                best = Some((similarity, profile));
            }
        }
        Ok(best.map(|(_, profile)| profile))
    }

    /// Built-in profiles offered by the onboarding wizard.
    ///
    /// The returned requests are validated by [`ProfileService::create`]; they
    /// are not persisted until the wizard creates them.
    #[must_use]
    pub fn default_profiles() -> Vec<CreateProfileRequest> {
        defaults::default_profiles()
    }

    fn ensure_unique_name(&self, name: &str, exclude: Option<Uuid>) -> ProfileResult<()> {
        let normalized = name.to_lowercase();
        for profile in self.list()? {
            if Some(profile.id) == exclude {
                continue;
            }
            if profile.name.trim().to_lowercase() == normalized {
                return Err(ProfileError::Validation(format!(
                    "a profile named `{name}` already exists"
                )));
            }
        }
        Ok(())
    }
}

fn normalize_name(raw: &str) -> ProfileResult<String> {
    let name = raw.trim();
    let length = name.chars().count();
    if length == 0 || length > MAX_PROFILE_NAME_CHARS {
        return Err(ProfileError::Validation(format!(
            "name must be between 1 and {MAX_PROFILE_NAME_CHARS} characters"
        )));
    }
    if name.chars().any(char::is_control) {
        return Err(ProfileError::Validation(
            "name must not contain control characters".to_owned(),
        ));
    }
    Ok(name.to_owned())
}

fn validate_tools(tools: Vec<String>) -> ProfileResult<Vec<String>> {
    let mut validated: Vec<String> = Vec::with_capacity(tools.len());
    for tool in tools {
        validation::tool_name(&tool)?;
        if !validated.contains(&tool) {
            validated.push(tool);
        }
    }
    Ok(validated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hs_core::state::{SecretRecord, SecretVault, Table};

    fn fixture() -> (ProfileService, Arc<Database>, Arc<SecretStore>) {
        let db = Arc::new(Database::open_in_memory().expect("in-memory database"));
        let secrets = Arc::new(SecretStore::new(Arc::clone(&db), SecretVault::generate()));
        let service = ProfileService::new(Arc::clone(&db), Arc::clone(&secrets));
        (service, db, secrets)
    }

    fn request(name: &str, tools: &[&str]) -> CreateProfileRequest {
        CreateProfileRequest {
            name: name.to_owned(),
            role: ProfileRole::Adult,
            avatar: Avatar::default(),
            language: "es".to_owned(),
            tone: Tone::Friendly,
            allowed_tools: tools.iter().map(|tool| (*tool).to_owned()).collect(),
        }
    }

    fn create(service: &ProfileService, name: &str, tools: &[&str]) -> Profile {
        service
            .create(request(name, tools))
            .expect("profile creation should succeed")
    }

    #[test]
    fn crud_roundtrip() {
        let (service, _db, _secrets) = fixture();
        let profile = create(&service, "Mamá", &["music.play"]);
        assert_eq!(profile.name, "Mamá");
        assert_eq!(profile.role, ProfileRole::Adult);
        assert!(!profile.face_enrolled);

        assert_eq!(service.list().unwrap().len(), 1);
        assert_eq!(service.get(profile.id).unwrap(), profile);

        let updated = service
            .update(
                profile.id,
                UpdateProfileRequest {
                    name: Some("Mamá Gómez".to_owned()),
                    tone: Some(Tone::Calm),
                    language: Some("en".to_owned()),
                    ..UpdateProfileRequest::default()
                },
            )
            .unwrap();
        assert_eq!(updated.name, "Mamá Gómez");
        assert_eq!(updated.tone, Tone::Calm);
        assert_eq!(updated.language, "en");
        assert_eq!(updated.allowed_tools, vec!["music.play".to_owned()]);

        assert!(service.delete(profile.id).unwrap());
        assert!(service.get(profile.id).is_err());
        assert!(!service.delete(profile.id).unwrap());
        assert!(service.list().unwrap().is_empty());
    }

    #[test]
    fn duplicate_names_are_rejected() {
        let (service, _db, _secrets) = fixture();
        create(&service, "Mamá", &[]);
        let error = service
            .create(request("  mamá  ", &[]))
            .expect_err("duplicate name must fail");
        assert!(matches!(error, ProfileError::Validation(_)), "{error:?}");

        let other = create(&service, "Invitado", &[]);
        let error = service
            .update(
                other.id,
                UpdateProfileRequest {
                    name: Some("MAMÁ".to_owned()),
                    ..UpdateProfileRequest::default()
                },
            )
            .expect_err("duplicate name must fail on update");
        assert!(matches!(error, ProfileError::Validation(_)), "{error:?}");
    }

    #[test]
    fn invalid_fields_are_rejected() {
        let (service, _db, _secrets) = fixture();

        let mut bad_avatar = request("Ana", &[]);
        bad_avatar.avatar.color = "blue".to_owned();
        assert!(service.create(bad_avatar).is_err());

        let mut bad_language = request("Ana", &[]);
        bad_language.language = "fr".to_owned();
        assert!(service.create(bad_language).is_err());

        assert!(service.create(request("", &[])).is_err());
        assert!(service.create(request(&"a".repeat(49), &[])).is_err());
        assert!(service.create(request("Ana", &["Bad Tool"])).is_err());
        assert!(service
            .create(request("Ana", &["music.play", "music.play"]))
            .is_ok());
        assert!(service.create(request("Control\u{0007}", &[])).is_err());
    }

    #[test]
    fn update_validates_partial_fields() {
        let (service, _db, _secrets) = fixture();
        let profile = create(&service, "Ana", &[]);

        let error = service
            .update(
                profile.id,
                UpdateProfileRequest {
                    avatar: Some(Avatar {
                        color: "#xyzxyz".to_owned(),
                        ..Avatar::default()
                    }),
                    ..UpdateProfileRequest::default()
                },
            )
            .expect_err("invalid avatar must fail");
        assert!(matches!(error, ProfileError::Validation(_)), "{error:?}");

        let unchanged = service.get(profile.id).unwrap();
        assert_eq!(unchanged.avatar, Avatar::default());
        assert!(service.get(Uuid::new_v4()).is_err());
    }

    #[test]
    fn permissions_matrix_from_default_profiles() {
        let (service, _db, _secrets) = fixture();
        let defaults = ProfileService::default_profiles();
        let mut created = Vec::new();
        for request in defaults {
            created.push(service.create(request).unwrap());
        }
        assert_eq!(created.len(), 4);

        let owner = &created[0];
        let adult = &created[1];
        let child = &created[2];
        let guest = &created[3];

        assert_eq!(owner.role, ProfileRole::Owner);
        assert_eq!(adult.role, ProfileRole::Adult);
        assert_eq!(child.role, ProfileRole::Child);
        assert_eq!(guest.role, ProfileRole::Guest);
        assert_eq!(owner.allowed_tools.len(), 16);
        assert!(owner.may_use_tool("system.status"));
        assert!(owner.may_use_tool("runtime.stop_instance"));
        assert!(!adult.may_use_tool("runtime.stop_instance"));
        assert!(adult.may_use_tool("runtime.start_instance"));
        assert!(child.may_use_tool("music.play"));
        assert!(!child.may_use_tool("runtime.start_instance"));
        assert!(!child.may_use_tool("http.webhook"));
        assert_eq!(guest.allowed_tools.len(), 3);
        assert!(guest.may_use_tool("homeassistant.list_entities"));
        assert!(!guest.may_use_tool("music.set_volume"));

        let updated = service
            .set_allowed_tools(child.id, vec!["music.play".to_owned()])
            .unwrap();
        assert_eq!(updated.allowed_tools, vec!["music.play".to_owned()]);
        assert!(updated.may_use_tool("music.play"));
        assert!(!updated.may_use_tool("homeassistant.list_entities"));
        assert!(service
            .set_allowed_tools(child.id, vec!["NOT A TOOL".to_owned()])
            .is_err());
    }

    #[test]
    fn biometric_matching_respects_threshold() {
        let (service, _db, _secrets) = fixture();
        let first = create(&service, "Ana", &[]);
        let second = create(&service, "Luis", &[]);

        let embedding_first = vec![1.0_f32, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        let embedding_second = vec![0.0_f32, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        service
            .enroll_face(first.id, embedding_first.clone())
            .unwrap();
        service
            .enroll_face(second.id, embedding_second.clone())
            .unwrap();

        let matched = service
            .match_face(&embedding_first, 0.9)
            .unwrap()
            .expect("exact embedding should match");
        assert_eq!(matched.id, first.id);

        // 0.8 similarity is below the 0.9 threshold and above 0.7.
        let partial = [0.8_f32, 0.6, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        assert!(service.match_face(&partial, 0.9).unwrap().is_none());
        assert_eq!(
            service.match_face(&partial, 0.7).unwrap().map(|p| p.id),
            Some(first.id)
        );
        // An unrelated direction matches nobody.
        let unrelated = [0.0_f32, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        assert!(service.match_face(&unrelated, 0.5).unwrap().is_none());
        // Threshold validation.
        assert!(service.match_face(&embedding_first, 1.5).is_err());
        assert!(service.match_face(&embedding_first, f32::NAN).is_err());
    }

    #[test]
    fn face_embedding_is_encrypted_at_rest() {
        let (service, db, secrets) = fixture();
        let profile = create(&service, "Ana", &[]);
        let embedding = vec![0.25_f32, -0.5, 0.75, 0.125, 0.0, 1.0, -1.0, 0.5];
        service.enroll_face(profile.id, embedding.clone()).unwrap();

        let secret_name = biometric::secret_name(profile.id);
        assert!(secrets.contains(&secret_name).unwrap());

        let record: SecretRecord = db
            .get(Table::Secrets, &secret_name)
            .unwrap()
            .expect("secret record must exist");
        let plaintext = serde_json::to_string(&embedding).unwrap();
        let serialized = serde_json::to_string(&record).unwrap();
        assert!(!serialized.contains(&plaintext));
        assert!(!serialized.contains("0.25"));
        assert_ne!(record.encrypted.ciphertext, plaintext);

        let decrypted = secrets.get(&secret_name).unwrap().unwrap();
        assert_eq!(decrypted.as_str(), plaintext);
        assert!(service.get(profile.id).unwrap().face_enrolled);
    }

    #[test]
    fn deleting_a_profile_removes_the_embedding() {
        let (service, db, secrets) = fixture();
        let profile = create(&service, "Ana", &[]);
        service.enroll_face(profile.id, vec![0.5_f32; 8]).unwrap();
        let secret_name = biometric::secret_name(profile.id);
        assert!(secrets.contains(&secret_name).unwrap());

        assert!(service.delete(profile.id).unwrap());
        assert!(!secrets.contains(&secret_name).unwrap());
        assert!(db.profile(profile.id).unwrap().is_none());
        assert!(service.match_face(&[0.5_f32; 8], 0.0).unwrap().is_none());
    }

    #[test]
    fn removing_a_face_disables_matching() {
        let (service, _db, secrets) = fixture();
        let profile = create(&service, "Ana", &[]);
        let embedding = vec![1.0_f32, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        service.enroll_face(profile.id, embedding.clone()).unwrap();
        assert!(service.match_face(&embedding, 0.9).unwrap().is_some());

        let updated = service.remove_face(profile.id).unwrap();
        assert!(!updated.face_enrolled);
        assert!(!secrets
            .contains(&biometric::secret_name(profile.id))
            .unwrap());
        assert!(service.match_face(&embedding, 0.9).unwrap().is_none());
    }

    #[test]
    fn enroll_face_validates_embeddings() {
        let (service, _db, _secrets) = fixture();
        let first = create(&service, "Ana", &[]);
        let second = create(&service, "Luis", &[]);

        assert!(service.enroll_face(first.id, vec![0.1_f32; 7]).is_err());
        assert!(service.enroll_face(first.id, vec![f32::NAN; 8]).is_err());
        assert!(service
            .enroll_face(Uuid::new_v4(), vec![0.1_f32; 8])
            .is_err());

        service.enroll_face(first.id, vec![0.1_f32; 8]).unwrap();
        let error = service
            .enroll_face(second.id, vec![0.1_f32; 16])
            .expect_err("dimension mismatch must be rejected");
        assert!(matches!(error, ProfileError::Validation(_)), "{error:?}");
        assert!(!service.get(second.id).unwrap().face_enrolled);
    }

    #[test]
    fn default_profiles_are_creatable_and_unique() {
        let (service, _db, _secrets) = fixture();
        let mut roles = Vec::new();
        let mut names = Vec::new();
        for request in ProfileService::default_profiles() {
            let profile = service.create(request).unwrap();
            roles.push(profile.role);
            names.push(profile.name);
        }
        names.sort();
        names.dedup();
        assert_eq!(names.len(), 4);
        assert_eq!(
            roles,
            vec![
                ProfileRole::Owner,
                ProfileRole::Adult,
                ProfileRole::Child,
                ProfileRole::Guest
            ]
        );
        assert_eq!(service.list().unwrap().len(), 4);
    }

    #[test]
    fn default_profiles_is_a_deterministic_constructor() {
        // `default_profiles` must be callable without a service instance.
        let first = ProfileService::default_profiles();
        let second = ProfileService::default_profiles();
        assert_eq!(first, second);
        assert_eq!(first.len(), 4);
    }
}
