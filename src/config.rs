use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::path::{Path, PathBuf};

use crate::cli::Cli;

#[derive(Debug, Clone)]
pub struct Config {
    pub access_key: String,
    pub access_key_secret: String,
    pub base_url: String,
    pub output_dir: PathBuf,
    pub status_file: Option<PathBuf>,
    pub overlap_days: u64,
}

#[derive(Debug, Default, Deserialize)]
struct FileConfig {
    access_key: Option<String>,
    access_key_secret: Option<String>,
    base_url: Option<String>,
    output_dir: Option<PathBuf>,
    status_file: Option<PathBuf>,
    #[serde(default)]
    sync: SyncConfig,
}

#[derive(Debug, Deserialize)]
struct SyncConfig {
    #[serde(default = "default_overlap_days")]
    overlap_days: u64,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            overlap_days: default_overlap_days(),
        }
    }
}

fn default_overlap_days() -> u64 {
    3
}

impl Config {
    pub fn path(cli_path: Option<&Path>) -> Result<PathBuf> {
        if let Some(path) = cli_path {
            return Ok(path.to_path_buf());
        }

        let home = std::env::var_os("HOME")
            .filter(|value| !value.is_empty())
            .context("HOME is not set; pass --config /path/to/config.toml")?;
        Ok(PathBuf::from(home)
            .join(".config")
            .join("gong-cli")
            .join("config.toml"))
    }

    pub fn load(cli: &Cli) -> Result<(Self, PathBuf)> {
        Self::load_inner(cli, false)
    }

    pub fn load_required(cli: &Cli) -> Result<(Self, PathBuf)> {
        Self::load_inner(cli, true)
    }

    fn load_inner(cli: &Cli, require_file: bool) -> Result<(Self, PathBuf)> {
        let path = Self::path(cli.config.as_deref())?;
        let contents = match std::fs::read_to_string(&path) {
            Ok(contents) => contents,
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound
                    && cli.config.is_none()
                    && !require_file =>
            {
                String::new()
            }
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "configuration file {} does not exist or cannot be read; create it or pass --config",
                        path.display()
                    )
                });
            }
        };
        let file: FileConfig = toml::from_str(&contents)
            .with_context(|| format!("invalid TOML in configuration file {}", path.display()))?;

        let access_key = required(
            "access_key",
            cli.access_key.clone().or(file.access_key),
            "set access_key in the config file, GONG_ACCESS_KEY, or --access-key",
        )?;
        let access_key_secret = required(
            "access_key_secret",
            cli.access_key_secret.clone().or(file.access_key_secret),
            "set access_key_secret in the config file, GONG_ACCESS_KEY_SECRET, or --access-key-secret",
        )?;
        let base_url = required(
            "base_url",
            cli.base_url.clone().or(file.base_url),
            "set base_url in the config file, GONG_BASE_URL, or --base-url",
        )?;
        let output_dir = cli.output_dir.clone().or(file.output_dir).with_context(|| {
            "missing setting output_dir; set it in the config file, GONG_OUTPUT_DIR, or --output-dir"
        })?;

        Ok((
            Self {
                access_key,
                access_key_secret,
                base_url: base_url.trim_end_matches('/').to_owned(),
                output_dir,
                status_file: file.status_file,
                overlap_days: file.sync.overlap_days,
            },
            path,
        ))
    }
}

fn required(name: &str, value: Option<String>, help: &str) -> Result<String> {
    match value.filter(|value| !value.trim().is_empty()) {
        Some(value) => Ok(value),
        None => bail!("missing setting {name}; {help}"),
    }
}
