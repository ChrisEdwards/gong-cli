use anyhow::{Context, Result, bail};
use std::fs::OpenOptions;

use crate::{api::GongClient, cli::Cli, config::Config};

pub async fn execute(cli: &Cli) -> Result<i32> {
    let (config, config_path) = Config::load_required(cli)?;
    println!("[PASS] config: loaded {}", config_path.display());
    report_permissions(&config_path)?;

    let client = GongClient::new(
        config.base_url.clone(),
        config.access_key.clone(),
        config.access_key_secret.clone(),
    );
    client
        .verify_credentials()
        .await
        .context("credentials or Gong API connectivity are invalid; check access_key, access_key_secret, and base_url")?;
    println!("[PASS] credentials: Gong API authentication succeeded");

    verify_output_directory(&config)?;
    println!(
        "[PASS] output directory: {} is writable",
        config.output_dir.display()
    );
    Ok(0)
}

#[cfg(unix)]
fn report_permissions(path: &std::path::Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let mode = std::fs::metadata(path)
        .with_context(|| format!("cannot inspect permissions for {}", path.display()))?
        .permissions()
        .mode();
    if mode & 0o077 != 0 {
        println!(
            "[WARN] config permissions: {} is readable by group or others; run chmod 600 {}",
            path.display(),
            path.display()
        );
    } else {
        println!("[PASS] config permissions: owner-only");
    }
    Ok(())
}

#[cfg(not(unix))]
fn report_permissions(_path: &std::path::Path) -> Result<()> {
    Ok(())
}

fn verify_output_directory(config: &Config) -> Result<()> {
    if !config.output_dir.is_dir() {
        bail!(
            "output_dir {} does not exist or is not a directory; create it or update output_dir",
            config.output_dir.display()
        );
    }

    let probe = config
        .output_dir
        .join(format!(".gong-write-check-{}", std::process::id()));
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
        .with_context(|| {
            format!(
                "output_dir {} is not writable; fix its permissions",
                config.output_dir.display()
            )
        })?;
    drop(file);
    std::fs::remove_file(&probe)
        .with_context(|| format!("could not remove write probe {}", probe.display()))?;
    Ok(())
}
