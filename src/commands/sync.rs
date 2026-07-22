use anyhow::Result;
use dialoguer::Confirm;
use std::io::{self, IsTerminal, Write};

use crate::{
    api::GongClient,
    cli::{Cli, SyncArgs},
    config::Config,
    sync::{StatusTracker, SyncOptions, SyncReport, execute as execute_plan, prepare},
};

pub async fn execute(cli: &Cli, args: &SyncArgs) -> Result<i32> {
    let (config, _) = Config::load(cli)?;
    let client = GongClient::new(config.base_url, config.access_key, config.access_key_secret);
    let status_path = args.status_file.clone().or(config.status_file);
    let options = SyncOptions {
        output_dir: config.output_dir,
        overlap_days: config.overlap_days,
        from: args.from,
        to: args.to,
        dry_run: args.dry_run,
        quiet: args.quiet,
        full: args.full,
        force: args.force,
    };
    let status = if args.dry_run {
        None
    } else {
        status_path.map(StatusTracker::start).transpose()?
    };
    let result: Result<SyncReport> = async {
        let plan = prepare(&client, &options).await?;
        if options.full && !options.quiet {
            plan.report_orphans();
        }
        if options.force {
            eprintln!(
                "preflight: overwrite {}, new {}, orphans {}",
                plan.overwrite_count(),
                plan.new_count(),
                plan.orphan_count()
            );
        }
        if options.dry_run {
            return Ok(plan.preview(options.quiet));
        }
        if options.force && !args.yes {
            let confirmed = confirm_force()?;
            if !confirmed {
                eprintln!("sync cancelled; no Call Files were changed");
                return Ok(SyncReport::default());
            }
        }
        execute_plan(&client, &options, plan).await
    }
    .await;
    match result {
        Ok(report) => {
            let summary = report.summary();
            if let Some(status) = status {
                status.finish(report, &summary)?;
            }
            println!("{summary}");
            Ok(if report.failed == 0 { 0 } else { 2 })
        }
        Err(error) => {
            if let Some(status) = status {
                status.finish_failure(SyncReport::default(), &format!("{error:#}"))?;
            }
            Err(error)
        }
    }
}

fn confirm_force() -> Result<bool> {
    if io::stdin().is_terminal() && io::stderr().is_terminal() {
        return Ok(Confirm::new()
            .with_prompt("Proceed with forced Call File overwrites?")
            .default(false)
            .interact()?);
    }

    eprint!("Proceed with forced Call File overwrites? [y/N] ");
    io::stderr().flush()?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    Ok(matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}
