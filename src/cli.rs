use chrono::NaiveDate;
use clap::{ArgAction, Args, Parser, Subcommand};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(
    name = "gong",
    version,
    about = "Sync Gong Customer Calls into Markdown Call Files"
)]
pub struct Cli {
    /// Path to the TOML configuration file.
    #[arg(long, global = true)]
    pub config: Option<PathBuf>,

    /// Gong API access key.
    #[arg(long, global = true, env = "GONG_ACCESS_KEY", hide_env_values = true)]
    pub access_key: Option<String>,

    /// Gong API access key secret.
    #[arg(
        long,
        global = true,
        env = "GONG_ACCESS_KEY_SECRET",
        hide_env_values = true
    )]
    pub access_key_secret: Option<String>,

    /// Gong API base URL for the organization.
    #[arg(long, global = true, env = "GONG_BASE_URL")]
    pub base_url: Option<String>,

    /// Directory containing canonical Call Files.
    #[arg(long, global = true, env = "GONG_OUTPUT_DIR")]
    pub output_dir: Option<PathBuf>,

    /// Increase diagnostic verbosity (-v, -vv, -vvv).
    #[arg(short, long, global = true, action = ArgAction::Count)]
    pub verbose: u8,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Validate configuration, credentials, API access, and the Output Directory.
    Check,
    /// List Customer Calls in a date range without writing Call Files.
    List(ListArgs),
    /// Fetch and render one Call by its exact Gong Call ID.
    Get(GetArgs),
    /// Incrementally sync Customer Calls into the Output Directory.
    Sync(SyncArgs),
}

#[derive(Debug, Args)]
pub struct GetArgs {
    /// Exact Gong Call ID. IDs are handled as strings and never as numbers.
    pub call_id: String,

    /// Write the canonical Call File to this path instead of stdout.
    #[arg(long, value_name = "PATH", conflicts_with = "json")]
    pub output: Option<PathBuf>,

    /// Emit the merged extensive Call and Transcript payload as JSON.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct SyncArgs {
    /// Override the first date to fetch (YYYY-MM-DD).
    #[arg(long)]
    pub from: Option<NaiveDate>,

    /// Override the last whole date to fetch (YYYY-MM-DD).
    #[arg(long)]
    pub to: Option<NaiveDate>,

    /// Fetch all available Call history instead of deriving a High-Water Mark.
    #[arg(long, conflicts_with_all = ["from", "to"])]
    pub full: bool,

    /// Re-fetch and overwrite matched complete Call Files after confirmation.
    #[arg(long)]
    pub force: bool,

    /// Confirm a forced overwrite non-interactively.
    #[arg(long, requires = "force")]
    pub yes: bool,

    /// Preview new, Healing, and skip decisions without fetching Transcripts or writing files.
    #[arg(long)]
    pub dry_run: bool,

    /// Suppress non-error detail on stderr; the stdout summary is retained.
    #[arg(long)]
    pub quiet: bool,

    /// Override the optional flat key=value sync status file.
    #[arg(long, value_name = "PATH")]
    pub status_file: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct ListArgs {
    /// First calendar date to include (YYYY-MM-DD, UTC query boundary).
    #[arg(long)]
    pub from: NaiveDate,

    /// Last calendar date to include (YYYY-MM-DD); defaults to today.
    #[arg(long)]
    pub to: Option<NaiveDate>,

    /// Emit a JSON array for scripts instead of the human table.
    #[arg(long)]
    pub json: bool,
}
