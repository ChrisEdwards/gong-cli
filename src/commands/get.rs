use anyhow::{Context, Result};

use crate::{
    api::GongClient,
    cli::{Cli, GetArgs},
    config::Config,
    renderer::{MergedCall, render_call},
};

pub async fn execute(cli: &Cli, args: &GetArgs) -> Result<i32> {
    let (config, _) = Config::load(cli)?;
    let client = GongClient::new(config.base_url, config.access_key, config.access_key_secret);
    let call = client
        .get_call(&args.call_id)
        .await
        .with_context(|| format!("failed to fetch Call {}", args.call_id))?;
    let transcript = client
        .get_transcript(&args.call_id)
        .await
        .with_context(|| format!("failed to fetch Transcript for Call {}", args.call_id))?;
    let merged = MergedCall { call, transcript };

    if args.json {
        println!("{}", serde_json::to_string_pretty(&merged)?);
        return Ok(0);
    }

    let rendered =
        render_call(&merged).with_context(|| format!("failed to render Call {}", args.call_id))?;
    if let Some(path) = &args.output {
        std::fs::write(path, rendered.markdown.as_bytes())
            .with_context(|| format!("failed to write Call File {}", path.display()))?;
    } else {
        print!("{}", rendered.markdown);
    }
    Ok(0)
}
