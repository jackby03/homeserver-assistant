//! System configuration schema.
//!
//! [`SystemConfig`] is the single source of truth persisted as
//! `<data_dir>/config.json`. It never contains secrets: API keys, Home
//! Assistant tokens and tunnel credentials live in the encrypted secret store.
//!
//! The schema is versioned with [`CONFIG_SCHEMA_VERSION`]; migrations live in
//! [`crate::state::ConfigManager`].

use serde::{Deserialize, Serialize};

pub use crate::dns::DnsProvider;
use crate::error::{CoreError, CoreResult};
use crate::validation;

/// Current configuration schema version.
pub const CONFIG_SCHEMA_VERSION: u32 = 1;

/// Default admin portal port (TLS terminated by the local reverse proxy).
pub const DEFAULT_ADMIN_PORT: u16 = 8443;
/// Default kiosk port, bound to loopback only.
pub const DEFAULT_KIOSK_PORT: u16 = 3000;
/// Default DMZ bridge name.
pub const DEFAULT_DMZ_BRIDGE: &str = "br-dmz";
/// Default trusted LAN bridge name.
pub const DEFAULT_LAN_BRIDGE: &str = "br-lan";
/// Default DMZ subnet.
pub const DEFAULT_DMZ_SUBNET: &str = "10.20.30.0/24";
/// Default DMZ gateway.
pub const DEFAULT_DMZ_GATEWAY: &str = "10.20.30.1";

/// Top-level appliance configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SystemConfig {
    /// Schema version, used for forward-compatible migrations.
    pub schema_version: u32,
    /// Whether the first-boot onboarding wizard has been completed.
    pub initialized: bool,
    /// Device identity and localization.
    pub device: DeviceConfig,
    /// Network, DNS and DMZ settings.
    pub network: NetworkConfig,
    /// Audio routing to the home stereo.
    pub audio: AudioConfig,
    /// Administrative web portal settings.
    pub security: SecurityConfig,
    /// Default LLM provider used by the assistant.
    pub agent: AgentConfig,
    /// Always-on voice assistant and wake-word activation.
    pub voice: VoiceConfig,
    /// Wall tablet / kiosk runtime settings.
    pub kiosk: KioskConfig,
    /// Optional homeserver services.
    pub services: ServicesConfig,
}

impl Default for SystemConfig {
    fn default() -> Self {
        Self {
            schema_version: CONFIG_SCHEMA_VERSION,
            initialized: false,
            device: DeviceConfig::default(),
            network: NetworkConfig::default(),
            audio: AudioConfig::default(),
            security: SecurityConfig::default(),
            agent: AgentConfig::default(),
            voice: VoiceConfig::default(),
            kiosk: KioskConfig::generate(),
            services: ServicesConfig::default(),
        }
    }
}

impl SystemConfig {
    /// Create a fresh configuration with a generated kiosk token.
    #[must_use]
    pub fn bootstrap() -> Self {
        Self::default()
    }

    /// Validate the whole configuration.
    ///
    /// # Errors
    /// Returns [`CoreError::Validation`] for the first invalid field.
    pub fn validate(&self) -> CoreResult<()> {
        if self.schema_version != CONFIG_SCHEMA_VERSION {
            return Err(CoreError::Config(format!(
                "unsupported schema version {} (expected {CONFIG_SCHEMA_VERSION})",
                self.schema_version
            )));
        }
        validation::device_name(&self.device.name)?;
        validation::language(&self.device.language)?;
        validation::timezone(&self.device.timezone)?;
        validation::hostname(&self.device.hostname)?;
        self.network.validate()?;
        self.audio.validate()?;
        self.security.validate()?;
        self.agent.validate()?;
        self.voice.validate()?;
        self.kiosk.validate()?;
        self.services.validate()?;
        Ok(())
    }

    /// Register a freshly generated kiosk token (used by first boot).
    pub fn rotate_kiosk_token(&mut self) {
        self.kiosk.token = new_kiosk_token();
    }
}

/// Device identity and localization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DeviceConfig {
    /// Friendly name displayed in the portal and kiosk.
    pub name: String,
    /// UI language: `es` or `en`.
    pub language: String,
    /// IANA timezone.
    pub timezone: String,
    /// Hostname applied to the appliance.
    pub hostname: String,
}

impl Default for DeviceConfig {
    fn default() -> Self {
        Self {
            name: "HomeServer Assistant".to_owned(),
            language: "es".to_owned(),
            timezone: "UTC".to_owned(),
            hostname: "homeserver".to_owned(),
        }
    }
}

/// Network addressing mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkMode {
    /// Obtain the address with DHCP.
    Dhcp,
    /// Use the configured static address.
    Static,
}

/// Static IPv4 configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StaticIpv4 {
    /// Address in dotted notation.
    pub address: String,
    /// Prefix length (1..=32).
    pub prefix_len: u8,
    /// Default gateway.
    pub gateway: String,
}

/// LAN interface configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NetworkConfig {
    /// Physical interface, usually `eth0`.
    pub interface: String,
    /// Host bridge that carries the trusted LAN (Incus LAN workloads).
    pub lan_bridge: String,
    /// Addressing mode.
    pub mode: NetworkMode,
    /// Static address when `mode == static`.
    pub static_ipv4: Option<StaticIpv4>,
    /// DNS filtering and encrypted upstream configuration.
    pub dns: DnsConfig,
    /// Isolated DMZ settings.
    pub dmz: DmzConfig,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            interface: "eth0".to_owned(),
            lan_bridge: DEFAULT_LAN_BRIDGE.to_owned(),
            mode: NetworkMode::Dhcp,
            static_ipv4: None,
            dns: DnsConfig::default(),
            dmz: DmzConfig::default(),
        }
    }
}

impl NetworkConfig {
    /// Validate network settings.
    ///
    /// # Errors
    /// Returns [`CoreError::Validation`] when a field is invalid.
    pub fn validate(&self) -> CoreResult<()> {
        validation::hostname(&self.interface.to_lowercase())
            .map_err(|_| CoreError::validation("interface", "must be a valid interface name"))?;
        validation::hostname(&self.lan_bridge)
            .map_err(|_| CoreError::validation("lan_bridge", "must be a valid bridge name"))?;
        if self.lan_bridge == self.dmz.bridge {
            return Err(CoreError::validation(
                "lan_bridge",
                "LAN and DMZ bridges must be different interfaces",
            ));
        }
        if self.mode == NetworkMode::Static {
            let Some(static_ipv4) = &self.static_ipv4 else {
                return Err(CoreError::validation(
                    "static_ipv4",
                    "is required when network mode is static",
                ));
            };
            validation::ipv4(&static_ipv4.address)?;
            validation::ipv4(&static_ipv4.gateway)?;
            if static_ipv4.prefix_len == 0 || static_ipv4.prefix_len > 32 {
                return Err(CoreError::validation(
                    "prefix_len",
                    "must be between 1 and 32",
                ));
            }
        }
        self.dns.validate()?;
        self.dmz.validate()
    }
}

/// DNS filtering engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DnsEngine {
    /// AdGuard Home (default).
    #[serde(rename = "adguard_home")]
    AdGuardHome,
    /// Technitium DNS server.
    Technitium,
}

/// DNS transport used between the appliance and the upstream resolver.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DnsProtocol {
    /// DNS over HTTPS.
    Doh,
    /// DNS over TLS.
    Dot,
    /// Plain DNS on port 53. Only offered for legacy providers without an
    /// encrypted endpoint; the UI marks it as unencrypted.
    Plain,
}

impl DnsProtocol {
    /// Stable identifier used by the API and UI.
    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Self::Doh => "doh",
            Self::Dot => "dot",
            Self::Plain => "plain",
        }
    }

    /// Whether the transport is encrypted.
    #[must_use]
    pub fn is_encrypted(self) -> bool {
        matches!(self, Self::Doh | Self::Dot)
    }
}

/// DNS filtering configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DnsConfig {
    /// When false the DNS service is not provisioned.
    pub enabled: bool,
    /// Filtering engine to deploy.
    pub engine: DnsEngine,
    /// Upstream provider (validated catalog id).
    pub provider: DnsProvider,
    /// Transport used towards the upstream.
    pub protocol: DnsProtocol,
    /// Custom DoH URL / DoT host for `custom`, or the NextDNS configuration id.
    pub custom_upstream: Option<String>,
    /// Dynamic blocklist URLs (ads, trackers, malware).
    pub blocklists: Vec<String>,
}

impl Default for DnsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            engine: DnsEngine::AdGuardHome,
            provider: DnsProvider::cloudflare(),
            protocol: DnsProtocol::Doh,
            custom_upstream: None,
            blocklists: vec![
                "https://adguardteam.github.io/HostlistsRegistry/assets/filter_1.txt".to_owned(),
                "https://adguardteam.github.io/HostlistsRegistry/assets/filter_2.txt".to_owned(),
            ],
        }
    }
}

impl DnsConfig {
    /// Validate DNS settings.
    ///
    /// # Errors
    /// Returns [`CoreError::Validation`] when a field is invalid, the provider
    /// does not support the selected transport, or a required custom/NextDNS
    /// configuration id is missing.
    pub fn validate(&self) -> CoreResult<()> {
        if self.provider.spec().is_none() {
            return Err(CoreError::validation(
                "provider",
                format!("unknown DNS provider `{}`", self.provider),
            ));
        }
        if !self.provider.supports(self.protocol) {
            return Err(CoreError::validation(
                "protocol",
                format!(
                    "provider `{}` does not support the `{}` transport",
                    self.provider,
                    self.protocol.id()
                ),
            ));
        }

        if self.provider.is_custom() {
            let Some(upstream) = self.custom_upstream.as_deref() else {
                return Err(CoreError::validation(
                    "custom_upstream",
                    "is required when provider is custom",
                ));
            };
            match self.protocol {
                DnsProtocol::Doh => validation::http_url(upstream)?,
                DnsProtocol::Dot => {
                    if upstream.is_empty() || upstream.contains(char::is_whitespace) {
                        return Err(CoreError::validation(
                            "custom_upstream",
                            "must be a DoT hostname such as dns.example.com",
                        ));
                    }
                }
                DnsProtocol::Plain => {
                    validation::ipv4(upstream)?;
                }
            }
        } else if self.provider.requires_config_id() {
            let Some(id) = self.custom_upstream.as_deref() else {
                return Err(CoreError::validation(
                    "custom_upstream",
                    format!(
                        "provider `{}` requires your configuration id",
                        self.provider.label()
                    ),
                ));
            };
            let id = id.trim();
            if !(4..=32).contains(&id.len())
                || !id
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || character == '-')
            {
                return Err(CoreError::validation(
                    "custom_upstream",
                    "configuration id must be 4-32 letters, digits or hyphens",
                ));
            }
        }

        for url in &self.blocklists {
            validation::http_url(url)?;
        }
        Ok(())
    }
}

/// Isolated DMZ configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DmzConfig {
    /// Whether the DMZ exists.
    pub enabled: bool,
    /// Bridge interface connecting DMZ workloads.
    pub bridge: String,
    /// DMZ subnet in CIDR notation.
    pub subnet: String,
    /// DMZ gateway address (host side of the bridge).
    pub gateway: String,
    /// Allow DMZ workloads to reach the host DNS resolver.
    pub allow_dns: bool,
}

impl Default for DmzConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bridge: DEFAULT_DMZ_BRIDGE.to_owned(),
            subnet: DEFAULT_DMZ_SUBNET.to_owned(),
            gateway: DEFAULT_DMZ_GATEWAY.to_owned(),
            allow_dns: true,
        }
    }
}

impl DmzConfig {
    /// Validate DMZ settings.
    ///
    /// # Errors
    /// Returns [`CoreError::Validation`] when a field is invalid.
    pub fn validate(&self) -> CoreResult<()> {
        if !self.enabled {
            return Ok(());
        }
        validation::hostname(&self.bridge)?;
        validation::ipv4_cidr(&self.subnet)?;
        validation::ipv4(&self.gateway)?;
        if !self.subnet.contains(':') {
            // ensure gateway belongs to the subnet
            let (network, prefix) = self
                .subnet
                .split_once('/')
                .ok_or_else(|| CoreError::validation("subnet", "must use CIDR notation"))?;
            let prefix: u8 = prefix
                .parse()
                .map_err(|_| CoreError::validation("subnet", "invalid prefix"))?;
            let network = network
                .parse::<std::net::Ipv4Addr>()
                .map_err(|_| CoreError::validation("subnet", "invalid network address"))?;
            let gateway = self
                .gateway
                .parse::<std::net::Ipv4Addr>()
                .map_err(|_| CoreError::validation("gateway", "invalid IPv4 address"))?;
            let mask = if prefix == 0 {
                0u32
            } else {
                u32::MAX << (32 - prefix)
            };
            let network = u32::from(network) & mask;
            if u32::from(gateway) & mask != network {
                return Err(CoreError::validation(
                    "gateway",
                    "must belong to the DMZ subnet",
                ));
            }
        }
        Ok(())
    }
}

/// Audio routing configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioConfig {
    /// PipeWire node id selected for music playback.
    pub sink_id: Option<String>,
    /// Friendly sink name (display only).
    pub sink_name: Option<String>,
    /// Default volume (0..=100).
    pub volume: u8,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            sink_id: None,
            sink_name: None,
            volume: 60,
        }
    }
}

impl AudioConfig {
    /// Validate audio settings.
    ///
    /// # Errors
    /// Returns [`CoreError::Validation`] when the volume is out of range.
    pub fn validate(&self) -> CoreResult<()> {
        if self.volume > 100 {
            return Err(CoreError::validation("volume", "must be between 0 and 100"));
        }
        Ok(())
    }
}

/// Administrative portal configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SecurityConfig {
    /// Port the reverse proxy exposes for the admin portal.
    pub admin_web_port: u16,
    /// Loopback port that serves the wall kiosk.
    pub kiosk_port: u16,
    /// Session lifetime in hours.
    pub session_ttl_hours: u32,
    /// Extra origins allowed to call the API (development UI).
    pub allowed_origins: Vec<String>,
    /// Enable the DMZ isolation ruleset.
    pub enforce_dmz_isolation: bool,
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            admin_web_port: DEFAULT_ADMIN_PORT,
            kiosk_port: DEFAULT_KIOSK_PORT,
            session_ttl_hours: 12,
            allowed_origins: Vec::new(),
            enforce_dmz_isolation: true,
        }
    }
}

impl SecurityConfig {
    /// Validate security settings.
    ///
    /// # Errors
    /// Returns [`CoreError::Validation`] when a field is invalid.
    pub fn validate(&self) -> CoreResult<()> {
        validation::port(self.admin_web_port)?;
        validation::port(self.kiosk_port)?;
        if self.session_ttl_hours == 0 || self.session_ttl_hours > 24 * 30 {
            return Err(CoreError::validation(
                "session_ttl_hours",
                "must be between 1 and 720",
            ));
        }
        for origin in &self.allowed_origins {
            validation::http_url(origin)?;
        }
        Ok(())
    }
}

/// Supported LLM providers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LlmProviderKind {
    /// OpenAI Chat Completions.
    #[serde(rename = "openai")]
    OpenAi,
    /// Anthropic Messages API.
    Anthropic,
    /// DeepSeek (OpenAI-compatible).
    #[serde(rename = "deepseek")]
    DeepSeek,
    /// opencode-go gateway (OpenAI-compatible).
    OpencodeGo,
    /// Any OpenAI-compatible endpoint.
    Custom,
}

impl LlmProviderKind {
    /// Stable identifier used by the API and UI.
    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::Anthropic => "anthropic",
            Self::DeepSeek => "deepseek",
            Self::OpencodeGo => "opencode_go",
            Self::Custom => "custom",
        }
    }

    /// Whether the provider speaks the OpenAI wire protocol.
    #[must_use]
    pub fn is_openai_compatible(self) -> bool {
        matches!(
            self,
            Self::OpenAi | Self::DeepSeek | Self::OpencodeGo | Self::Custom
        )
    }
}

/// Default assistant provider configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentConfig {
    /// Active provider, when configured.
    pub provider: Option<LlmProviderKind>,
    /// Model identifier, e.g. `gpt-4.1-mini`.
    pub model: String,
    /// Optional base URL override (required for `custom`).
    pub base_url: Option<String>,
    /// Maximum number of tool-calling steps per user turn.
    pub max_tool_steps: u8,
    /// Sampling temperature (0.0..=2.0).
    pub temperature: f32,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            provider: None,
            model: String::new(),
            base_url: None,
            max_tool_steps: 6,
            temperature: 0.7,
        }
    }
}

impl AgentConfig {
    /// Validate agent settings.
    ///
    /// # Errors
    /// Returns [`CoreError::Validation`] when a field is invalid.
    pub fn validate(&self) -> CoreResult<()> {
        if let Some(provider) = self.provider {
            if self.model.trim().is_empty() {
                return Err(CoreError::validation(
                    "model",
                    "is required when an LLM provider is configured",
                ));
            }
            match (provider, &self.base_url) {
                (LlmProviderKind::Custom, None) => {
                    return Err(CoreError::validation(
                        "base_url",
                        "is required for a custom provider",
                    ));
                }
                (_, Some(url)) => validation::http_url(url)?,
                (_, None) => {}
            }
        }
        if self.max_tool_steps == 0 || self.max_tool_steps > 20 {
            return Err(CoreError::validation(
                "max_tool_steps",
                "must be between 1 and 20",
            ));
        }
        if !(0.0..=2.0).contains(&self.temperature) {
            return Err(CoreError::validation(
                "temperature",
                "must be between 0.0 and 2.0",
            ));
        }
        Ok(())
    }
}

/// How the household can start a conversation with the assistant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VoiceActivationMode {
    /// Always-on listener: call the assistant by name to start a request.
    WakeWord,
    /// Only the kiosk "talk" button starts a request (no microphone hotword).
    PushToTalk,
    /// Both wake word and push-to-talk are available.
    Both,
}

impl VoiceActivationMode {
    /// Stable identifier used by the API and the UI.
    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Self::WakeWord => "wake_word",
            Self::PushToTalk => "push_to_talk",
            Self::Both => "both",
        }
    }
}

/// Always-on voice assistant configuration.
///
/// The name is the wake word: the household says `"Oye Aura, ..."` and the
/// daemon matches the transcript locally. Audio never leaves the appliance; an
/// optional local STT helper produces the transcript.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VoiceConfig {
    /// Name the household calls the assistant by (`Aura`, `Casa`, `Jarvis`).
    pub assistant_name: String,
    /// Whether the always-on wake-word listener is active.
    pub wake_word_enabled: bool,
    /// Wake-word, push-to-talk or both.
    pub activation_mode: VoiceActivationMode,
    /// Wake-word sensitivity in `0.0..=1.0` (higher tolerates more STT typos).
    pub sensitivity: f32,
    /// Seconds the assistant keeps listening for the request after being called.
    pub listening_window_secs: u8,
    /// Language of the local speech recognition pipeline (`es` or `en`).
    pub language: String,
}

impl Default for VoiceConfig {
    fn default() -> Self {
        Self {
            assistant_name: "Aura".to_owned(),
            wake_word_enabled: true,
            activation_mode: VoiceActivationMode::Both,
            sensitivity: 0.6,
            listening_window_secs: 8,
            language: "es".to_owned(),
        }
    }
}

impl VoiceConfig {
    /// Validate voice-assistant settings.
    ///
    /// # Errors
    /// Returns [`CoreError::Validation`] when a field is invalid.
    pub fn validate(&self) -> CoreResult<()> {
        validation::assistant_name(&self.assistant_name)?;
        validation::language(&self.language)?;
        if !self.sensitivity.is_finite() || !(0.0..=1.0).contains(&self.sensitivity) {
            return Err(CoreError::validation(
                "voice.sensitivity",
                "must be between 0.0 and 1.0",
            ));
        }
        if !(3..=60).contains(&self.listening_window_secs) {
            return Err(CoreError::validation(
                "voice.listening_window_secs",
                "must be between 3 and 60 seconds",
            ));
        }
        Ok(())
    }

    /// Phrase the household should say to wake the assistant, e.g. `"Oye Aura"`.
    #[must_use]
    pub fn wake_phrase(&self) -> String {
        format!("Oye {}", self.assistant_name.trim())
    }
}

/// Wall tablet / kiosk configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct KioskConfig {
    /// Shared secret used by the local kiosk UI to call the kiosk API.
    pub token: String,
    /// Seconds of inactivity before dimming.
    pub idle_dim_seconds: u32,
    /// Profile activated when nobody is recognized.
    pub default_profile: Option<String>,
}

impl Default for KioskConfig {
    fn default() -> Self {
        Self {
            token: String::new(),
            idle_dim_seconds: 120,
            default_profile: None,
        }
    }
}

impl KioskConfig {
    /// Create a kiosk configuration with a fresh 256-bit token.
    #[must_use]
    pub fn generate() -> Self {
        Self {
            token: new_kiosk_token(),
            ..Self::default()
        }
    }

    /// Validate kiosk settings.
    ///
    /// # Errors
    /// Returns [`CoreError::Validation`] when the token is too weak.
    pub fn validate(&self) -> CoreResult<()> {
        if self.token.len() < 32 {
            return Err(CoreError::validation(
                "kiosk.token",
                "must be at least 32 characters",
            ));
        }
        if self.idle_dim_seconds > 86_400 {
            return Err(CoreError::validation(
                "idle_dim_seconds",
                "must be at most 86400",
            ));
        }
        Ok(())
    }
}

/// Public exposure strategy for apps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TunnelProvider {
    /// Do not publish anything outside the LAN.
    None,
    /// Cloudflare Tunnel (outbound only).
    CloudflareTunnel,
    /// WireGuard mesh (outbound only).
    #[serde(rename = "wireguard")]
    WireGuard,
}

/// Optional homeserver services.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ServicesConfig {
    /// Provision Home Assistant.
    pub enable_home_assistant: bool,
    /// Home Assistant base URL once provisioned.
    pub home_assistant_url: Option<String>,
    /// Enable the Caddy reverse proxy.
    pub enable_reverse_proxy: bool,
    /// Base domain used for app subdomains.
    pub base_domain: String,
    /// Enable outbound tunnel publishing.
    pub enable_tunnel: bool,
    /// Tunnel implementation.
    pub tunnel_provider: TunnelProvider,
    /// Provision the music assistant service.
    pub enable_music: bool,
}

impl Default for ServicesConfig {
    fn default() -> Self {
        Self {
            enable_home_assistant: true,
            home_assistant_url: None,
            enable_reverse_proxy: true,
            base_domain: "home.local".to_owned(),
            enable_tunnel: false,
            tunnel_provider: TunnelProvider::None,
            enable_music: true,
        }
    }
}

impl ServicesConfig {
    /// Validate service settings.
    ///
    /// # Errors
    /// Returns [`CoreError::Validation`] when a field is invalid.
    pub fn validate(&self) -> CoreResult<()> {
        if let Some(url) = &self.home_assistant_url {
            validation::http_url(url)?;
        }
        validation::domain(&self.base_domain)?;
        if self.enable_tunnel && self.tunnel_provider == TunnelProvider::None {
            return Err(CoreError::validation(
                "tunnel_provider",
                "must be set when tunnel publishing is enabled",
            ));
        }
        Ok(())
    }
}

fn new_kiosk_token() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_config() -> SystemConfig {
        let mut config = SystemConfig::bootstrap();
        config.device.timezone = "America/Lima".to_owned();
        config
    }

    #[test]
    fn default_config_is_valid() {
        valid_config()
            .validate()
            .expect("default config must be valid");
    }

    #[test]
    fn rejects_bad_language() {
        let mut config = valid_config();
        config.device.language = "fr".to_owned();
        assert!(config.validate().is_err());
    }

    #[test]
    fn static_mode_requires_address() {
        let mut config = valid_config();
        config.network.mode = NetworkMode::Static;
        assert!(config.validate().is_err());
        config.network.static_ipv4 = Some(StaticIpv4 {
            address: "192.168.1.10".to_owned(),
            prefix_len: 24,
            gateway: "192.168.1.1".to_owned(),
        });
        config.validate().expect("static config should be valid");
    }

    #[test]
    fn dmz_gateway_must_be_in_subnet() {
        let mut config = valid_config();
        config.network.dmz.enabled = true;
        config.network.dmz.gateway = "10.99.0.1".to_owned();
        assert!(config.validate().is_err());
        config.network.dmz.gateway = DEFAULT_DMZ_GATEWAY.to_owned();
        config.validate().expect("dmz should be valid");
    }

    #[test]
    fn custom_dns_requires_upstream() {
        let mut config = valid_config();
        config.network.dns.provider = DnsProvider::custom();
        assert!(config.validate().is_err());
        config.network.dns.custom_upstream = Some("https://dns.example.com/dns-query".to_owned());
        config.validate().expect("custom dns should be valid");
    }

    #[test]
    fn kiosk_tokens_are_unique_and_long() {
        let a = KioskConfig::generate();
        let b = KioskConfig::generate();
        assert_ne!(a.token, b.token);
        assert_eq!(a.token.len(), 64);
    }

    #[test]
    fn voice_defaults_are_valid_and_customizable() {
        let mut config = valid_config();
        assert_eq!(config.voice.assistant_name, "Aura");
        assert_eq!(config.voice.wake_phrase(), "Oye Aura");
        config.voice.assistant_name = "Jarvis".to_owned();
        config.voice.sensitivity = 0.9;
        config.voice.activation_mode = VoiceActivationMode::WakeWord;
        config.validate().expect("custom voice config is valid");
        assert_eq!(config.voice.wake_phrase(), "Oye Jarvis");
    }

    #[test]
    fn rejects_invalid_voice_settings() {
        let mut config = valid_config();
        config.voice.assistant_name = "x".to_owned();
        assert!(config.validate().is_err());
        config.voice.assistant_name = "Aura".to_owned();
        config.voice.sensitivity = 1.5;
        assert!(config.validate().is_err());
        config.voice.sensitivity = 0.5;
        config.voice.listening_window_secs = 2;
        assert!(config.validate().is_err());
        config.voice.listening_window_secs = 8;
        config.voice.language = "fr".to_owned();
        assert!(config.validate().is_err());
    }

    #[test]
    fn config_roundtrips_json() {
        let mut config = valid_config();
        config.network.dmz.enabled = true;
        config.agent.provider = Some(LlmProviderKind::DeepSeek);
        config.agent.model = "deepseek-chat".to_owned();
        config.validate().unwrap();
        let json = serde_json::to_string(&config).unwrap();
        let parsed: SystemConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, config);
    }
}
