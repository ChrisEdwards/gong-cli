mod check;
mod get;
mod list;
mod sync;

use anyhow::Result;

use crate::cli::{Cli, Command};

pub async fn execute(cli: Cli) -> Result<i32> {
    match cli.command {
        Command::Check => check::execute(&cli).await,
        Command::List(ref args) => list::execute(&cli, args).await,
        Command::Get(ref args) => get::execute(&cli, args).await,
        Command::Sync(ref args) => sync::execute(&cli, args).await,
    }
}
