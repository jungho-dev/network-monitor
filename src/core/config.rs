//! Optional `config.toml` (capture defaults, display, and saved named filters).
//! CLI arguments take precedence over config values.

use crate::tui::level::OutputLevel;
use network_monitor_core::app_dir::{app_dir_in, home_dir};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Config {
    pub capture: CaptureConfig,
    pub display: DisplayConfig,
    /// Saved named filters, recalled in the filter bar as `:name`.
    pub filters: HashMap<String, String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct CaptureConfig {
    pub default_interface: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct DisplayConfig {
    pub color_scheme: String,
    pub output_level: Option<String>,
}

impl Default for DisplayConfig {
    fn default() -> Self {
        Self {
            color_scheme: "dark".into(),
            output_level: None,
        }
    }
}

impl Config {
    /// Load `~/.config/network-monitor/config.toml` (`%USERPROFILE%\.config` on Windows),
    /// falling back to defaults (and warning) on any error.
    pub fn load() -> Self {
        Self::load_in(home_dir().as_deref())
    }

    /// `home` is injected so tests never read the real profile.
    fn load_in(home: Option<&Path>) -> Self {
        let Some(home) = home else {
            return Self::default();
        };
        let path = app_dir_in(home).join("config.toml");
        match std::fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text).unwrap_or_else(|e| {
                eprintln!("network-monitor: ignoring invalid {}: {e}", path.display());
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    /// `display.output_level`: debug when absent; an unknown value is reported
    /// on stderr once and falls back to debug.
    pub fn output_level(&self) -> OutputLevel {
        let Some(text) = self.display.output_level.as_deref() else {
            return OutputLevel::Debug;
        };
        if let Some(level) = OutputLevel::from_code(text) {
            return level;
        }
        eprintln!(
            "network-monitor: ignoring unknown display.output_level \"{text}\" (expected debug, info, warn, or error)"
        );
        OutputLevel::Debug
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn write_config(home: &Path, text: &str) -> PathBuf {
        let path = app_dir_in(home).join("config.toml");
        std::fs::create_dir_all(path.parent().expect("config dir")).expect("create dir");
        std::fs::write(&path, text).expect("write config");
        path
    }

    #[test]
    fn load_reads_the_file_under_the_injected_home() {
        let home = tempfile::tempdir().expect("temp home");
        write_config(
            home.path(),
            "[capture]\ndefault_interface = \"eth9\"\n[filters]\nweb = \"tcp.port == 443\"\n",
        );
        let config = Config::load_in(Some(home.path()));
        assert_eq!(config.capture.default_interface.as_deref(), Some("eth9"));
        assert_eq!(config.display.color_scheme, "dark");
        assert_eq!(
            config.filters.get("web").map(String::as_str),
            Some("tcp.port == 443")
        );
    }

    #[test]
    fn example_config_parses_with_the_documented_defaults() {
        let config: Config =
            toml::from_str(include_str!("../../docs/config.example.toml")).expect("example parses");
        assert!(config.capture.default_interface.is_none());
        assert_eq!(config.display.color_scheme, "dark");
        assert_eq!(
            config.filters.get("https").map(String::as_str),
            Some("tcp.port == 443")
        );
    }

    #[test]
    fn load_falls_back_to_defaults_without_a_home_or_a_file() {
        let home = tempfile::tempdir().expect("temp home");
        assert!(Config::load_in(None).capture.default_interface.is_none());
        assert!(Config::load_in(Some(home.path())).filters.is_empty());
    }

    #[test]
    fn output_level_defaults_to_debug_and_reads_each_documented_value() {
        assert_eq!(Config::default().output_level(), OutputLevel::Debug);
        for (text, expected) in [
            ("debug", OutputLevel::Debug),
            ("info", OutputLevel::Info),
            ("warn", OutputLevel::Warn),
            ("error", OutputLevel::Error),
        ] {
            let config: Config = toml::from_str(&format!("[display]\noutput_level = \"{text}\"\n"))
                .expect("config parses");
            assert_eq!(config.output_level(), expected, "{text}");
        }
    }

    #[test]
    fn an_unknown_output_level_falls_back_to_debug() {
        let config: Config =
            toml::from_str("[display]\noutput_level = \"loud\"\n").expect("config parses");
        assert_eq!(config.display.output_level.as_deref(), Some("loud"));
        assert_eq!(config.output_level(), OutputLevel::Debug);
    }
}
