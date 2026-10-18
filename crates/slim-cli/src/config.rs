//! Configuration management for SLIM CLI.

use directories::ProjectDirs;
use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};

use slim_core::model::automation::AutomationRule;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Config {
    pub db_path: Option<PathBuf>,
    pub default_actor: Option<String>,
    #[serde(default)]
    pub rules: Option<Vec<AutomationRule>>,
    pub rules_path: Option<PathBuf>,
    pub allow_commands: Option<bool>,
    pub command_allowlist: Option<Vec<String>>,
}

impl Config {
    /// An explicitly selected or discovered invalid file is an error, never a silent fallback.
    pub fn load(custom_path: Option<&Path>) -> anyhow::Result<Self> {
        let selected = custom_path.map(Path::to_path_buf).or_else(|| {
            let local = PathBuf::from("config.toml");
            if local.exists() {
                Some(local)
            } else {
                ProjectDirs::from("", "", "slim")
                    .map(|p| p.config_dir().join("config.toml"))
                    .filter(|p| p.exists())
            }
        });
        if let Some(path) = selected {
            use anyhow::Context;
            let content = fs::read_to_string(&path)
                .with_context(|| format!("Cannot read configuration {}", path.display()))?;
            return toml::from_str(&content)
                .with_context(|| format!("Invalid configuration {}", path.display()));
        }
        Ok(Self::default())
    }

    /// Resolve the database path from CLI flag, config file, or platform default.
    pub fn resolve_db_path(&self, cli_db_path: Option<&Path>) -> PathBuf {
        if let Some(path) = cli_db_path {
            return path.to_path_buf();
        }

        if let Some(ref path) = self.db_path {
            return path.clone();
        }

        if let Some(proj_dirs) = ProjectDirs::from("", "", "slim") {
            if let Ok(saved) =
                fs::read_to_string(proj_dirs.data_dir().join("preferred-inventory.txt"))
            {
                let preferred = PathBuf::from(saved.trim());
                if preferred.is_file() {
                    return preferred;
                }
            }
            proj_dirs.data_dir().join("slim.sqlite3")
        } else {
            PathBuf::from("slim.sqlite3")
        }
    }

    /// Load declarative automation rules from config or rules.toml files.
    pub fn load_file_rules(&self) -> Vec<AutomationRule> {
        let mut rules = self.rules.clone().unwrap_or_default();

        let paths_to_try = [self.rules_path.clone(), Some(PathBuf::from("rules.toml"))];

        for path in paths_to_try.into_iter().flatten() {
            if path.exists()
                && let Ok(content) = fs::read_to_string(&path)
            {
                #[derive(Deserialize)]
                struct RulesFile {
                    #[serde(default)]
                    rules: Vec<AutomationRule>,
                }
                if let Ok(file_data) = toml::from_str::<RulesFile>(&content) {
                    for r in file_data.rules {
                        if !rules.iter().any(|existing| existing.id == r.id) {
                            rules.push(r);
                        }
                    }
                }
            }
        }

        rules
    }

    /// Path to store active session token.
    pub fn session_file_path() -> PathBuf {
        if let Ok(path) = std::env::var("SLIM_SESSION_FILE") {
            let p = PathBuf::from(path);
            if let Some(parent) = p.parent() {
                let _ = fs::create_dir_all(parent);
            }
            return p;
        }
        if let Some(proj_dirs) = ProjectDirs::from("", "", "slim") {
            let dir = proj_dirs.data_dir();
            let _ = fs::create_dir_all(dir);
            dir.join("session.token")
        } else {
            PathBuf::from(".slim_session")
        }
    }

    /// Load active session token if present.
    pub fn load_session_token() -> Option<String> {
        let path = Self::session_file_path();
        if path.exists() {
            fs::read_to_string(path)
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        } else {
            None
        }
    }

    /// Save an active session token.
    pub fn save_session_token(token: &str) -> std::io::Result<()> {
        let path = Self::session_file_path();
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        fs::write(path, token.trim())
    }

    /// Clear stored session token.
    pub fn clear_session_token() -> std::io::Result<()> {
        let path = Self::session_file_path();
        if path.exists() {
            fs::remove_file(path)?;
        }
        Ok(())
    }
}
