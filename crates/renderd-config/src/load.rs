//! Layered configuration loading using Figment provider.

use std::path::{Path, PathBuf};

use figment::{
    providers::{Env, Format, Serialized, Toml},
    Figment,
};

use crate::error::ConfigError;
use crate::schema::RenderdConfig;

/// `ConfigBuilder` constructs a `RenderdConfig` instance across layered sources.
#[derive(Debug, Default)]
pub struct ConfigBuilder {
    figment: Figment,
}

impl ConfigBuilder {
    /// Creates a new `ConfigBuilder` pre-populated with hardcoded system defaults.
    #[must_use]
    pub fn new() -> Self {
        Self {
            figment: Figment::from(Serialized::defaults(RenderdConfig::default())),
        }
    }

    /// Merges settings from a TOML configuration file if it exists at `path`.
    #[must_use]
    pub fn add_file<P: AsRef<Path>>(mut self, path: P) -> Self {
        let p = path.as_ref();
        if p.exists() {
            self.figment = self.figment.merge(Toml::file(p));
        }
        self
    }

    /// Merges settings from environment variables prefixed with `RENDERD_` (e.g. `RENDERD_NETWORK__LISTEN_PORT=4433`).
    #[must_use]
    pub fn add_env(mut self) -> Self {
        self.figment = self.figment.merge(Env::prefixed("RENDERD_").split("__"));
        self
    }

    /// Merges an explicit key-value override into the builder.
    ///
    /// # Errors
    /// Returns [`ConfigError`] if the key cannot be serialized into Figment's value format.
    pub fn add_override<T: serde::Serialize>(
        mut self,
        key: &str,
        value: T,
    ) -> Result<Self, ConfigError> {
        let serialized_value = figment::value::Value::serialize(value)
            .map_err(|e| ConfigError::ParseError(e.to_string()))?;
        self.figment = self.figment.merge((key, serialized_value));
        Ok(self)
    }

    /// Builds and extracts the resolved `RenderdConfig` instance.
    ///
    /// # Errors
    /// Returns [`ConfigError`] if configuration parsing or deserialization fails.
    pub fn build(self) -> Result<RenderdConfig, ConfigError> {
        self.figment
            .extract::<RenderdConfig>()
            .map_err(|e| ConfigError::ParseError(e.to_string()))
    }
}

/// Helper function to locate default configuration paths on the current platform.
#[must_use]
pub fn default_config_path() -> Option<PathBuf> {
    let relative = PathBuf::from("renderd.toml");
    if relative.exists() {
        return Some(relative);
    }

    #[cfg(target_os = "macos")]
    {
        let home_config = std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|p| p.join(".config").join("renderd").join("renderd.toml"));
        if let Some(p) = home_config {
            if p.exists() {
                return Some(p);
            }
        }
        let sys_config = PathBuf::from("/etc/renderd/renderd.toml");
        if sys_config.exists() {
            return Some(sys_config);
        }
    }

    #[cfg(target_os = "windows")]
    {
        let app_data = std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .map(|p| p.join("Renderd").join("renderd.toml"));
        if let Some(p) = app_data {
            if p.exists() {
                return Some(p);
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {

    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_layered_config_loading() {
        let mut temp_file = NamedTempFile::new().unwrap();
        writeln!(
            temp_file,
            r"
[host]
target_fps = 120

[network]
listen_port = 9000
"
        )
        .unwrap();

        let config = ConfigBuilder::new()
            .add_file(temp_file.path())
            .add_override("network.listen_port", 9999)
            .unwrap()
            .build()
            .unwrap();

        assert_eq!(config.host.target_fps, 120);
        assert_eq!(config.network.listen_port, 9999);
    }
}
