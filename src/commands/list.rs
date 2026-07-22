use anyhow::{Context, Result, bail};
use chrono::{DateTime, Days, NaiveDate, Utc};
use serde::Serialize;

use crate::{
    api::{ExtensiveCall, GongClient},
    cli::{Cli, ListArgs},
    config::Config,
};

#[derive(Debug, Serialize)]
struct ListRow {
    id: String,
    title: String,
    started: String,
    duration_minutes: u64,
    account: String,
}

pub async fn execute(cli: &Cli, args: &ListArgs) -> Result<i32> {
    let (config, _) = Config::load(cli)?;
    let to_date = args.to.unwrap_or_else(|| Utc::now().date_naive());
    if to_date < args.from {
        bail!("--to must be the same as or later than --from");
    }
    let from = start_of_day(args.from)?;
    let to = start_of_day(
        to_date
            .checked_add_days(Days::new(1))
            .context("--to date is too large")?,
    )?;

    let client = GongClient::new(config.base_url, config.access_key, config.access_key_secret);
    let rows: Vec<ListRow> = client
        .list_calls(from, to)
        .await
        .context("failed to list Calls from Gong")?
        .into_iter()
        .filter(ExtensiveCall::is_customer_call)
        .map(ListRow::from)
        .collect();

    if args.json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
    } else {
        print_table(&rows);
    }
    Ok(0)
}

fn start_of_day(date: NaiveDate) -> Result<DateTime<Utc>> {
    date.and_hms_opt(0, 0, 0)
        .map(|value| value.and_utc())
        .context("date cannot be represented at midnight")
}

impl From<ExtensiveCall> for ListRow {
    fn from(call: ExtensiveCall) -> Self {
        Self {
            id: call.metadata.id.clone(),
            title: call.metadata.title.clone(),
            started: call.metadata.started.clone(),
            duration_minutes: call.metadata.duration / 60,
            account: call.account_name().unwrap_or_default().to_owned(),
        }
    }
}

fn print_table(rows: &[ListRow]) {
    println!("DATE\tTIME\tID\tTITLE\tACCOUNT");
    for row in rows {
        let (date, time) = DateTime::parse_from_rfc3339(&row.started)
            .map(|started| {
                (
                    started.format("%Y-%m-%d").to_string(),
                    started.format("%H:%M").to_string(),
                )
            })
            .unwrap_or_else(|_| (row.started.clone(), String::new()));
        println!(
            "{}\t{}\t{}\t{}\t{}",
            date, time, row.id, row.title, row.account
        );
    }
}
