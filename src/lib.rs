use anyhow::Result;
use clap::Parser;

pub mod api;
pub mod cli;
pub mod commands;
pub mod config;
pub mod renderer;
pub mod sync;

pub async fn run() -> Result<i32> {
    let cli = cli::Cli::parse();
    init_tracing(cli.verbose);
    commands::execute(cli).await
}

fn init_tracing(verbose: u8) {
    let directive = match verbose {
        0 => "warn",
        1 => "info",
        2 => "debug",
        _ => "trace",
    };

    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new(directive))
        .with_writer(std::io::stderr)
        .without_time()
        .try_init();
}
