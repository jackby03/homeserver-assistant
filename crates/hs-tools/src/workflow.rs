//! Node-based workflow execution.
//!
//! A [`Workflow`] is a directed graph of nodes. The engine starts at the
//! `trigger` node and follows a linear chain, executing `tool`, `condition` and
//! `delay` nodes. Tool nodes go through [`ToolRegistry::invoke`], so the active
//! profile permissions are enforced exactly like a direct agent tool call.
//!
//! Cyclic graphs and graphs longer than [`MAX_WORKFLOW_STEPS`] executed nodes
//! are rejected before they can loop forever.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use hs_core::models::{Profile, Workflow, WorkflowNode, WorkflowNodeKind};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::error::{ToolError, ToolResult};
use crate::registry::ToolRegistry;

/// Maximum number of nodes executed by a single workflow run.
pub const MAX_WORKFLOW_STEPS: usize = 64;

/// Maximum allowed delay of a `delay` node, in seconds.
pub const MAX_DELAY_SECONDS: u64 = 3600;

/// Trace of a single executed workflow node.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct WorkflowStep {
    /// Node identifier.
    pub node_id: String,
    /// Node kind: `tool`, `condition` or `delay`.
    pub kind: String,
    /// Whether the profile was allowed to run this step. Always `true` except
    /// for a denied tool call.
    pub allowed: bool,
    /// Structured output produced by the step.
    pub output: Option<Value>,
    /// Error message when the step failed.
    pub error: Option<String>,
    /// Wall-clock duration in milliseconds.
    pub duration_ms: u64,
}

/// Result of one workflow run.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct WorkflowRun {
    /// Workflow identifier.
    pub workflow_id: Uuid,
    /// `false` when a tool was denied or failed; a condition that does not match
    /// ends the run successfully.
    pub success: bool,
    /// Executed steps, in order.
    pub steps: Vec<WorkflowStep>,
    /// Final output: the last tool output, or a structured explanation when the
    /// run stopped on a condition.
    pub output: Value,
}

/// Executes workflows against a tool registry.
pub struct WorkflowEngine {
    registry: Arc<ToolRegistry>,
}

impl WorkflowEngine {
    /// Create an engine that executes tool nodes through `registry`.
    #[must_use]
    pub fn new(registry: Arc<ToolRegistry>) -> Self {
        Self { registry }
    }

    /// Execute `workflow` on behalf of `profile` with `input`.
    ///
    /// # Errors
    /// Returns [`ToolError::Execution`] when the workflow is disabled,
    /// malformed (missing trigger, unknown node, duplicate node, cycle or too
    /// many steps) and [`ToolError::InvalidArguments`] when a `delay` node
    /// exceeds [`MAX_DELAY_SECONDS`]. Tool failures are reported inside the
    /// returned [`WorkflowRun`] instead, so the caller can display the trace.
    pub async fn run(
        &self,
        workflow: &Workflow,
        profile: &Profile,
        input: Value,
    ) -> ToolResult<WorkflowRun> {
        if !workflow.enabled {
            return Err(ToolError::Execution(format!(
                "workflow `{}` is disabled",
                workflow.name
            )));
        }

        let mut nodes: BTreeMap<&str, &WorkflowNode> = BTreeMap::new();
        for node in &workflow.nodes {
            if nodes.insert(node.id.as_str(), node).is_some() {
                return Err(ToolError::Execution(format!(
                    "workflow contains duplicate node id `{}`",
                    node.id
                )));
            }
        }

        let mut successors: BTreeMap<&str, &str> = BTreeMap::new();
        for edge in &workflow.edges {
            if !nodes.contains_key(edge.from.as_str()) {
                return Err(ToolError::Execution(format!(
                    "workflow edge references unknown node `{}`",
                    edge.from
                )));
            }
            if !nodes.contains_key(edge.to.as_str()) {
                return Err(ToolError::Execution(format!(
                    "workflow edge references unknown node `{}`",
                    edge.to
                )));
            }
            if let Some(previous) = successors.insert(edge.from.as_str(), edge.to.as_str()) {
                tracing::warn!(
                    from = %edge.from,
                    followed = %previous,
                    ignored = %edge.to,
                    "workflow node has multiple outgoing edges; the first one is followed"
                );
            }
        }

        let Some(trigger) = workflow
            .nodes
            .iter()
            .find(|node| matches!(node.kind, WorkflowNodeKind::Trigger { .. }))
        else {
            return Err(ToolError::Execution(
                "workflow has no trigger node".to_owned(),
            ));
        };

        let trigger_id = trigger.id.as_str();
        let mut visited: BTreeSet<&str> = BTreeSet::new();
        visited.insert(trigger_id);
        let mut current = successors.get(trigger_id).copied();
        let mut steps: Vec<WorkflowStep> = Vec::new();
        let mut previous_output: Option<Value> = None;

        while let Some(node_id) = current {
            if steps.len() >= MAX_WORKFLOW_STEPS {
                return Err(ToolError::Execution(format!(
                    "workflow exceeded the maximum of {MAX_WORKFLOW_STEPS} executed steps"
                )));
            }
            if !visited.insert(node_id) {
                return Err(ToolError::Execution(format!(
                    "workflow contains a cycle at node `{node_id}`"
                )));
            }
            let Some(node) = nodes.get(node_id).copied() else {
                return Err(ToolError::Execution(format!(
                    "workflow references unknown node `{node_id}`"
                )));
            };

            let started = Instant::now();
            let mut step = WorkflowStep {
                node_id: node.id.clone(),
                kind: node_kind_name(&node.kind).to_owned(),
                allowed: true,
                output: None,
                error: None,
                duration_ms: 0,
            };

            match &node.kind {
                WorkflowNodeKind::Trigger { .. } => {
                    return Err(ToolError::Execution(format!(
                        "node `{}` is a trigger but was reached as a regular step",
                        node.id
                    )));
                }
                WorkflowNodeKind::Tool { tool, arguments } => {
                    let resolved =
                        substitute_arguments(arguments, &input, previous_output.as_ref());
                    match self.registry.invoke(profile, tool, resolved).await {
                        Ok(outcome) => {
                            step.duration_ms = outcome.duration_ms;
                            step.output = Some(outcome.output.clone());
                            previous_output = Some(outcome.output);
                        }
                        Err(error) => {
                            step.duration_ms = elapsed_ms(started);
                            let message = match &error {
                                ToolError::Forbidden { tool } => {
                                    format!("permission denied for tool `{tool}`")
                                }
                                other => other.to_string(),
                            };
                            if matches!(error, ToolError::Forbidden { .. }) {
                                step.allowed = false;
                            }
                            step.error = Some(message.clone());
                            steps.push(step);
                            return Ok(WorkflowRun {
                                workflow_id: workflow.id,
                                success: false,
                                output: json!({ "error": message, "node_id": node.id }),
                                steps,
                            });
                        }
                    }
                }
                WorkflowNodeKind::Condition { field, equals } => {
                    let actual = previous_output
                        .as_ref()
                        .and_then(|value| dotted_lookup(value, field))
                        .cloned()
                        .unwrap_or(Value::Null);
                    let matched = &actual == equals;
                    step.duration_ms = elapsed_ms(started);
                    step.output = Some(json!({
                        "field": field,
                        "actual": actual,
                        "expected": equals,
                        "matched": matched
                    }));
                    if !matched {
                        steps.push(step);
                        return Ok(WorkflowRun {
                            workflow_id: workflow.id,
                            success: true,
                            output: json!({
                                "stopped": "condition_false",
                                "node_id": node.id,
                                "field": field,
                                "actual": actual,
                                "expected": equals
                            }),
                            steps,
                        });
                    }
                }
                WorkflowNodeKind::Delay { seconds } => {
                    if *seconds > MAX_DELAY_SECONDS {
                        return Err(ToolError::InvalidArguments(format!(
                            "delay node `{}` requests {seconds} seconds, the maximum is {MAX_DELAY_SECONDS}",
                            node.id
                        )));
                    }
                    if *seconds > 0 {
                        tokio::time::sleep(Duration::from_secs(*seconds)).await;
                    }
                    step.duration_ms = elapsed_ms(started);
                    step.output = Some(json!({ "delayed_seconds": seconds }));
                }
            }

            steps.push(step);
            current = successors.get(node_id).copied();
        }

        Ok(WorkflowRun {
            workflow_id: workflow.id,
            success: true,
            steps,
            output: previous_output.unwrap_or(Value::Null),
        })
    }
}

fn node_kind_name(kind: &WorkflowNodeKind) -> &'static str {
    match kind {
        WorkflowNodeKind::Trigger { .. } => "trigger",
        WorkflowNodeKind::Tool { .. } => "tool",
        WorkflowNodeKind::Condition { .. } => "condition",
        WorkflowNodeKind::Delay { .. } => "delay",
    }
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// Resolve `a.b[2].c` style paths inside structured tool output.
fn dotted_lookup<'value>(value: &'value Value, path: &str) -> Option<&'value Value> {
    let mut current = value;
    for segment in path.split('.').filter(|segment| !segment.is_empty()) {
        current = match current {
            Value::Object(map) => map.get(segment)?,
            Value::Array(items) => items.get(segment.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(current)
}

/// Replace `{{input}}` and `{{previous.output}}` placeholders in tool arguments.
fn substitute_arguments(value: &Value, input: &Value, previous: Option<&Value>) -> Value {
    match value {
        Value::String(text) => substitute_string(text, input, previous),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| substitute_arguments(item, input, previous))
                .collect(),
        ),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, item)| (key.clone(), substitute_arguments(item, input, previous)))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn substitute_string(text: &str, input: &Value, previous: Option<&Value>) -> Value {
    const INPUT: &str = "{{input}}";
    const PREVIOUS: &str = "{{previous.output}}";

    let previous_value = previous.cloned().unwrap_or(Value::Null);
    if text == INPUT {
        return input.clone();
    }
    if text == PREVIOUS {
        return previous_value;
    }
    if !text.contains(INPUT) && !text.contains(PREVIOUS) {
        return Value::String(text.to_owned());
    }

    let mut rendered = text.to_owned();
    for (placeholder, value) in [(INPUT, input), (PREVIOUS, &previous_value)] {
        let replacement = match value {
            Value::String(text) => text.clone(),
            other => other.to_string(),
        };
        rendered = rendered.replace(placeholder, &replacement);
    }
    Value::String(rendered)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{fake_ports, profile_with_tools, CallCounts};
    use hs_core::models::{
        Workflow, WorkflowEdge, WorkflowNode, WorkflowNodeKind, WorkflowTrigger,
    };
    use std::sync::Arc;

    fn node(id: &str, kind: WorkflowNodeKind) -> WorkflowNode {
        WorkflowNode {
            id: id.to_owned(),
            kind,
        }
    }

    fn trigger(id: &str) -> WorkflowNode {
        node(
            id,
            WorkflowNodeKind::Trigger {
                trigger: WorkflowTrigger::Manual,
            },
        )
    }

    fn tool_node(id: &str, tool: &str, arguments: Value) -> WorkflowNode {
        node(
            id,
            WorkflowNodeKind::Tool {
                tool: tool.to_owned(),
                arguments,
            },
        )
    }

    fn condition_node(id: &str, field: &str, equals: Value) -> WorkflowNode {
        node(
            id,
            WorkflowNodeKind::Condition {
                field: field.to_owned(),
                equals,
            },
        )
    }

    fn delay_node(id: &str, seconds: u64) -> WorkflowNode {
        node(id, WorkflowNodeKind::Delay { seconds })
    }

    fn edge(from: &str, to: &str) -> WorkflowEdge {
        WorkflowEdge {
            from: from.to_owned(),
            to: to.to_owned(),
        }
    }

    fn workflow(nodes: Vec<WorkflowNode>, edges: Vec<WorkflowEdge>) -> Workflow {
        Workflow {
            id: Uuid::new_v4(),
            name: "Test workflow".to_owned(),
            description: "Workflow used by unit tests".to_owned(),
            enabled: true,
            nodes,
            edges,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    fn fixture(
        profile_tools: &[&str],
    ) -> (
        WorkflowEngine,
        hs_core::models::Profile,
        Arc<CallCounts>,
        Arc<Mutex<Option<Value>>>,
    ) {
        let (ports, counts, webhook_payload) = fake_ports();
        let registry = ToolRegistry::with_ports(ports);
        let engine = WorkflowEngine::new(registry);
        let profile = profile_with_tools(profile_tools);
        (engine, profile, counts, webhook_payload)
    }

    use std::sync::Mutex;

    #[tokio::test]
    async fn runs_tool_then_matching_condition_and_continues() {
        let (engine, profile, counts, _payload) = fixture(&["system.status", "music.play"]);
        let workflow = workflow(
            vec![
                trigger("start"),
                tool_node("status", "system.status", json!({})),
                condition_node("check", "status", json!("ok")),
                tool_node("play", "music.play", json!({})),
            ],
            vec![
                edge("start", "status"),
                edge("status", "check"),
                edge("check", "play"),
            ],
        );
        let run = engine
            .run(&workflow, &profile, json!({}))
            .await
            .expect("workflow should run");
        assert!(run.success);
        assert_eq!(run.steps.len(), 3);
        assert_eq!(
            run.steps
                .iter()
                .map(|step| step.kind.as_str())
                .collect::<Vec<_>>(),
            vec!["tool", "condition", "tool"]
        );
        assert_eq!(run.output, json!({ "status": "playing" }));
        let [runtime, _domotics, audio, system, _webhooks] = counts.snapshot();
        assert_eq!(runtime, 0);
        assert_eq!(audio, 1);
        assert_eq!(system, 1);
    }

    #[tokio::test]
    async fn mismatched_condition_stops_the_run_successfully() {
        let (engine, profile, counts, _payload) = fixture(&["system.status", "music.play"]);
        let workflow = workflow(
            vec![
                trigger("start"),
                tool_node("status", "system.status", json!({})),
                condition_node("check", "status", json!("down")),
                tool_node("play", "music.play", json!({})),
            ],
            vec![
                edge("start", "status"),
                edge("status", "check"),
                edge("check", "play"),
            ],
        );
        let run = engine
            .run(&workflow, &profile, json!({}))
            .await
            .expect("condition mismatch is not an engine error");
        assert!(run.success);
        assert_eq!(run.steps.len(), 2);
        assert_eq!(run.output["stopped"], "condition_false");
        assert_eq!(run.output["field"], "status");
        assert_eq!(counts.audio.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn denied_tool_step_is_marked_and_never_executes() {
        let (engine, profile, counts, _payload) = fixture(&["system.status"]);
        let workflow = workflow(
            vec![trigger("start"), tool_node("play", "music.play", json!({}))],
            vec![edge("start", "play")],
        );
        let run = engine
            .run(&workflow, &profile, json!({}))
            .await
            .expect("denied tool is reported inside the run");
        assert!(!run.success);
        assert_eq!(run.steps.len(), 1);
        assert!(!run.steps[0].allowed);
        assert_eq!(run.steps[0].kind, "tool");
        assert!(run.steps[0].output.is_none());
        assert!(
            run.steps[0]
                .error
                .as_deref()
                .is_some_and(|error| error.contains("permission denied")),
            "unexpected error: {:?}",
            run.steps[0].error
        );
        assert_eq!(counts.audio.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn zero_second_delay_is_allowed_and_recorded() {
        let (engine, profile, counts, _payload) = fixture(&["system.status"]);
        let workflow = workflow(
            vec![
                trigger("start"),
                delay_node("pause", 0),
                tool_node("status", "system.status", json!({})),
            ],
            vec![edge("start", "pause"), edge("pause", "status")],
        );
        let run = engine
            .run(&workflow, &profile, json!({}))
            .await
            .expect("zero delay must be accepted");
        assert!(run.success);
        assert_eq!(run.steps.len(), 2);
        assert_eq!(run.steps[0].kind, "delay");
        assert_eq!(run.steps[0].output, Some(json!({ "delayed_seconds": 0 })));
        assert_eq!(counts.system.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn cycles_are_detected() {
        let (engine, profile, _counts, _payload) = fixture(&[]);
        let workflow = workflow(
            vec![trigger("start"), delay_node("pause", 0)],
            vec![edge("start", "pause"), edge("pause", "start")],
        );
        let error = engine
            .run(&workflow, &profile, json!({}))
            .await
            .expect_err("cyclic workflow must be rejected");
        assert!(
            matches!(&error, ToolError::Execution(message) if message.contains("cycle")),
            "unexpected error: {error:?}"
        );
    }

    #[tokio::test]
    async fn too_many_steps_are_rejected() {
        let (engine, profile, _counts, _payload) = fixture(&[]);
        let mut nodes = vec![trigger("start")];
        let mut edges = Vec::new();
        for index in 0..70 {
            let id = format!("delay-{index}");
            nodes.push(delay_node(&id, 0));
            let previous = if index == 0 {
                "start".to_owned()
            } else {
                format!("delay-{}", index - 1)
            };
            edges.push(edge(&previous, &id));
        }
        let workflow = workflow(nodes, edges);
        let error = engine
            .run(&workflow, &profile, json!({}))
            .await
            .expect_err("step limit must be enforced");
        assert!(
            matches!(&error, ToolError::Execution(message) if message.contains("maximum")),
            "unexpected error: {error:?}"
        );
    }

    #[tokio::test]
    async fn malformed_workflows_are_rejected() {
        let (engine, profile, _counts, _payload) = fixture(&[]);

        let no_trigger = workflow(
            vec![tool_node("status", "system.status", json!({}))],
            Vec::new(),
        );
        assert!(engine.run(&no_trigger, &profile, json!({})).await.is_err());

        let unknown_target = workflow(vec![trigger("start")], vec![edge("start", "missing")]);
        assert!(engine
            .run(&unknown_target, &profile, json!({}))
            .await
            .is_err());

        let duplicate = workflow(vec![trigger("start"), trigger("start")], Vec::new());
        assert!(engine.run(&duplicate, &profile, json!({})).await.is_err());

        let mut disabled = workflow(vec![trigger("start")], Vec::new());
        disabled.enabled = false;
        assert!(engine.run(&disabled, &profile, json!({})).await.is_err());

        let too_long_delay = workflow(
            vec![trigger("start"), delay_node("pause", 3601)],
            vec![edge("start", "pause")],
        );
        let error = engine
            .run(&too_long_delay, &profile, json!({}))
            .await
            .expect_err("delay over one hour must be rejected");
        assert!(matches!(error, ToolError::InvalidArguments(_)), "{error:?}");
    }

    #[tokio::test]
    async fn arguments_are_substituted_from_input_and_previous_output() {
        let (engine, profile, counts, payload) = fixture(&["system.status", "http.webhook"]);
        let workflow = workflow(
            vec![
                trigger("start"),
                tool_node("status", "system.status", json!({})),
                tool_node(
                    "notify",
                    "http.webhook",
                    json!({
                        "url": "https://hooks.example.test/notify",
                        "payload": {
                            "previous": "{{previous.output}}",
                            "input": "{{input}}",
                            "label": "status is {{previous.output}}"
                        }
                    }),
                ),
            ],
            vec![edge("start", "status"), edge("status", "notify")],
        );
        let input = json!({ "message": "hola" });
        let run = engine
            .run(&workflow, &profile, input.clone())
            .await
            .expect("workflow should run");
        assert!(run.success);
        assert_eq!(counts.webhooks.load(std::sync::atomic::Ordering::SeqCst), 1);

        let delivered = payload
            .lock()
            .expect("webhook payload lock")
            .clone()
            .expect("webhook must be called");
        assert_eq!(delivered["input"], input);
        assert_eq!(delivered["previous"]["status"], "ok");
        assert!(
            delivered["label"]
                .as_str()
                .is_some_and(|label| label.contains("\"status\":\"ok\"")),
            "unexpected label: {}",
            delivered["label"]
        );
    }

    #[tokio::test]
    async fn tool_errors_end_the_run_without_success() {
        let (engine, profile, counts, _payload) = fixture(&["http.webhook"]);
        let workflow = workflow(
            vec![
                trigger("start"),
                tool_node(
                    "notify",
                    "http.webhook",
                    json!({ "url": "not-a-valid-url" }),
                ),
            ],
            vec![edge("start", "notify")],
        );
        let run = engine
            .run(&workflow, &profile, json!({}))
            .await
            .expect("tool failures are reported inside the run");
        assert!(!run.success);
        assert_eq!(run.steps.len(), 1);
        assert!(run.steps[0].allowed);
        assert!(
            run.steps[0]
                .error
                .as_deref()
                .is_some_and(|error| error.contains("invalid tool arguments")),
            "unexpected error: {:?}",
            run.steps[0].error
        );
        assert_eq!(counts.webhooks.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[test]
    fn dotted_lookup_descends_objects_and_arrays() {
        let value = json!({
            "cpu": { "usage_percent": 7.5 },
            "items": [ { "state": "on" } ]
        });
        assert_eq!(
            dotted_lookup(&value, "cpu.usage_percent"),
            Some(&json!(7.5))
        );
        assert_eq!(dotted_lookup(&value, "items.0.state"), Some(&json!("on")));
        assert!(dotted_lookup(&value, "cpu.missing").is_none());
        assert!(dotted_lookup(&value, "items.nope").is_none());
        assert_eq!(dotted_lookup(&value, ""), Some(&value));
    }
}
