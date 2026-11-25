//! Integration smoke test: wires `hs-profiles`, `hs-tools` and `hs-agent`
//! exactly like the `hs-daemon` composition root does.
//!
//! It exercises the public constructors and adapter ports the daemon must
//! implement, but performs no network I/O: the `DefaultFactory` is only
//! constructed and asked to build a client.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use hs_agent::{AgentService, DefaultFactory, LlmClientFactory, UpsertProviderRequest};
use hs_core::config::LlmProviderKind;
use hs_core::models::{
    InstanceSummary, Workflow, WorkflowEdge, WorkflowNode, WorkflowNodeKind, WorkflowTrigger,
};
use hs_core::state::{Database, SecretStore, SecretVault};
use hs_profiles::{ProfileService, Tone};
use hs_tools::{
    AudioControl, DomoticsControl, EntityLite, RuntimeControl, SystemControl, ToolPorts,
    ToolRegistry, WebhookClient, WorkflowEngine,
};
use serde_json::{json, Value};
use uuid::Uuid;

struct FakeRuntime;

#[async_trait]
impl RuntimeControl for FakeRuntime {
    async fn list_instances(&self) -> Result<Vec<InstanceSummary>, String> {
        Ok(Vec::new())
    }

    async fn start_instance(&self, _name: &str) -> Result<(), String> {
        Ok(())
    }

    async fn stop_instance(&self, _name: &str) -> Result<(), String> {
        Ok(())
    }
}

struct FakeDomotics;

#[async_trait]
impl DomoticsControl for FakeDomotics {
    async fn list_entities(&self) -> Result<Vec<EntityLite>, String> {
        Ok(Vec::new())
    }

    async fn call_service(
        &self,
        _domain: &str,
        _service: &str,
        _data: Value,
    ) -> Result<(), String> {
        Ok(())
    }
}

struct FakeAudio;

#[async_trait]
impl AudioControl for FakeAudio {
    async fn play(&self) -> Result<(), String> {
        Ok(())
    }

    async fn pause(&self) -> Result<(), String> {
        Ok(())
    }

    async fn set_volume(&self, _percent: u8) -> Result<(), String> {
        Ok(())
    }
}

struct FakeSystem {
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl SystemControl for FakeSystem {
    async fn overview(&self) -> Result<Value, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(json!({ "status": "ok", "cpu": { "usage_percent": 5.0 } }))
    }
}

struct FakeWebhooks;

#[async_trait]
impl WebhookClient for FakeWebhooks {
    async fn post(&self, _url: &str, _payload: Value) -> Result<u16, String> {
        Ok(204)
    }
}

fn workflow() -> Workflow {
    Workflow {
        id: Uuid::new_v4(),
        name: "Status check".to_owned(),
        description: String::new(),
        enabled: true,
        nodes: vec![
            WorkflowNode {
                id: "start".to_owned(),
                kind: WorkflowNodeKind::Trigger {
                    trigger: WorkflowTrigger::Manual,
                },
            },
            WorkflowNode {
                id: "status".to_owned(),
                kind: WorkflowNodeKind::Tool {
                    tool: "system.status".to_owned(),
                    arguments: json!({}),
                },
            },
        ],
        edges: vec![WorkflowEdge {
            from: "start".to_owned(),
            to: "status".to_owned(),
        }],
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

#[tokio::test]
async fn daemon_wiring_compiles_and_runs() {
    let db = Arc::new(Database::open_in_memory().expect("in-memory database"));
    let secrets = Arc::new(SecretStore::new(Arc::clone(&db), SecretVault::generate()));
    let profiles = Arc::new(ProfileService::new(Arc::clone(&db), Arc::clone(&secrets)));

    let system_calls = Arc::new(AtomicUsize::new(0));
    let ports = ToolPorts {
        runtime: Arc::new(FakeRuntime),
        domotics: Arc::new(FakeDomotics),
        audio: Arc::new(FakeAudio),
        system: Arc::new(FakeSystem {
            calls: Arc::clone(&system_calls),
        }),
        webhooks: Arc::new(FakeWebhooks),
        memory: Arc::new(hs_tools::EphemeralMemoryStore::default()),
    };
    // Exact constructor used by the daemon composition root.
    let tools: Arc<ToolRegistry> = ToolRegistry::builtin(ports);
    assert_eq!(tools.list().len(), 13);

    // The onboarding wizard consumes the built-in profile requests.
    let request = ProfileService::default_profiles()
        .into_iter()
        .next()
        .expect("built-in profiles exist");
    let profile = profiles.create(request).expect("profile must be created");
    assert!(profile.may_use_tool("system.status"));
    assert_eq!(profile.language, "es");
    assert_eq!(profile.tone, Tone::Friendly);

    let engine = WorkflowEngine::new(Arc::clone(&tools));
    let run = engine
        .run(&workflow(), &profile, json!({}))
        .await
        .expect("workflow should run");
    assert!(run.success);
    assert_eq!(run.steps.len(), 1);
    assert_eq!(run.output["status"], "ok");
    assert_eq!(system_calls.load(Ordering::SeqCst), 1);

    // Real default factory; no request is sent in this test.
    let factory: Arc<dyn LlmClientFactory> = Arc::new(DefaultFactory::default());
    let agent = AgentService::new(
        Arc::clone(&db),
        Arc::clone(&secrets),
        Arc::clone(&profiles),
        Arc::clone(&tools),
        factory,
        0.7,
        6,
    );
    let provider = agent
        .upsert_provider(UpsertProviderRequest {
            kind: LlmProviderKind::OpenAi,
            label: "OpenAI".to_owned(),
            model: "gpt-4.1-mini".to_owned(),
            base_url: None,
            api_key: Some("sk-integration-test".to_owned()),
            enabled: true,
        })
        .expect("provider must be created");
    assert_eq!(agent.list_providers().expect("providers").len(), 1);
    agent
        .set_active_provider(provider.id)
        .await
        .expect("provider can be selected");
    assert_eq!(agent.active_provider().await, Some(provider.id));
    assert!(secrets
        .contains(&provider.api_key_secret)
        .expect("secret lookup"));
    assert!(!serde_json::to_string(&provider)
        .expect("serialize provider")
        .contains("sk-integration-test"));

    // The factory builds a client for the stored record without network I/O.
    let factory = DefaultFactory::default();
    assert!(factory.create(&provider, "sk-test").await.is_ok());

    assert!(agent
        .delete_provider(provider.id)
        .await
        .expect("delete provider"));
    assert!(agent.active_provider().await.is_none());
    assert!(agent.list_providers().expect("providers").is_empty());
}
