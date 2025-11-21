//! Configuration types for Ouli

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::{OuliError, Result};

/// Operating mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Record mode: capture traffic and store
    Record,
    /// Replay mode: serve from recordings
    Replay,
}

/// Main configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Operating mode
    pub mode: Mode,
    /// Directory for storing/loading recordings
    pub recording_dir: PathBuf,
    /// Endpoint configurations
    pub endpoints: Vec<EndpointConfig>,
    /// Redaction configuration
    #[serde(default)]
    pub redaction: RedactionConfig,
    /// Resource limits
    #[serde(default)]
    pub limits: LimitsConfig,
}

/// Endpoint configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EndpointConfig {
    /// Target host to proxy to
    pub target_host: String,
    /// Target port
    pub target_port: u16,
    /// Source port to listen on
    pub source_port: u16,
    /// Target type (http/https)
    #[serde(default = "default_https")]
    pub target_type: String,
    /// Source type (http/https)
    #[serde(default = "default_http")]
    pub source_type: String,
    /// Headers to redact from requests
    #[serde(default)]
    pub redact_request_headers: Vec<String>,
}

fn default_https() -> String {
    "https".to_string()
}

fn default_http() -> String {
    "http".to_string()
}

/// Redaction configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RedactionConfig {
    /// Literal secrets to redact
    #[serde(default)]
    pub secrets: Vec<String>,
    /// Regex patterns for redaction
    #[serde(default)]
    pub regex_patterns: Vec<String>,
}

/// Resource limits
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LimitsConfig {
    /// Maximum concurrent connections
    pub max_connections: usize,
    /// Maximum request size in bytes
    pub max_request_size: usize,
    /// Maximum response size in bytes
    pub max_response_size: usize,
    /// Maximum headers per request/response
    pub max_headers: usize,
}

impl Default for LimitsConfig {
    fn default() -> Self {
        Self {
            max_connections: 4096,
            max_request_size: 16 * 1024 * 1024,   // 16 MB
            max_response_size: 256 * 1024 * 1024, // 256 MB
            max_headers: 128,
        }
    }
}

impl Config {
    /// Load configuration from TOML file
    ///
    /// # Errors
    ///
    /// Returns error if file cannot be read or parsed
    pub fn from_file(path: &std::path::Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| OuliError::ConfigError(format!("Failed to read config file: {e}")))?;

        let config: Self = toml::from_str(&content)
            .map_err(|e| OuliError::ConfigError(format!("Failed to parse config: {e}")))?;

        config.validate()?;
        Ok(config)
    }

    /// Validate configuration
    ///
    /// # Errors
    ///
    /// Returns error if configuration is invalid
    ///
    /// # Panics
    ///
    /// Panics if resource limits are zero (programming error)
    pub fn validate(&self) -> Result<()> {
        // Validate recording directory
        if !self.recording_dir.exists() {
            return Err(OuliError::ConfigError(format!(
                "Recording directory does not exist: {}",
                self.recording_dir.display()
            )));
        }

        // Validate at least one endpoint
        if self.endpoints.is_empty() {
            return Err(OuliError::ConfigError(
                "At least one endpoint must be configured".to_string(),
            ));
        }

        // Validate endpoints
        for (i, endpoint) in self.endpoints.iter().enumerate() {
            if endpoint.target_host.is_empty() {
                return Err(OuliError::ConfigError(format!(
                    "Endpoint {i}: target_host cannot be empty"
                )));
            }

            if endpoint.target_port == 0 {
                return Err(OuliError::ConfigError(format!(
                    "Endpoint {i}: target_port cannot be 0"
                )));
            }

            if endpoint.source_port == 0 {
                return Err(OuliError::ConfigError(format!(
                    "Endpoint {i}: source_port cannot be 0"
                )));
            }
        }

        // Validate limits
        assert!(
            self.limits.max_connections > 0,
            "max_connections must be > 0"
        );
        assert!(
            self.limits.max_request_size > 0,
            "max_request_size must be > 0"
        );
        assert!(
            self.limits.max_response_size > 0,
            "max_response_size must be > 0"
        );
        assert!(self.limits.max_headers > 0, "max_headers must be > 0");

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_config_parse() {
        let config_toml = r#"
            mode = "record"
            recording_dir = "/tmp"

            [[endpoints]]
            target_host = "api.example.com"
            target_port = 443
            source_port = 8080
        "#;

        let config: Config = toml::from_str(config_toml).unwrap();
        assert_eq!(config.mode, Mode::Record);
        assert_eq!(config.endpoints.len(), 1);
        assert_eq!(config.endpoints[0].target_host, "api.example.com");
    }

    #[test]
    fn test_config_validation() {
        let mut file = NamedTempFile::new().unwrap();
        let config_toml = r#"
            mode = "replay"
            recording_dir = "/tmp"

            [[endpoints]]
            target_host = "api.example.com"
            target_port = 443
            source_port = 8080
        "#;
        file.write_all(config_toml.as_bytes()).unwrap();

        let config = Config::from_file(file.path()).unwrap();
        assert_eq!(config.mode, Mode::Replay);
    }

    #[test]
    fn test_invalid_config_no_endpoints() {
        let config_toml = r#"
            mode = "record"
            recording_dir = "/tmp"
            endpoints = []
        "#;

        let config: Config = toml::from_str(config_toml).unwrap();
        assert!(config.validate().is_err());
    }
}
