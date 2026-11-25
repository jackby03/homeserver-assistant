/**
 * API types mirroring the Rust serde models exposed by `hs-daemon`.
 *
 * Rules:
 * - JSON keys are snake_case exactly as serde serializes them.
 * - Timestamps are RFC 3339 UTC strings.
 * - IDs are UUID strings.
 * - Optional values that serde may omit are typed with `?`, values that are
 *   always present but nullable (Rust `Option<T>`) are typed as `| null`.
 *
 * These types are the single source of truth for the UI; do not widen them in
 * feature code.
 */

/* -------------------------------------------------------------------------- */
/* Shared                                                                     */
/* -------------------------------------------------------------------------- */

export type Uuid = string;
export type IsoTimestamp = string;

export type AdminRole = 'admin' | 'operator' | 'viewer';

export interface ApiErrorPayload {
  error: {
    code: string;
    message: string;
    details?: unknown;
  };
}

/* -------------------------------------------------------------------------- */
/* Auth, setup and system                                                     */
/* -------------------------------------------------------------------------- */

export interface HealthResponse {
  status: 'ok';
  version: string;
}

export interface SetupStatus {
  initialized: boolean;
  device_name: string;
  language: string;
  completed_steps: number[];
}

export interface User {
  id: Uuid;
  username: string;
  display_name: string;
  role: AdminRole;
}

export interface LoginRequest {
  username: string;
  password: string;
}

export interface SessionResponse {
  user: User;
  csrf_token: string;
}

export type NetworkMode = 'dhcp' | 'static';
export type NetworkSegment = 'lan' | 'dmz';
export type DnsEngine = 'adguard_home' | 'technitium';
/** Catalog id (validated by the daemon against `hs_core::dns::DNS_PROVIDERS`). */
export type DnsProvider = string;
export type DnsProtocol = 'doh' | 'dot' | 'plain';

/** One free-DNS catalog entry returned by `GET /network/dns/providers`. */
export interface DnsProviderInfo {
  id: string;
  label: string;
  vendor: { id: string; name: string };
  ipv4: string[];
  ipv6: string[];
  doh: string | null;
  dot: string | null;
  plain: boolean;
  requires_config_id: boolean;
  note: string | null;
}

export interface VoiceActivationMode {
  mode: 'wake_word' | 'push_to_talk' | 'both';
}

export interface VoiceConfig {
  assistant_name: string;
  wake_word_enabled: boolean;
  activation_mode: 'wake_word' | 'push_to_talk' | 'both';
  sensitivity: number;
  listening_window_secs: number;
  language: string;
}

export interface VoiceStatus {
  assistant_name: string;
  wake_word_enabled: boolean;
  activation_mode: 'wake_word' | 'push_to_talk' | 'both';
  listening: boolean;
  last_activation: string | null;
  activations: number;
  listener_connected: boolean;
}

export interface VoiceConfigResponse {
  config: VoiceConfig;
  status: VoiceStatus;
}

export interface WakeOutcome {
  activated: boolean;
  assistant_name: string;
  listening: boolean;
  trailing_query: string | null;
  reason: string;
}

export interface StaticIpv4 {
  address: string;
  prefix_len: number;
  gateway: string;
}

export interface DnsConfig {
  enabled: boolean;
  engine: DnsEngine;
  provider: DnsProvider;
  protocol: DnsProtocol;
  custom_upstream?: string | null;
  blocklists: string[];
}

export interface DmzConfig {
  enabled: boolean;
  bridge: string;
  subnet: string;
  gateway: string;
  allow_dns: boolean;
}

export interface NetworkConfig {
  interface: string;
  /** LAN bridge owned by the appliance (advanced; server defaults to br-lan). */
  lan_bridge?: string;
  mode: NetworkMode;
  static_ipv4: StaticIpv4 | null;
  dns: DnsConfig;
  dmz: DmzConfig;
}

export interface DeviceConfigInput {
  name: string;
  language: string;
  timezone: string;
}

export interface SetupAdminRequest {
  username: string;
  password: string;
  device: DeviceConfigInput;
}export interface SetupNetworkRequest {
  interface: string;
  mode: NetworkMode;
  static_ipv4?: StaticIpv4;
  dns: DnsConfig;
  dmz: DmzConfig;
}

export interface AudioSink {
  id: string;
  name: string;
  description: string;
  is_default: boolean;
}

/**
 * Peripheral entries are intentionally loose: the daemon reports camera and
 * touch devices from a crate shared with `hs-system`, and only the label is
 * needed by the wizard. Use {@link peripheralLabel} to render them safely.
 */
export interface SetupPeripherals {
  camera_devices: unknown[];
  touch_devices: unknown[];
  audio_sinks: AudioSink[];
}

export interface AudioTestResponse {
  played: boolean;
}

export interface SetupProfileInput {
  name: string;
  role: ProfileRole;
  avatar: Avatar;
  language: string;
  tone: Tone;
  allowed_tools: string[];
}

export interface SetupProfilesRequest {
  profiles: SetupProfileInput[];
}

export type LlmProviderKind = 'openai' | 'anthropic' | 'deepseek' | 'opencode_go' | 'custom';

export interface SetupAgentRequest {
  kind?: LlmProviderKind;
  label?: string;
  model?: string;
  base_url?: string;
  api_key?: string;
}

export interface SetupFinishResponse {
  job_id: string;
}

export interface SetupProgressMessage {
  type: 'progress';
  step: string;
  label: string;
  percent: number;
  done: boolean;
}

export interface SetupFinishedMessage {
  type: 'finished';
}

export type SetupWsMessage = SetupProgressMessage | SetupFinishedMessage;

/* -------------------------------------------------------------------------- */
/* System overview                                                            */
/* -------------------------------------------------------------------------- */

export interface CpuInfo {
  usage_percent: number;
  cores: number;
  load_1: number;
  load_5: number;
  load_15: number;
}

export interface MemoryInfo {
  total_mb: number;
  used_mb: number;
  available_mb: number;
  swap_total_mb: number;
  swap_used_mb: number;
}

export interface DiskInfo {
  mount: string;
  filesystem: string;
  total_gb: number;
  used_gb: number;
  available_gb: number;
  usage_percent: number;
  kind: string;
}

export interface BatteryInfo {
  present: boolean;
  percent: number;
  charging: boolean;
  time_remaining_min: number | null;
}

export interface NetworkInterfaceStats {
  name: string;
  ipv4: string[];
  mac: string;
  bytes_sent: number;
  bytes_received: number;
}

export interface SystemService {
  name: string;
  active: boolean;
  detail: string | null;
}

export interface SystemOverview {
  hostname: string;
  os: string;
  kernel: string;
  uptime_secs: number;
  cpu: CpuInfo;
  memory: MemoryInfo;
  disks: DiskInfo[];
  temperature_c: number | null;
  battery: BatteryInfo | null;
  network: NetworkInterfaceStats[];
  services: SystemService[];
  generated_at: IsoTimestamp;
}

export interface SystemVersion {
  version: string;
}

export type AuditOutcome = 'success' | 'denied' | 'failure';

export interface AuditEntry {
  seq: number;
  timestamp: IsoTimestamp;
  actor: string;
  action: string;
  target: string | null;
  outcome: AuditOutcome;
  ip: string | null;
  details: unknown;
}

/* -------------------------------------------------------------------------- */
/* Profiles and avatars                                                       */
/* -------------------------------------------------------------------------- */

export type AvatarShape = 'circle' | 'squircle' | 'cloud' | 'cone' | 'droplet';
export type AvatarState =
  | 'idle'
  | 'tracking'
  | 'listening'
  | 'thinking'
  | 'speaking'
  | 'profile_switch';
export type ProfileRole = 'owner' | 'adult' | 'child' | 'guest';
export type Tone = 'friendly' | 'calm' | 'playful' | 'formal';

export interface Avatar {
  shape: AvatarShape;
  color: string;
  eye_color: string;
  eye_tilt_deg: number;
}

export interface TrackingPoint {
  x: number;
  y: number;
  confidence: number;
}

export interface Profile {
  id: Uuid;
  name: string;
  role: ProfileRole;
  avatar: Avatar;
  language: string;
  tone: Tone;
  allowed_tools: string[];
  face_enrolled: boolean;
  created_at: IsoTimestamp;
  updated_at: IsoTimestamp;
}

export interface ProfilesResponse {
  /** Kept for older daemons; current `GET /profiles` returns a bare array. */
  profiles: Profile[];
}

/** Normalize the bare-array profile response used by the daemon. */
export function normalizeProfiles(response: Profile[] | ProfilesResponse): Profile[] {
  return Array.isArray(response) ? response : response.profiles;
}

export interface CreateProfileRequest {
  name: string;
  role: ProfileRole;
  avatar: Avatar;
  language: string;
  tone: Tone;
  allowed_tools: string[];
}

export type UpdateProfileRequest = Partial<CreateProfileRequest>;

export interface FaceEnrollRequest {
  embedding: number[];
}

/* -------------------------------------------------------------------------- */
/* Runtime instances                                                          */
/* -------------------------------------------------------------------------- */

export type RuntimeKind = 'container' | 'vm';
export type RuntimeStatus =
  | 'running'
  | 'stopped'
  | 'starting'
  | 'stopping'
  | 'frozen'
  | 'error'
  | 'unknown';

export interface ResourceLimits {
  cpu_limit: number | null;
  memory_mb: number | null;
}

export interface InstanceSummary {
  name: string;
  kind: RuntimeKind;
  status: RuntimeStatus;
  image: string;
  ipv4_addresses: string[];
  limits: ResourceLimits;
  created_at: IsoTimestamp | null;
  snapshot_count: number;
}

export interface InstanceListResponse {
  instances: InstanceSummary[];
  provider: {
    name: string;
    available: boolean;
  };
}

export interface CreateInstanceRequest {
  name: string;
  kind: RuntimeKind;
  image: string;
  segment: NetworkSegment | null;
  limits: ResourceLimits;
  env: Record<string, string>;
  start: boolean;
}

export interface UpdateInstanceLimitsRequest {
  cpu_limit: number | null;
  memory_mb: number | null;
}

export interface SnapshotSummary {
  name: string;
  created_at: IsoTimestamp | null;
  size_bytes: number;
}

export interface SnapshotListResponse {
  snapshots: SnapshotSummary[];
}

export interface InstanceLogsResponse {
  logs: string;
}

export interface CreateSnapshotRequest {
  name: string;
}

export type ExecClientMessage = { type: 'input'; data: string };
export type ExecServerMessage =
  | { type: 'output'; data: string }
  | { type: 'exit'; code: number };

/* -------------------------------------------------------------------------- */
/* Apps                                                                       */
/* -------------------------------------------------------------------------- */

export type AppCategory =
  | 'automation'
  | 'media'
  | 'networking'
  | 'productivity'
  | 'storage'
  | 'other';

export type AppStatus = 'pending' | 'deploying' | 'running' | 'stopped' | 'failed' | 'removed';

export interface AppTemplateEnv {
  key: string;
  label: string;
  required: boolean;
  secret: boolean;
  default: string | null;
}

export interface AppTemplate {
  id: string;
  name: string;
  description: string;
  image: string;
  default_port: number;
  category: AppCategory;
  segment: NetworkSegment;
  env: AppTemplateEnv[];
  featured: boolean;
}

export interface AppCatalogResponse {
  /** Kept for older daemons; current `GET /apps/catalog` returns a bare array. */
  templates: AppTemplate[];
}

/** Normalize the bare-array catalog response used by the daemon. */
export function normalizeCatalog(
  response: AppTemplate[] | AppCatalogResponse,
): AppTemplate[] {
  return Array.isArray(response) ? response : response.templates;
}

export interface AppRecord {
  id: Uuid;
  name: string;
  instance_name: string;
  template_id: string | null;
  image: string;
  segment: NetworkSegment;
  internal_port: number;
  hostname: string | null;
  expose_externally: boolean;
  env: Record<string, string>;
  status: AppStatus;
  created_at: IsoTimestamp;
  updated_at: IsoTimestamp;
}

export interface AppsResponse {
  apps: AppRecord[];
}

export interface DeployAppRequest {
  name: string;
  template_id?: string;
  image?: string;
  segment: NetworkSegment;
  internal_port: number;
  subdomain?: string;
  expose_externally: boolean;
  env: Record<string, string>;
}

/* -------------------------------------------------------------------------- */
/* Network management                                                         */
/* -------------------------------------------------------------------------- */

export interface NetworkInterface {
  name: string;
  mac: string;
  ipv4: string;
  is_up: boolean;
}

export interface NetworkViewResponse {
  interfaces: NetworkInterface[];
  config: NetworkConfig;
}

export interface FirewallStatusResponse {
  ruleset: string;
  last_applied_at: IsoTimestamp | null;
}

export interface FirewallApplyResponse {
  applied: boolean;
  rules_count: number;
}

export interface DnsTestResponse {
  ok: boolean;
  latency_ms: number | null;
  provider: string;
}

/* -------------------------------------------------------------------------- */
/* Discovery                                                                  */
/* -------------------------------------------------------------------------- */

export type DiscoveryProtocol = 'mdns' | 'ssdp' | 'arp';

export interface DiscoveredDevice {
  id: string;
  ip: string;
  mac: string | null;
  hostname: string | null;
  vendor: string | null;
  protocols: DiscoveryProtocol[];
  last_seen: IsoTimestamp;
  adopted: boolean;
}

export interface DiscoveryResponse {
  devices: DiscoveredDevice[];
}

export interface AdoptDeviceResponse {
  adopted: true;
}

/* -------------------------------------------------------------------------- */
/* Home Assistant                                                             */
/* -------------------------------------------------------------------------- */

export interface HomeAssistantStatus {
  configured: boolean;
  running: boolean;
  version: string | null;
  url: string | null;
  instance: InstanceSummary | null;
}

export interface HomeAssistantProvisionResponse {
  instance: InstanceSummary;
  url: string;
}

export interface HomeAssistantConfigRequest {
  url: string;
  token: string;
}

export interface HomeAssistantEntity {
  entity_id: string;
  state: string;
  name: string;
  domain: string;
  attributes: Record<string, unknown>;
  last_changed: IsoTimestamp | null;
}

export interface HomeAssistantEntitiesResponse {
  entities: HomeAssistantEntity[];
}

export interface HomeAssistantScene {
  entity_id: string;
  name: string;
}

/* -------------------------------------------------------------------------- */
/* Agent, tools and workflows                                                 */
/* -------------------------------------------------------------------------- */

export interface ProviderTestResult {
  ok: boolean;
  model: string | null;
  reply: string | null;
  error: string | null;
}

export interface ProviderRecord {
  id: Uuid;
  kind: LlmProviderKind;
  label: string;
  model: string;
  base_url: string | null;
  api_key_secret: string;
  enabled: boolean;
  created_at: IsoTimestamp;
}

export interface ProvidersResponse {
  providers: ProviderRecord[];
}

export interface CreateProviderRequest {
  kind: LlmProviderKind;
  label: string;
  model: string;
  base_url?: string | null;
  api_key?: string | null;
  enabled: boolean;
}

export interface ToolDefinition {
  name: string;
  description: string;
  parameters: unknown;
}

export interface ToolsResponse {
  /** Kept for older daemons; current `GET /agent/tools` returns a bare array. */
  tools: ToolDefinition[];
}

/** Normalize the bare-array tools response used by the daemon. */
export function normalizeTools(response: ToolDefinition[] | ToolsResponse): ToolDefinition[] {
  return Array.isArray(response) ? response : response.tools;
}

export type ChatRole = 'system' | 'user' | 'assistant' | 'tool';

export interface ToolCall {
  id: string;
  name: string;
  arguments: unknown;
}

export interface ChatMessage {
  role: ChatRole;
  content: string;
  tool_calls?: ToolCall[];
  tool_call_id?: string | null;
}

export interface AgentChatRequest {
  profile_id: Uuid;
  messages: ChatMessage[];
  use_tools: boolean;
}

export interface ToolTraceEntry {
  tool: string;
  arguments: unknown;
  allowed: boolean;
  output: string | null;
  error: string | null;
  duration_ms: number;
}

export interface TokenUsage {
  prompt_tokens: number;
  completion_tokens: number;
}

export interface AgentChatResponse {
  reply: string;
  model: string;
  tool_trace: ToolTraceEntry[];
  usage: TokenUsage;
}

export type WorkflowTrigger =
  | { type: 'manual' }
  | { type: 'webhook'; path: string }
  | { type: 'profile_activated' };

export type WorkflowNodeKind =
  | { type: 'trigger'; trigger: WorkflowTrigger }
  | { type: 'tool'; tool: string; arguments: unknown }
  | { type: 'condition'; field: string; equals: unknown }
  | { type: 'delay'; seconds: number };

export type WorkflowNode = { id: string } & WorkflowNodeKind;

export interface WorkflowEdge {
  from: string;
  to: string;
}

export interface Workflow {
  id: Uuid;
  name: string;
  description: string;
  enabled: boolean;
  nodes: WorkflowNode[];
  edges: WorkflowEdge[];
  created_at: IsoTimestamp;
  updated_at: IsoTimestamp;
}

export interface WorkflowsResponse {
  workflows: Workflow[];
}

export interface UpsertWorkflowRequest {
  name: string;
  description: string;
  enabled: boolean;
  nodes: WorkflowNode[];
  edges: WorkflowEdge[];
}

export interface WorkflowRunRequest {
  profile_id: Uuid;
  input: unknown;
}

export interface WorkflowRunResponse {
  workflow_id: Uuid;
  success: boolean;
  steps: unknown[];
  output: unknown;
}

/* -------------------------------------------------------------------------- */
/* Kiosk                                                                      */
/* -------------------------------------------------------------------------- */

export type AgentActivity = 'idle' | 'listening' | 'thinking' | 'speaking' | 'error';
export type NotificationLevel = 'info' | 'success' | 'warning' | 'error';

export interface MusicState {
  playing: boolean;
  track: string | null;
  artist: string | null;
  volume: number;
  sink: string | null;
}

export interface NetworkHealth {
  online: boolean;
  dns_ok: boolean;
  latency_ms: number | null;
}

export interface KioskScene {
  entity_id: string;
  name: string;
}

export interface KioskStateResponse {
  device_name: string;
  profile: Profile | null;
  music: MusicState;
  scenes: KioskScene[];
  network: NetworkHealth;
  agent_activity: AgentActivity;
  assistant_name: string;
  voice: {
    wake_word_enabled: boolean;
    activation_mode: 'wake_word' | 'push_to_talk' | 'both';
    listening: boolean;
    listening_window_secs: number;
    listener_connected: boolean;
  };
}

export type KioskEvent =
  | { type: 'avatar'; state: AvatarState; profile_id: Uuid | null }
  | { type: 'tracking'; point: TrackingPoint | null }
  | { type: 'profile_activated'; profile: Profile }
  | { type: 'wake'; assistant_name: string; trailing_query: string | null }
  | { type: 'speech'; text: string }
  | { type: 'music'; state: MusicState }
  | { type: 'agent'; activity: AgentActivity }
  | { type: 'network'; status: NetworkHealth }
  | { type: 'notification'; level: NotificationLevel; message: string };

/* -------------------------------------------------------------------------- */
/* Narrowing helpers                                                          */
/* -------------------------------------------------------------------------- */

export function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

export function getString(source: Record<string, unknown>, key: string): string | null {
  const value = source[key];
  return typeof value === 'string' ? value : null;
}

/** Best-effort label for an opaque peripheral descriptor. */
export function peripheralLabel(raw: unknown, fallback: string): string {
  if (typeof raw === 'string' && raw.trim() !== '') return raw;
  if (isRecord(raw)) {
    for (const key of ['name', 'description', 'path', 'id', 'device']) {
      const value = getString(raw, key);
      if (value !== null && value.trim() !== '') return value;
    }
  }
  return fallback;
}

/** Type guard for WebSocket payloads decoded from JSON. */
export function isKioskEvent(value: unknown): value is KioskEvent {
  if (!isRecord(value) || typeof value.type !== 'string') return false;
  return [
    'avatar',
    'tracking',
    'profile_activated',
    'wake',
    'music',
    'agent',
    'network',
    'notification',
  ].includes(value.type);
}

