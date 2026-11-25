//! Adapters that connect the agentic tool ports to concrete infrastructure.
//!
//! `hs-tools` deliberately knows nothing about Incus, Home Assistant or
//! PipeWire. The daemon implements the port traits here and injects them into
//! the tool registry. Every permission check happens inside the registry before
//! these adapters are called.

use std::sync::Arc;

use async_trait::async_trait;
use hs_homeassistant::HaService;
use hs_runtime::RuntimeProvider;
use hs_system::SystemService;
use hs_tools::{
    AudioControl, DomoticsControl, EntityLite, RuntimeControl, SystemControl, ToolPorts,
    WebhookClient,
};

/// Runtime operations exposed to the agent.
pub struct RuntimeAdapter {
    /// Runtime provider.
    pub runtime: Arc<dyn RuntimeProvider>,
}

#[async_trait]
impl RuntimeControl for RuntimeAdapter {
    async fn list_instances(&self) -> Result<Vec<hs_core::models::InstanceSummary>, String> {
        self.runtime.list().await.map_err(|err| err.to_string())
    }

    async fn start_instance(&self, name: &str) -> Result<(), String> {
        self.runtime
            .start(name)
            .await
            .map_err(|err| err.to_string())
    }

    async fn stop_instance(&self, name: &str) -> Result<(), String> {
        self.runtime.stop(name).await.map_err(|err| err.to_string())
    }
}

/// Home Assistant operations exposed to the agent.
pub struct DomoticsAdapter {
    /// Home Assistant service.
    pub ha: Arc<HaService>,
}

#[async_trait]
impl DomoticsControl for DomoticsAdapter {
    async fn list_entities(&self) -> Result<Vec<EntityLite>, String> {
        let entities = self.ha.entities().await.map_err(|err| err.to_string())?;
        Ok(entities
            .into_iter()
            .map(|entity| EntityLite {
                entity_id: entity.entity_id,
                state: entity.state,
                name: entity.name,
            })
            .collect())
    }

    async fn call_service(
        &self,
        domain: &str,
        service: &str,
        data: serde_json::Value,
    ) -> Result<(), String> {
        self.ha
            .call_service(domain, service, data)
            .await
            .map_err(|err| err.to_string())
    }
}

/// Music/audio operations exposed to the agent.
///
/// Playback is delegated to the first Home Assistant `media_player` entity; the
/// master volume is applied through PipeWire/WirePlumber via `hs-system`.
pub struct AudioAdapter {
    /// Home Assistant service.
    pub ha: Arc<HaService>,
    /// System hardware service.
    pub system: Arc<SystemService>,
}

#[async_trait]
impl AudioControl for AudioAdapter {
    async fn play(&self) -> Result<(), String> {
        let entity_id = self
            .first_media_player()
            .await?
            .ok_or_else(|| "no Home Assistant media player available".to_owned())?;
        self.ha
            .call_service(
                "media_player",
                "media_play",
                serde_json::json!({"entity_id": entity_id}),
            )
            .await
            .map_err(|err| err.to_string())
    }

    async fn pause(&self) -> Result<(), String> {
        let entity_id = self
            .first_media_player()
            .await?
            .ok_or_else(|| "no Home Assistant media player available".to_owned())?;
        self.ha
            .call_service(
                "media_player",
                "media_pause",
                serde_json::json!({"entity_id": entity_id}),
            )
            .await
            .map_err(|err| err.to_string())
    }

    async fn set_volume(&self, percent: u8) -> Result<(), String> {
        self.system
            .set_volume(percent)
            .map_err(|err| err.to_string())
    }
}

impl AudioAdapter {
    async fn first_media_player(&self) -> Result<Option<String>, String> {
        let entities = self.ha.entities().await.map_err(|err| err.to_string())?;
        Ok(entities
            .into_iter()
            .find(|entity| entity.domain == "media_player")
            .map(|entity| entity.entity_id))
    }
}

/// Host metrics exposed to the agent.
pub struct SystemAdapter {
    /// System service.
    pub system: Arc<SystemService>,
}

#[async_trait]
impl SystemControl for SystemAdapter {
    async fn overview(&self) -> Result<serde_json::Value, String> {
        let overview = self
            .system
            .overview()
            .await
            .map_err(|err| err.to_string())?;
        serde_json::to_value(overview).map_err(|err| err.to_string())
    }
}

/// HTTP webhook delivery for automations (n8n, arbitrary endpoints).
pub struct WebhookAdapter {
    /// Shared HTTP client.
    pub http: reqwest::Client,
}

#[async_trait]
impl WebhookClient for WebhookAdapter {
    async fn post(&self, url: &str, payload: serde_json::Value) -> Result<u16, String> {
        hs_core::validation::http_url(url).map_err(|err| err.to_string())?;
        let response = self
            .http
            .post(url)
            .json(&payload)
            .send()
            .await
            .map_err(|err| err.to_string())?;
        Ok(response.status().as_u16())
    }
}

/// Build the port bundle injected into the tool registry.
#[must_use]
pub fn build_ports(
    runtime: Arc<dyn RuntimeProvider>,
    system: Arc<SystemService>,
    ha: Arc<HaService>,
    data_dir: &std::path::Path,
    timers: Arc<crate::timers::TimerService>,
) -> ToolPorts {
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .user_agent(concat!("HomeServerAssistant/", env!("CARGO_PKG_VERSION")))
        .build()
        .unwrap_or_default();
    ToolPorts {
        runtime: Arc::new(RuntimeAdapter { runtime }),
        domotics: Arc::new(DomoticsAdapter { ha: ha.clone() }),
        audio: Arc::new(AudioAdapter {
            ha: ha.clone(),
            system: system.clone(),
        }),
        system: Arc::new(SystemAdapter { system }),
        webhooks: Arc::new(WebhookAdapter { http }),
        memory: Arc::new(crate::memory::FileMemoryStore::new(data_dir.join("memory"))),
        timers,
    }
}
