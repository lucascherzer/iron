//! Firewall with device ownership claims
//!
//! Implements a whitelist-based firewall that allows users to trust person identities
//! as well as individual devices. This provides a two-tier key system:
//! - **Person keys**: Long-term identity keys representing humans
//! - **Device keys**: Ephemeral keys (iroh EndpointId) for specific devices
//!
//! # Overview
//!
//! Instead of whitelisting each device individually, users can trust a person's
//! identity key. When that person adds new devices, they sign an ownership claim
//! that proves the device belongs to them. The firewall verifies these claims
//! and caches verified devices for performance.
//!
//! # Security Model
//!
//! - Person keys are long-lived Ed25519 keypairs
//! - Device ownership claims are signed by person keys
//! - Claims have expiration dates (default: 1 year)
//! - Verification results are cached persistently
//! - Manual revocation supported by removing person from whitelist

use anyhow::{Context, Result};
use iroh::EndpointId;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

/// A person's long-term identity key
///
/// This is separate from device keys and represents a human identity.
/// Person keys are Ed25519 keypairs used to sign ownership claims for devices.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PersonKey {
    /// Ed25519 public key (32 bytes)
    /// Users exchange and whitelist these keys
    #[serde(with = "serde_bytes")]
    public_key: [u8; 32],
}

impl PersonKey {
    /// Create a new PersonKey from raw bytes
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self { public_key: bytes }
    }

    /// Get the raw bytes of the public key
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.public_key
    }

    /// Verify a signature on a message
    pub fn verify(&self, message: &[u8], signature: &[u8; 64]) -> bool {
        use ed25519_dalek::{Signature, Verifier, VerifyingKey};

        let Ok(verifying_key) = VerifyingKey::from_bytes(&self.public_key) else {
            return false;
        };

        let sig = Signature::from_bytes(signature);
        verifying_key.verify(message, &sig).is_ok()
    }

    /// Convert to hex string for display
    pub fn to_hex(&self) -> String {
        hex::encode(self.public_key)
    }

    /// Parse from hex string
    pub fn from_hex(s: &str) -> Result<Self> {
        let bytes = hex::decode(s).context("Invalid hex string")?;
        if bytes.len() != 32 {
            anyhow::bail!(
                "Invalid person key length: expected 32 bytes, got {}",
                bytes.len()
            );
        }
        let mut array = [0u8; 32];
        array.copy_from_slice(&bytes);
        Ok(Self::from_bytes(array))
    }
}

/// A person's private key for signing ownership claims
#[derive(Clone)]
pub struct PersonSecretKey {
    /// Ed25519 secret key (32 bytes seed)
    secret_key: [u8; 32],
}

impl PersonSecretKey {
    /// Get the default path for storing person secret key
    pub fn default_path() -> Result<PathBuf> {
        let home = std::env::var("HOME").context("HOME environment variable not set")?;
        Ok(PathBuf::from(home).join(".config/iron/person_key.secret"))
    }

    /// Load person secret key from default location
    pub fn load_from_default_path() -> Result<Self> {
        let path = Self::default_path()?;
        let hex = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read person key from {}", path.display()))?;
        Self::from_hex(hex.trim())
    }

    /// Check if person secret key exists at default location
    pub fn exists_at_default_path() -> bool {
        Self::default_path().map(|p| p.exists()).unwrap_or(false)
    }
}

impl PersonSecretKey {
    /// Generate a new random person key
    pub fn generate() -> Self {
        use rand::RngCore;
        let mut secret_key = [0u8; 32];
        rand::rng().fill_bytes(&mut secret_key);
        Self { secret_key }
    }

    /// Create from raw bytes
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self { secret_key: bytes }
    }

    /// Get the public key
    pub fn public_key(&self) -> PersonKey {
        use ed25519_dalek::SigningKey;
        let signing_key = SigningKey::from_bytes(&self.secret_key);
        PersonKey::from_bytes(signing_key.verifying_key().to_bytes())
    }

    /// Sign a message
    pub fn sign(&self, message: &[u8]) -> [u8; 64] {
        use ed25519_dalek::{Signer, SigningKey};
        let signing_key = SigningKey::from_bytes(&self.secret_key);
        signing_key.sign(message).to_bytes()
    }

    /// Convert to hex string
    pub fn to_hex(&self) -> String {
        hex::encode(self.secret_key)
    }

    /// Parse from hex string
    pub fn from_hex(s: &str) -> Result<Self> {
        let bytes = hex::decode(s).context("Invalid hex string")?;
        if bytes.len() != 32 {
            anyhow::bail!(
                "Invalid secret key length: expected 32 bytes, got {}",
                bytes.len()
            );
        }
        let mut array = [0u8; 32];
        array.copy_from_slice(&bytes);
        Ok(Self::from_bytes(array))
    }
}

/// A device's identity (what we currently call EndpointId)
///
/// This is the iroh EndpointId that identifies a specific device.
/// These can be rotated or changed without affecting the person's identity.
pub type DeviceKey = EndpointId;

/// Proof that a device is owned by a person
///
/// This is generated by the person and carried by the device.
/// The signature proves the person key holder authorized this device.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnershipClaim {
    /// The person who owns this device
    pub person_key: PersonKey,

    /// The device being claimed
    pub device_key: DeviceKey,

    /// Signature: sign(person_key || device_key, person_private_key)
    /// This proves the person key holder authorized this device
    #[serde(with = "serde_bytes")]
    pub signature: [u8; 64],

    /// Timestamp when claim was created (Unix timestamp in seconds)
    pub created_at: u64,

    /// Expiry timestamp (Unix timestamp in seconds)
    pub expires_at: u64,
}

impl OwnershipClaim {
    /// Create a new ownership claim
    pub fn new(
        person_secret: &PersonSecretKey,
        device_key: DeviceKey,
        validity_seconds: u64,
    ) -> Self {
        let person_key = person_secret.public_key();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // Create the message to sign: "iron:ownership:" + device_key
        let message = format!("iron:ownership:{}", device_key);
        let signature = person_secret.sign(message.as_bytes());

        Self {
            person_key,
            device_key,
            signature,
            created_at: now,
            expires_at: now + validity_seconds,
        }
    }

    /// Verify the signature on this claim
    pub fn verify_signature(&self) -> bool {
        let message = format!("iron:ownership:{}", self.device_key);
        self.person_key.verify(message.as_bytes(), &self.signature)
    }

    /// Check if the claim has expired
    pub fn is_expired(&self) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        now > self.expires_at
    }

    /// Verify the claim (signature + expiry)
    pub fn verify(&self) -> Result<()> {
        if self.is_expired() {
            anyhow::bail!("Ownership claim expired");
        }

        if !self.verify_signature() {
            anyhow::bail!("Invalid signature on ownership claim");
        }

        Ok(())
    }
}

/// Firewall action (currently only Accept for whitelist-based approach)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum FirewallAction {
    /// Accept packets from this source
    Accept,
}

impl std::fmt::Display for FirewallAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FirewallAction::Accept => write!(f, "accept"),
        }
    }
}

/// Source of a packet for policy matching
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PacketSource {
    /// Match any peer (wildcard)
    Any,

    /// Match by person key (by name from trusted_persons)
    Person(String), // references TrustedPerson.name

    /// Match by specific device EndpointId
    Peer(EndpointId),
}

impl PacketSource {
    /// Parse a source string into a PacketSource
    /// Formats:
    /// - "*" -> Any
    /// - "person:alice" -> Person("alice")
    /// - "peer:<endpoint_id>" -> Peer(endpoint_id)
    /// - "<endpoint_id>" -> Peer(endpoint_id) (backward compat)
    pub fn parse(s: &str) -> Result<Self> {
        if s == "*" {
            return Ok(Self::Any);
        }

        if let Some(name) = s.strip_prefix("person:") {
            return Ok(Self::Person(name.to_string()));
        }

        if let Some(id_str) = s.strip_prefix("peer:") {
            let endpoint_id =
                EndpointId::from_str(id_str).context("Invalid endpoint ID in peer source")?;
            return Ok(Self::Peer(endpoint_id));
        }

        // Try parsing as raw endpoint ID (backward compatibility)
        match EndpointId::from_str(s) {
            Ok(endpoint_id) => Ok(Self::Peer(endpoint_id)),
            Err(_) => anyhow::bail!(
                "Invalid source format. Use '*', 'person:<name>', or 'peer:<endpoint_id>'"
            ),
        }
    }

    /// Check if this source matches a given device
    pub fn matches(
        &self,
        device: &DeviceKey,
        person: Option<&PersonKey>,
        config: &FirewallConfig,
    ) -> bool {
        match self {
            Self::Any => true,
            Self::Peer(id) => id == device,
            Self::Person(name) => {
                // Check if the device is owned by this person
                if let Some(person_key) = person {
                    config
                        .trusted_persons
                        .iter()
                        .any(|p| &p.name == name && &p.key == person_key)
                } else {
                    false
                }
            }
        }
    }
}
/// Port range for firewall rules
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PortRange {
    /// Match any port (wildcard)
    Any,

    /// Match a specific port
    Single(u16),

    /// Match ports from min to max (inclusive)
    Range { min: u16, max: u16 },

    /// Match ports from min to 65535
    From(u16),
}

impl PortRange {
    /// Parse a port range string
    /// Formats:
    /// - "*" -> Any
    /// - "80" -> Single(80)
    /// - "1000-2000" -> Range{min: 1000, max: 2000}
    /// - "1000-" -> From(1000)
    pub fn parse(s: &str) -> Result<Self> {
        if s == "*" {
            return Ok(Self::Any);
        }

        // Check for range formats
        if let Some(dash_pos) = s.find('-') {
            let start_str = &s[..dash_pos];
            let end_str = &s[dash_pos + 1..];

            if end_str.is_empty() {
                // Format: "1000-"
                let min = start_str
                    .parse::<u16>()
                    .context("Invalid port number in range")?;
                return Ok(Self::From(min));
            } else {
                // Format: "1000-2000"
                let min = start_str
                    .parse::<u16>()
                    .context("Invalid start port in range")?;
                let max = end_str
                    .parse::<u16>()
                    .context("Invalid end port in range")?;

                if min > max {
                    anyhow::bail!(
                        "Invalid port range: start port {} is greater than end port {}",
                        min,
                        max
                    );
                }

                return Ok(Self::Range { min, max });
            }
        }

        // Single port
        let port = s.parse::<u16>().context("Invalid port number")?;
        Ok(Self::Single(port))
    }

    /// Check if a port matches this range
    pub fn matches(&self, port: u16) -> bool {
        match self {
            Self::Any => true,
            Self::Single(p) => port == *p,
            Self::Range { min, max } => port >= *min && port <= *max,
            Self::From(min) => port >= *min,
        }
    }
}

impl std::fmt::Display for PortRange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Any => write!(f, "*"),
            Self::Single(p) => write!(f, "{}", p),
            Self::Range { min, max } => write!(f, "{}-{}", min, max),
            Self::From(min) => write!(f, "{}-", min),
        }
    }
}

/// A trusted person entry in the firewall configuration
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TrustedPerson {
    /// User-provided name (used as identifier in policies)
    pub name: String,

    /// Optional user comment
    pub comment: Option<String>,

    /// Ed25519 public key
    pub key: PersonKey,
}

/// Firewall policy (whitelist rule)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FirewallPolicy {
    /// Action to take (currently only Accept)
    pub action: FirewallAction,

    /// Source to match
    #[serde(rename = "src")]
    pub source: PacketSource,

    /// Destination port to match (optional, defaults to any)
    #[serde(rename = "dstPort", skip_serializing_if = "Option::is_none", default)]
    pub dst_port: Option<PortRange>,
}

impl FirewallPolicy {
    /// Create a new policy accepting from a source on any port
    pub fn accept_from(source: PacketSource) -> Self {
        Self {
            action: FirewallAction::Accept,
            source,
            dst_port: None,
        }
    }

    /// Create a new policy accepting from a source on specific ports
    pub fn accept_from_with_port(source: PacketSource, dst_port: PortRange) -> Self {
        Self {
            action: FirewallAction::Accept,
            source,
            dst_port: Some(dst_port),
        }
    }

    /// Check if this policy matches a given packet
    pub fn matches(
        &self,
        device: &DeviceKey,
        person: Option<&PersonKey>,
        dst_port: u16,
        config: &FirewallConfig,
    ) -> bool {
        // Check source match
        if !self.source.matches(device, person, config) {
            return false;
        }

        // Check port match
        if let Some(port_range) = &self.dst_port
            && !port_range.matches(dst_port)
        {
            return false;
        }

        // All conditions match
        true
    }
}

/// Firewall configuration for a node
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct FirewallConfig {
    /// Whether the firewall is enabled
    pub enabled: bool,

    /// List of trusted persons
    pub trusted_persons: Vec<TrustedPerson>,

    /// Firewall policies (whitelist rules)
    pub policies: Vec<FirewallPolicy>,

    /// Cache of verified devices (device_key -> person_key)
    /// Persisted to disk for performance
    #[serde(skip)]
    pub verified_devices: HashMap<DeviceKey, PersonKey>,
}

impl FirewallConfig {
    /// Create a new firewall configuration
    pub fn new() -> Self {
        Self::default()
    }

    /// Verify an ownership claim and check if the person is trusted
    pub fn verify_claim(&mut self, claim: &OwnershipClaim) -> Result<bool> {
        // 1. Verify signature and expiry
        claim.verify()?;

        // 2. Check if person is trusted (look up by key in trusted_persons)
        let is_trusted = self
            .trusted_persons
            .iter()
            .any(|p| p.key == claim.person_key);

        if !is_trusted {
            return Ok(false); // Valid claim, but person not trusted
        }

        // 3. Cache the verified device
        self.verified_devices
            .insert(claim.device_key, claim.person_key.clone());

        Ok(true)
    }

    /// Check if a device is allowed to communicate
    pub fn is_device_allowed(&self, device_key: &DeviceKey) -> bool {
        if !self.enabled {
            return true; // Firewall disabled, allow all
        }

        self.verified_devices.contains_key(device_key)
    }

    /// Check if a packet is allowed based on policies
    /// Returns true if any policy matches, or if no policies are configured (backward compat)
    pub fn is_packet_allowed(&self, device_key: &DeviceKey, dst_port: u16) -> bool {
        if !self.enabled {
            return true; // Firewall disabled, allow all
        }

        // If no policies configured, fall back to device verification only
        if self.policies.is_empty() {
            return self.is_device_allowed(device_key);
        }

        // Get the person key for this device (if verified)
        let person_key = self.verified_devices.get(device_key);

        // Check if any policy matches
        for policy in &self.policies {
            if policy.matches(device_key, person_key, dst_port, self) {
                return true;
            }
        }

        false
    }

    /// Add a trusted person
    pub fn add_person(&mut self, person: TrustedPerson) {
        self.trusted_persons.push(person);
    }

    /// Remove a trusted person by name
    pub fn remove_person(&mut self, name: &str) -> bool {
        if let Some(pos) = self.trusted_persons.iter().position(|p| p.name == name) {
            let person = self.trusted_persons.remove(pos);
            // Also remove all verified devices owned by this person
            self.verified_devices.retain(|_, pk| pk != &person.key);
            true
        } else {
            false
        }
    }

    /// Get a person by name
    pub fn get_person(&self, name: &str) -> Option<&TrustedPerson> {
        self.trusted_persons.iter().find(|p| p.name == name)
    }

    /// Get the path to the firewall config file
    #[allow(dead_code)]
    fn config_path() -> Result<PathBuf> {
        Self::config_path_with_home(None)
    }

    /// Get the path to the firewall config file with optional home override (for testing)
    fn config_path_with_home(home_override: Option<&str>) -> Result<PathBuf> {
        let home = if let Some(home) = home_override {
            home.to_string()
        } else {
            std::env::var("HOME").context("HOME environment variable not set")?
        };
        Ok(PathBuf::from(home).join(".config/iron/firewall.json"))
    }

    /// Get the path to the verified devices cache
    #[allow(dead_code)]
    fn cache_path() -> Result<PathBuf> {
        Self::cache_path_with_home(None)
    }

    /// Get the path to the verified devices cache with optional home override (for testing)
    fn cache_path_with_home(home_override: Option<&str>) -> Result<PathBuf> {
        let home = if let Some(home) = home_override {
            home.to_string()
        } else {
            std::env::var("HOME").context("HOME environment variable not set")?
        };
        Ok(PathBuf::from(home).join(".config/iron/firewall_cache.json"))
    }

    /// Get the path to the claims directory
    fn claims_dir() -> Result<PathBuf> {
        let home = std::env::var("HOME").context("HOME environment variable not set")?;
        Ok(PathBuf::from(home).join(".config/iron/claims"))
    }

    /// Get the path to a claim file for a specific person key and device key
    pub fn claim_path(person_key: &PersonKey, device_key: &DeviceKey) -> Result<PathBuf> {
        let claims_dir = Self::claims_dir()?;
        let filename = format!("{}-{}.json", person_key.to_hex(), device_key);
        Ok(claims_dir.join(filename))
    }

    /// Save a claim to the standard claims directory
    pub fn save_claim(claim: &OwnershipClaim) -> Result<()> {
        let claims_dir = Self::claims_dir()?;

        // Create directory if it doesn't exist
        fs::create_dir_all(&claims_dir).context("Failed to create claims directory")?;

        // Set directory permissions to 0700 (owner only)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&claims_dir)?.permissions();
            perms.set_mode(0o700);
            fs::set_permissions(&claims_dir, perms)?;
        }

        let claim_path = Self::claim_path(&claim.person_key, &claim.device_key)?;
        let json = serde_json::to_string_pretty(claim).context("Failed to serialize claim")?;

        fs::write(&claim_path, json)
            .with_context(|| format!("Failed to write claim to {}", claim_path.display()))?;

        // Set file permissions to 0600 (owner read/write only)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&claim_path)?.permissions();
            perms.set_mode(0o600);
            fs::set_permissions(&claim_path, perms)?;
        }

        Ok(())
    }

    /// Load a claim for a specific device (if it exists)
    pub fn load_claim(device_key: &DeviceKey) -> Result<Option<OwnershipClaim>> {
        let claims_dir = Self::claims_dir()?;

        if !claims_dir.exists() {
            return Ok(None);
        }

        // Look for any claim file matching this device key
        let entries = fs::read_dir(&claims_dir).context("Failed to read claims directory")?;

        for entry in entries {
            let entry = entry?;
            let path = entry.path();

            if !path.is_file() {
                continue;
            }

            // Check if filename ends with this device key
            if let Some(filename) = path.file_name().and_then(|n| n.to_str())
                && filename.ends_with(&format!("{}.json", device_key))
            {
                // Found a matching claim file
                let json = fs::read_to_string(&path)
                    .with_context(|| format!("Failed to read claim file: {}", path.display()))?;

                let claim: OwnershipClaim = serde_json::from_str(&json)
                    .with_context(|| format!("Failed to parse claim file: {}", path.display()))?;

                return Ok(Some(claim));
            }
        }

        Ok(None)
    }

    /// Load firewall configuration from disk
    pub fn load() -> Result<Self> {
        Self::load_from_home(None)
    }

    /// Load firewall configuration with optional home override (for testing)
    pub fn load_from_home(home_override: Option<&str>) -> Result<Self> {
        let config_path = Self::config_path_with_home(home_override)?;

        // Load main config if it exists, otherwise use default
        let mut config: Self = if config_path.exists() {
            let json =
                fs::read_to_string(&config_path).context("Failed to read firewall config")?;
            serde_json::from_str(&json).context("Failed to parse firewall config")?
        } else {
            Self::default()
        };

        // Always try to load verified devices cache (even if main config doesn't exist)
        // This allows cache to persist independently
        let cache_path = Self::cache_path_with_home(home_override)?;
        if cache_path.exists() {
            let cache_json =
                fs::read_to_string(&cache_path).context("Failed to read firewall cache")?;
            config.verified_devices =
                serde_json::from_str(&cache_json).context("Failed to parse firewall cache")?;
        }

        Ok(config)
    }

    /// Save firewall configuration to disk
    pub fn save(&self) -> Result<()> {
        Self::save_with_home(self, None)
    }

    /// Save firewall configuration with optional home override (for testing)
    pub fn save_with_home(&self, home_override: Option<&str>) -> Result<()> {
        let config_path = Self::config_path_with_home(home_override)?;

        // Create directory if it doesn't exist
        if let Some(dir) = config_path.parent() {
            fs::create_dir_all(dir).context("Failed to create config directory")?;

            // Set directory permissions to 0700 (owner only)
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = fs::metadata(dir)?.permissions();
                perms.set_mode(0o700);
                fs::set_permissions(dir, perms)?;
            }
        }

        // Save main config (without verified_devices cache)
        let json =
            serde_json::to_string_pretty(self).context("Failed to serialize firewall config")?;
        fs::write(&config_path, json).context("Failed to write firewall config")?;

        // Set file permissions to 0600 (owner read/write only)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&config_path)?.permissions();
            perms.set_mode(0o600);
            fs::set_permissions(&config_path, perms)?;
        }

        // Save verified devices cache separately
        self.save_cache_with_home(home_override)?;

        Ok(())
    }

    /// Save only the verified devices cache
    pub fn save_cache(&self) -> Result<()> {
        self.save_cache_with_home(None)
    }

    /// Save only the verified devices cache with optional home override (for testing)
    fn save_cache_with_home(&self, home_override: Option<&str>) -> Result<()> {
        let cache_path = Self::cache_path_with_home(home_override)?;

        // Create directory if it doesn't exist (usually already exists)
        if let Some(dir) = cache_path.parent() {
            fs::create_dir_all(dir).context("Failed to create config directory")?;
        }

        let cache_json = serde_json::to_string_pretty(&self.verified_devices)
            .context("Failed to serialize firewall cache")?;
        fs::write(&cache_path, cache_json).context("Failed to write firewall cache")?;

        // Set file permissions to 0600 (owner read/write only)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&cache_path)?.permissions();
            perms.set_mode(0o600);
            fs::set_permissions(&cache_path, perms)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_person_key_from_bytes() {
        let bytes = [42u8; 32];
        let key = PersonKey::from_bytes(bytes);
        assert_eq!(key.as_bytes(), &bytes);
    }

    #[test]
    fn test_person_key_hex() {
        let bytes = [42u8; 32];
        let key = PersonKey::from_bytes(bytes);
        let hex = key.to_hex();
        let parsed = PersonKey::from_hex(&hex).unwrap();
        assert_eq!(key, parsed);
    }

    #[test]
    fn test_person_secret_key_generation() {
        let key1 = PersonSecretKey::generate();
        let key2 = PersonSecretKey::generate();
        // Different keys should be generated
        assert_ne!(key1.to_hex(), key2.to_hex());
    }

    #[test]
    fn test_sign_and_verify() {
        let secret = PersonSecretKey::generate();
        let public = secret.public_key();
        let message = b"test message";
        let signature = secret.sign(message);

        assert!(public.verify(message, &signature));
        assert!(!public.verify(b"different message", &signature));
    }

    #[test]
    fn test_ownership_claim_creation() {
        let person_secret = PersonSecretKey::generate();
        let device_key = iroh::SecretKey::generate(&mut rand::rng()).public();

        let claim = OwnershipClaim::new(&person_secret, device_key, 3600);

        assert_eq!(claim.device_key, device_key);
        assert_eq!(claim.person_key, person_secret.public_key());
        assert!(claim.verify_signature());
        assert!(!claim.is_expired());
    }

    #[test]
    fn test_ownership_claim_expiry() {
        let person_secret = PersonSecretKey::generate();
        let device_key = iroh::SecretKey::generate(&mut rand::rng()).public();

        // Create claim with 1 second validity
        let claim = OwnershipClaim::new(&person_secret, device_key, 1);

        // Should not be expired immediately
        assert!(!claim.is_expired());

        // Wait 2 seconds for it to expire
        std::thread::sleep(std::time::Duration::from_secs(2));
        assert!(claim.is_expired());
    }

    #[test]
    fn test_firewall_config_default() {
        let config = FirewallConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.trusted_persons.len(), 0);
        assert_eq!(config.policies.len(), 0);
    }

    #[test]
    fn test_firewall_add_remove_person() {
        let mut config = FirewallConfig::new();
        let person_key = PersonSecretKey::generate().public_key();

        let person = TrustedPerson {
            name: "alice".to_string(),
            comment: Some("Test person".to_string()),
            key: person_key.clone(),
        };

        config.add_person(person);
        assert_eq!(config.trusted_persons.len(), 1);
        assert!(config.get_person("alice").is_some());

        assert!(config.remove_person("alice"));
        assert_eq!(config.trusted_persons.len(), 0);
        assert!(config.get_person("alice").is_none());
    }

    #[test]
    fn test_firewall_verify_claim() {
        let mut config = FirewallConfig::new();
        config.enabled = true;

        let person_secret = PersonSecretKey::generate();
        let person_key = person_secret.public_key();
        let device_key = iroh::SecretKey::generate(&mut rand::rng()).public();

        // Add person to trusted list
        config.add_person(TrustedPerson {
            name: "bob".to_string(),
            comment: None,
            key: person_key.clone(),
        });

        // Create and verify claim
        let claim = OwnershipClaim::new(&person_secret, device_key, 3600);
        let result = config.verify_claim(&claim);

        assert!(result.is_ok());
        assert!(result.unwrap()); // Person is trusted
        assert!(config.verified_devices.contains_key(&device_key));
    }

    #[test]
    fn test_firewall_untrusted_person() {
        let mut config = FirewallConfig::new();
        config.enabled = true;

        let person_secret = PersonSecretKey::generate();
        let device_key = iroh::SecretKey::generate(&mut rand::rng()).public();

        // Don't add person to trusted list
        let claim = OwnershipClaim::new(&person_secret, device_key, 3600);
        let result = config.verify_claim(&claim);

        assert!(result.is_ok());
        assert!(!result.unwrap()); // Person not trusted
        assert!(!config.verified_devices.contains_key(&device_key));
    }

    #[test]
    fn test_firewall_disabled_allows_all() {
        let config = FirewallConfig::new(); // disabled by default
        let device_key = iroh::SecretKey::generate(&mut rand::rng()).public();

        assert!(config.is_device_allowed(&device_key));
    }

    #[test]
    fn test_config_persistence() {
        use tempfile::TempDir;
        let temp_dir = TempDir::new().unwrap();

        let home_path = temp_dir.path().to_str().unwrap();

        let mut config = FirewallConfig::new();
        config.enabled = true;

        let person_key = PersonSecretKey::generate().public_key();
        config.add_person(TrustedPerson {
            name: "test_person".to_string(),
            comment: Some("Test".to_string()),
            key: person_key.clone(),
        });

        // Save config
        config.save_with_home(Some(home_path)).unwrap();

        // Load config
        let loaded = FirewallConfig::load_from_home(Some(home_path)).unwrap();
        assert!(loaded.enabled);
        assert_eq!(loaded.trusted_persons.len(), 1);
        assert_eq!(loaded.trusted_persons[0].name, "test_person");
    }

    #[test]
    fn test_cache_persistence() {
        use tempfile::TempDir;
        let temp_dir = TempDir::new().unwrap();

        let home_path = temp_dir.path().to_str().unwrap();

        let mut config = FirewallConfig::new();
        let person_key = PersonSecretKey::generate().public_key();
        let device_key = iroh::SecretKey::generate(&mut rand::rng()).public();

        config
            .verified_devices
            .insert(device_key, person_key.clone());

        // Save cache
        config.save_cache_with_home(Some(home_path)).unwrap();

        // Load config (which includes cache)
        let loaded = FirewallConfig::load_from_home(Some(home_path)).unwrap();
        assert!(loaded.verified_devices.contains_key(&device_key));
    }

    #[test]
    fn test_person_secret_key_helpers() {
        use tempfile::TempDir;
        let _temp_dir = TempDir::new().unwrap();

        // Generate a key
        let secret = PersonSecretKey::generate();
        let public = secret.public_key();

        // Test hex round-trip
        let hex = secret.to_hex();
        let parsed = PersonSecretKey::from_hex(&hex).unwrap();
        assert_eq!(parsed.public_key(), public);
    }

    #[test]
    fn test_claim_save_and_load() {
        use tempfile::TempDir;
        let temp_dir = TempDir::new().unwrap();

        // Save original HOME
        let original_home = std::env::var("HOME").ok();

        // Override HOME for this test
        unsafe {
            std::env::set_var("HOME", temp_dir.path());
        }

        let person_secret = PersonSecretKey::generate();
        let device_key = iroh::SecretKey::generate(&mut rand::rng()).public();

        // Create and save a claim
        let claim = OwnershipClaim::new(&person_secret, device_key, 3600);
        FirewallConfig::save_claim(&claim).unwrap();

        // Load it back
        let loaded = FirewallConfig::load_claim(&device_key).unwrap();
        assert!(loaded.is_some());

        let loaded_claim = loaded.unwrap();
        assert_eq!(loaded_claim.device_key, device_key);
        assert_eq!(loaded_claim.person_key, person_secret.public_key());

        // Restore original HOME
        unsafe {
            if let Some(home) = original_home {
                std::env::set_var("HOME", home);
            } else {
                std::env::remove_var("HOME");
            }
        }
    }

    #[test]
    fn test_port_range_parsing() {
        // Test wildcard
        let any = PortRange::parse("*").unwrap();
        assert_eq!(any, PortRange::Any);

        // Test single port
        let single = PortRange::parse("80").unwrap();
        assert_eq!(single, PortRange::Single(80));

        // Test range
        let range = PortRange::parse("1000-2000").unwrap();
        assert_eq!(
            range,
            PortRange::Range {
                min: 1000,
                max: 2000
            }
        );

        // Test from
        let from = PortRange::parse("1000-").unwrap();
        assert_eq!(from, PortRange::From(1000));

        // Test invalid
        assert!(PortRange::parse("invalid").is_err());
        assert!(PortRange::parse("2000-1000").is_err()); // Reversed range
    }

    #[test]
    fn test_port_range_matching() {
        // Any matches everything
        let any = PortRange::Any;
        assert!(any.matches(1));
        assert!(any.matches(80));
        assert!(any.matches(65535));

        // Single matches only exact port
        let single = PortRange::Single(80);
        assert!(!single.matches(79));
        assert!(single.matches(80));
        assert!(!single.matches(81));

        // Range matches inclusive
        let range = PortRange::Range {
            min: 1000,
            max: 2000,
        };
        assert!(!range.matches(999));
        assert!(range.matches(1000));
        assert!(range.matches(1500));
        assert!(range.matches(2000));
        assert!(!range.matches(2001));

        // From matches anything >= min
        let from = PortRange::From(1000);
        assert!(!from.matches(999));
        assert!(from.matches(1000));
        assert!(from.matches(50000));
        assert!(from.matches(65535));
    }

    #[test]
    fn test_packet_source_parsing() {
        // Test wildcard
        let any = PacketSource::parse("*").unwrap();
        assert_eq!(any, PacketSource::Any);

        // Test person
        let person = PacketSource::parse("person:alice").unwrap();
        assert_eq!(person, PacketSource::Person("alice".to_string()));

        // Test peer with prefix
        let endpoint_id = iroh::SecretKey::generate(&mut rand::rng()).public();
        let peer = PacketSource::parse(&format!("peer:{}", endpoint_id)).unwrap();
        assert_eq!(peer, PacketSource::Peer(endpoint_id));

        // Test backward compat (raw endpoint ID)
        let peer2 = PacketSource::parse(&endpoint_id.to_string()).unwrap();
        assert_eq!(peer2, PacketSource::Peer(endpoint_id));

        // Test invalid
        assert!(PacketSource::parse("invalid").is_err());
    }

    #[test]
    fn test_packet_source_matching() {
        let mut config = FirewallConfig::new();

        let person_secret = PersonSecretKey::generate();
        let person_key = person_secret.public_key();
        let device1 = iroh::SecretKey::generate(&mut rand::rng()).public();
        let device2 = iroh::SecretKey::generate(&mut rand::rng()).public();

        // Add person to config
        config.add_person(TrustedPerson {
            name: "alice".to_string(),
            comment: None,
            key: person_key.clone(),
        });

        // Test Any - matches everything
        let any = PacketSource::Any;
        assert!(any.matches(&device1, None, &config));
        assert!(any.matches(&device2, Some(&person_key), &config));

        // Test Peer - matches specific device only
        let peer = PacketSource::Peer(device1);
        assert!(peer.matches(&device1, None, &config));
        assert!(!peer.matches(&device2, None, &config));

        // Test Person - matches device owned by that person
        let person = PacketSource::Person("alice".to_string());
        assert!(person.matches(&device1, Some(&person_key), &config));
        assert!(!person.matches(&device1, None, &config)); // No person info
        assert!(person.matches(&device2, Some(&person_key), &config)); // Same person

        // Test unknown person
        let unknown = PacketSource::Person("bob".to_string());
        assert!(!unknown.matches(&device1, Some(&person_key), &config));
    }

    #[test]
    fn test_firewall_policy_matching() {
        let mut config = FirewallConfig::new();
        config.enabled = true;

        let person_secret = PersonSecretKey::generate();
        let person_key = person_secret.public_key();
        let device = iroh::SecretKey::generate(&mut rand::rng()).public();

        config.add_person(TrustedPerson {
            name: "alice".to_string(),
            comment: None,
            key: person_key.clone(),
        });

        // Cache the device as verified
        config.verified_devices.insert(device, person_key.clone());

        // Policy: accept from alice on any port
        let policy1 = FirewallPolicy::accept_from(PacketSource::Person("alice".to_string()));
        assert!(policy1.matches(&device, Some(&person_key), 80, &config));
        assert!(policy1.matches(&device, Some(&person_key), 443, &config));

        // Policy: accept from alice on port 80 only
        let policy2 = FirewallPolicy::accept_from_with_port(
            PacketSource::Person("alice".to_string()),
            PortRange::Single(80),
        );
        assert!(policy2.matches(&device, Some(&person_key), 80, &config));
        assert!(!policy2.matches(&device, Some(&person_key), 443, &config));

        // Policy: accept from any on ports 1000+
        let policy3 =
            FirewallPolicy::accept_from_with_port(PacketSource::Any, PortRange::From(1000));
        assert!(policy3.matches(&device, Some(&person_key), 1000, &config));
        assert!(policy3.matches(&device, Some(&person_key), 65535, &config));
        assert!(!policy3.matches(&device, Some(&person_key), 999, &config));
    }

    #[test]
    fn test_firewall_is_packet_allowed() {
        let mut config = FirewallConfig::new();
        config.enabled = true;

        let person_secret = PersonSecretKey::generate();
        let person_key = person_secret.public_key();
        let device = iroh::SecretKey::generate(&mut rand::rng()).public();

        config.add_person(TrustedPerson {
            name: "alice".to_string(),
            comment: None,
            key: person_key.clone(),
        });

        // Cache the device
        config.verified_devices.insert(device, person_key.clone());

        // No policies - should fall back to device verification
        assert!(config.is_packet_allowed(&device, 80));

        // Add policy: accept from alice on port 80 only
        config.policies.push(FirewallPolicy::accept_from_with_port(
            PacketSource::Person("alice".to_string()),
            PortRange::Single(80),
        ));

        // Should allow port 80
        assert!(config.is_packet_allowed(&device, 80));

        // Should reject port 443
        assert!(!config.is_packet_allowed(&device, 443));

        // Add wildcard policy for ports 1000+
        config.policies.push(FirewallPolicy::accept_from_with_port(
            PacketSource::Any,
            PortRange::From(1000),
        ));

        // Should now allow ports 1000+
        assert!(config.is_packet_allowed(&device, 1000));
        assert!(config.is_packet_allowed(&device, 8080));

        // Should still reject other ports
        assert!(!config.is_packet_allowed(&device, 443));
    }
}
