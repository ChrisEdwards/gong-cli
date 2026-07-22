use crate::{
    api::{CallTranscript, ExtensiveCall, GongClient},
    renderer::{MergedCall, PLACEHOLDER, canonical_filename, render_call},
};
use anyhow::{Context, Result, bail};
use chrono::{DateTime, Days, NaiveDate, SecondsFormat, Utc};
use indicatif::{ProgressBar, ProgressStyle};
use std::{
    collections::{HashMap, HashSet},
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
use unicode_normalization::UnicodeNormalization;

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
pub struct SyncOptions {
    pub output_dir: PathBuf,
    pub overlap_days: u64,
    pub from: Option<NaiveDate>,
    pub to: Option<NaiveDate>,
    pub dry_run: bool,
    pub quiet: bool,
    pub full: bool,
    pub force: bool,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SyncReport {
    pub new: usize,
    pub healed: usize,
    pub skipped: usize,
    pub failed: usize,
}

impl SyncReport {
    pub fn summary(self) -> String {
        format!(
            "synced {} new, healed {}, skipped {}, failed {}",
            self.new, self.healed, self.skipped, self.failed
        )
    }
}

#[derive(Debug)]
pub struct StatusTracker {
    path: PathBuf,
    last_run_at: String,
    previous_success_at: String,
    previous_failure_at: String,
}

impl StatusTracker {
    pub fn start(path: PathBuf) -> Result<Self> {
        let previous = read_status_values(&path)?;
        let tracker = Self {
            path,
            last_run_at: timestamp(),
            previous_success_at: previous.get("last_success_at").cloned().unwrap_or_default(),
            previous_failure_at: previous.get("last_failure_at").cloned().unwrap_or_default(),
        };
        tracker.write(
            "running",
            &tracker.previous_success_at,
            &tracker.previous_failure_at,
            "sync running",
            SyncReport::default(),
        )?;
        Ok(tracker)
    }

    pub fn finish(&self, report: SyncReport, message: &str) -> Result<()> {
        if report.failed == 0 {
            self.write(
                "success",
                &timestamp(),
                &self.previous_failure_at,
                message,
                report,
            )
        } else {
            self.write(
                "partial",
                &self.previous_success_at,
                &timestamp(),
                message,
                report,
            )
        }
    }

    pub fn finish_failure(&self, report: SyncReport, message: &str) -> Result<()> {
        self.write(
            "failure",
            &self.previous_success_at,
            &timestamp(),
            message,
            report,
        )
    }

    fn write(
        &self,
        state: &str,
        success_at: &str,
        failure_at: &str,
        message: &str,
        report: SyncReport,
    ) -> Result<()> {
        let message = message.replace(['\n', '\r'], " ");
        let contents = format!(
            "last_state={state}\n\
             last_run_at={}\n\
             last_success_at={success_at}\n\
             last_failure_at={failure_at}\n\
             last_message={message}\n\
             new={}\n\
             healed={}\n\
             skipped={}\n\
             failed={}\n",
            self.last_run_at, report.new, report.healed, report.skipped, report.failed
        );
        atomic_write(&self.path, contents.as_bytes())
            .with_context(|| format!("cannot update sync status file {}", self.path.display()))
    }
}

#[derive(Debug)]
struct ExistingFiles {
    by_id: HashMap<String, PathBuf>,
    by_filename: HashMap<String, PathBuf>,
    incomplete: HashSet<PathBuf>,
    all_paths: HashSet<PathBuf>,
}

#[derive(Debug)]
struct PlannedCall {
    call: ExtensiveCall,
    filename: String,
}

#[derive(Debug, Clone, Copy)]
enum Action {
    New,
    Heal,
    Overwrite,
    Skip,
}

#[derive(Debug)]
struct PlannedOperation {
    call: ExtensiveCall,
    target: PathBuf,
    action: Action,
}

#[derive(Debug)]
pub struct SyncPlan {
    operations: Vec<PlannedOperation>,
    orphans: Vec<PathBuf>,
}

impl SyncPlan {
    pub fn overwrite_count(&self) -> usize {
        self.operations
            .iter()
            .filter(|operation| matches!(operation.action, Action::Heal | Action::Overwrite))
            .count()
    }

    pub fn new_count(&self) -> usize {
        self.operations
            .iter()
            .filter(|operation| matches!(operation.action, Action::New))
            .count()
    }

    pub fn orphan_count(&self) -> usize {
        self.orphans.len()
    }

    pub fn report_orphans(&self) {
        for path in &self.orphans {
            eprintln!("orphan: {}", path.display());
        }
    }

    pub fn preview(&self, quiet: bool) -> SyncReport {
        let mut report = SyncReport::default();
        for operation in &self.operations {
            record_action(&mut report, operation.action);
            if !quiet {
                eprintln!(
                    "would {} {} {} -> {}",
                    action_name(operation.action),
                    operation.call.metadata.id,
                    operation.call.metadata.title,
                    operation.target.display()
                );
            }
        }
        report
    }
}

pub async fn prepare(client: &GongClient, options: &SyncOptions) -> Result<SyncPlan> {
    if !options.output_dir.is_dir() {
        bail!(
            "Output Directory {} does not exist or is not a directory",
            options.output_dir.display()
        );
    }
    let (existing, calls) = if options.full {
        (
            scan_existing(&options.output_dir, None)?,
            client
                .list_all_calls()
                .await
                .context("failed to list full Call history for sync")?,
        )
    } else {
        let (from, to, from_date, to_date) = sync_window(options)?;
        (
            scan_existing(&options.output_dir, Some((from_date, to_date)))?,
            client
                .list_calls(from, to)
                .await
                .context("failed to list Calls for sync")?,
        )
    };
    let calls = calls
        .into_iter()
        .filter(ExtensiveCall::is_customer_call)
        .collect();
    let planned = plan_filenames(calls)?;
    let mut operations = Vec::with_capacity(planned.len());
    let mut matched_paths = HashSet::new();
    let mut reserved_filenames: HashSet<String> = existing
        .by_filename
        .keys()
        .map(|filename| filename_collision_key(filename))
        .collect();

    for planned_call in planned {
        let call = planned_call.call;
        let matched_path = existing
            .by_id
            .get(&call.metadata.id)
            .or_else(|| existing.by_filename.get(&planned_call.filename));
        let action = match matched_path {
            None => Action::New,
            Some(path) if existing.incomplete.contains(path) => Action::Heal,
            Some(_) if options.force => Action::Overwrite,
            Some(_) => Action::Skip,
        };
        let target = match matched_path {
            Some(path) => path.clone(),
            None => options.output_dir.join(reserve_filename(
                &planned_call.filename,
                &call,
                &mut reserved_filenames,
            )?),
        };
        if matched_path.is_some() {
            matched_paths.insert(target.clone());
        }
        operations.push(PlannedOperation {
            call,
            target,
            action,
        });
    }
    let mut orphans = if options.full {
        existing
            .all_paths
            .difference(&matched_paths)
            .cloned()
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    orphans.sort();
    Ok(SyncPlan {
        operations,
        orphans,
    })
}

pub async fn execute(
    client: &GongClient,
    options: &SyncOptions,
    plan: SyncPlan,
) -> Result<SyncReport> {
    let mut report = SyncReport::default();
    let progress = if options.full && !options.quiet {
        let transcript_count = plan
            .operations
            .iter()
            .filter(|operation| !matches!(operation.action, Action::Skip))
            .count();
        let transcript_batches = transcript_count.div_ceil(100);
        let progress = ProgressBar::new((plan.operations.len() + transcript_batches) as u64);
        progress.set_style(
            ProgressStyle::with_template("{spinner:.green} {pos}/{len} {msg}")
                .expect("static progress template is valid"),
        );
        progress.set_message("fetching batched Transcripts");
        Some(progress)
    } else {
        None
    };
    let mut batched_transcripts = if options.full {
        fetch_batched_transcripts(client, &plan.operations, progress.as_ref()).await
    } else {
        HashMap::new()
    };
    if let Some(progress) = &progress {
        progress.set_message("rendering Call Files");
    }

    for operation in plan.operations {
        let PlannedOperation {
            call,
            target,
            action,
        } = operation;
        if matches!(action, Action::Skip) {
            report.skipped += 1;
            if !options.quiet {
                eprintln!("skipped {} {}", call.metadata.id, call.metadata.title);
            }
            if let Some(progress) = &progress {
                progress.inc(1);
            }
            continue;
        }

        let call_id = call.metadata.id.clone();
        let title = call.metadata.title.clone();
        let result = if options.full {
            match batched_transcripts.remove(&call_id) {
                Some(Ok(transcript)) => process_with_transcript(call, transcript, &target),
                Some(Err(error)) => Err(anyhow::anyhow!(error)),
                None => Err(anyhow::anyhow!(
                    "Gong returned no Transcript for Call {call_id}; it may not be ready yet"
                )),
            }
        } else {
            process_call(client, call, &target).await
        };
        match result {
            Ok(()) => {
                record_action(&mut report, action);
                if !options.quiet {
                    eprintln!(
                        "{} {} {} -> {}",
                        action_name(action),
                        call_id,
                        title,
                        target.display()
                    );
                }
            }
            Err(error) => {
                report.failed += 1;
                eprintln!("failed Call {call_id} {title}: {error:#}");
            }
        }
        if let Some(progress) = &progress {
            progress.inc(1);
        }
    }
    if let Some(progress) = progress {
        progress.finish_and_clear();
    }
    Ok(report)
}

async fn fetch_batched_transcripts(
    client: &GongClient,
    operations: &[PlannedOperation],
    progress: Option<&ProgressBar>,
) -> HashMap<String, std::result::Result<CallTranscript, String>> {
    let call_ids: Vec<String> = operations
        .iter()
        .filter(|operation| !matches!(operation.action, Action::Skip))
        .map(|operation| operation.call.metadata.id.clone())
        .collect();
    let mut results = HashMap::new();
    for chunk in call_ids.chunks(100) {
        match client.get_transcripts(chunk).await {
            Ok(transcripts) => {
                for transcript in transcripts {
                    results.insert(transcript.call_id.clone(), Ok(transcript));
                }
            }
            Err(batch_error) => {
                tracing::warn!(
                    error = %batch_error,
                    calls = chunk.len(),
                    "batched Transcript request failed; retrying Calls individually"
                );
                for call_id in chunk {
                    let result = client
                        .get_transcript(call_id)
                        .await
                        .map_err(|error| error.to_string());
                    results.insert(call_id.clone(), result);
                }
            }
        }
        if let Some(progress) = progress {
            progress.inc(1);
        }
    }
    results
}

fn sync_window(
    options: &SyncOptions,
) -> Result<(DateTime<Utc>, DateTime<Utc>, NaiveDate, NaiveDate)> {
    let now = Utc::now();
    let to_date = options.to.unwrap_or_else(|| now.date_naive());
    let to = if options.to.is_some() {
        start_of_day(
            to_date
                .checked_add_days(Days::new(1))
                .context("--to date is too large")?,
        )?
    } else {
        now
    };
    let from_date = if let Some(from) = options.from {
        from
    } else {
        let high_water = high_water_mark(&options.output_dir)?.with_context(
            || "Output Directory has no Call Files; pass --from YYYY-MM-DD or run sync --full",
        )?;
        high_water
            .checked_sub_days(Days::new(options.overlap_days))
            .context("Overlap Window underflowed the supported date range")?
    };
    let from = start_of_day(from_date)?;
    if from >= to {
        bail!("sync window is empty; --from must be earlier than the end of --to");
    }
    Ok((from, to, from_date, to_date))
}

fn start_of_day(date: NaiveDate) -> Result<DateTime<Utc>> {
    date.and_hms_opt(0, 0, 0)
        .map(|value| value.and_utc())
        .context("date cannot be represented at midnight")
}

fn high_water_mark(output_dir: &Path) -> Result<Option<NaiveDate>> {
    let mut newest = None;
    for entry in std::fs::read_dir(output_dir)
        .with_context(|| format!("cannot scan Output Directory {}", output_dir.display()))?
    {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        if let Some(date) = date_from_filename(&entry.file_name().to_string_lossy()) {
            newest = Some(newest.map_or(date, |current: NaiveDate| current.max(date)));
        }
    }
    Ok(newest)
}

fn scan_existing(
    output_dir: &Path,
    range: Option<(NaiveDate, NaiveDate)>,
) -> Result<ExistingFiles> {
    let mut result = ExistingFiles {
        by_id: HashMap::new(),
        by_filename: HashMap::new(),
        incomplete: HashSet::new(),
        all_paths: HashSet::new(),
    };
    for entry in std::fs::read_dir(output_dir)
        .with_context(|| format!("cannot scan Output Directory {}", output_dir.display()))?
    {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let filename = entry.file_name().to_string_lossy().into_owned();
        let Some(date) = date_from_filename(&filename) else {
            continue;
        };
        if let Some((from, to)) = range
            && (date < from || date > to)
        {
            continue;
        }
        let path = entry.path();
        result.all_paths.insert(path.clone());
        let contents = std::fs::read_to_string(&path)
            .with_context(|| format!("cannot read existing Call File {}", path.display()))?;
        if let Some(call_id) = frontmatter_call_id(&contents) {
            result.by_id.entry(call_id).or_insert_with(|| path.clone());
        }
        if contents.contains(PLACEHOLDER) {
            result.incomplete.insert(path.clone());
        }
        result.by_filename.insert(filename, path);
    }
    Ok(result)
}

fn date_from_filename(filename: &str) -> Option<NaiveDate> {
    if filename.len() < 14 || &filename[10..13] != " - " || !filename.ends_with(".md") {
        return None;
    }
    NaiveDate::parse_from_str(&filename[..10], "%Y-%m-%d").ok()
}

fn frontmatter_call_id(contents: &str) -> Option<String> {
    let mut delimiters = 0;
    for line in contents.lines() {
        if line == "---" {
            delimiters += 1;
            if delimiters == 2 {
                break;
            }
            continue;
        }
        if delimiters == 1
            && let Some(value) = line.strip_prefix("gong_call_id:")
        {
            return Some(value.trim().trim_matches('"').to_owned());
        }
    }
    None
}

fn plan_filenames(mut calls: Vec<ExtensiveCall>) -> Result<Vec<PlannedCall>> {
    for call in &calls {
        canonical_filename(call)?;
    }
    calls.sort_by(|left, right| {
        parsed_started(left)
            .cmp(&parsed_started(right))
            .then_with(|| left.metadata.id.cmp(&right.metadata.id))
    });
    let mut used = HashSet::new();
    calls
        .into_iter()
        .map(|call| {
            let base = canonical_filename(&call)?;
            let filename = reserve_filename(&base, &call, &mut used)?;
            Ok(PlannedCall { call, filename })
        })
        .collect()
}

fn parsed_started(call: &ExtensiveCall) -> DateTime<chrono::FixedOffset> {
    DateTime::parse_from_rfc3339(&call.metadata.started)
        .expect("canonical_filename already validates started timestamps")
}

fn add_start_suffix(base: &str, call: &ExtensiveCall) -> Result<String> {
    let started = DateTime::parse_from_rfc3339(&call.metadata.started)
        .with_context(|| format!("Call {} has invalid started timestamp", call.metadata.id))?;
    let stem = base.strip_suffix(".md").unwrap_or(base);
    Ok(format!("{} ({}).md", stem, started.format("%H-%M")))
}

fn filename_collision_key(filename: &str) -> String {
    filename.nfd().flat_map(char::to_lowercase).collect()
}

fn reserve_filename(
    base: &str,
    call: &ExtensiveCall,
    used: &mut HashSet<String>,
) -> Result<String> {
    if used.insert(filename_collision_key(base)) {
        return Ok(base.to_owned());
    }

    let with_start = add_start_suffix(base, call)?;
    if used.insert(filename_collision_key(&with_start)) {
        return Ok(with_start);
    }

    let stem = with_start.strip_suffix(".md").unwrap_or(&with_start);
    let with_id = format!("{stem}-{}.md", call.metadata.id);
    if used.insert(filename_collision_key(&with_id)) {
        return Ok(with_id);
    }

    for sequence in 2_u64.. {
        let candidate = format!("{stem}-{}-{sequence}.md", call.metadata.id);
        if used.insert(filename_collision_key(&candidate)) {
            return Ok(candidate);
        }
    }
    unreachable!("an unbounded numeric suffix always yields a unique filename")
}

async fn process_call(client: &GongClient, call: ExtensiveCall, target: &Path) -> Result<()> {
    let call_id = call.metadata.id.clone();
    let title = call.metadata.title.clone();
    let transcript = client
        .get_transcript(&call_id)
        .await
        .with_context(|| format!("could not fetch Transcript for {call_id} {title}"))?;
    process_with_transcript(call, transcript, target)
}

fn process_with_transcript(
    call: ExtensiveCall,
    transcript: CallTranscript,
    target: &Path,
) -> Result<()> {
    let call_id = call.metadata.id.clone();
    let title = call.metadata.title.clone();
    let merged = MergedCall { call, transcript };
    let rendered = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| render_call(&merged)))
        .map_err(|_| anyhow::anyhow!("renderer panicked for {call_id} {title}"))??;
    atomic_write(target, rendered.markdown.as_bytes())
}

fn atomic_write(path: &Path, contents: &[u8]) -> Result<()> {
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .context("target Call File has no UTF-8 filename")?;
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temporary = path.with_file_name(format!(
        ".{filename}.gong-tmp-{}-{sequence}",
        std::process::id()
    ));
    let result = (|| -> Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .with_context(|| format!("cannot create temporary file {}", temporary.display()))?;
        file.write_all(contents)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temporary, path)
            .with_context(|| format!("cannot atomically replace {}", path.display()))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

fn read_status_values(path: &Path) -> Result<HashMap<String, String>> {
    if !path.exists() {
        return Ok(HashMap::new());
    }
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read sync status file {}", path.display()))?;
    Ok(contents
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect())
}

fn timestamp() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn record_action(report: &mut SyncReport, action: Action) {
    match action {
        Action::New => report.new += 1,
        Action::Heal | Action::Overwrite => report.healed += 1,
        Action::Skip => report.skipped += 1,
    }
}

fn action_name(action: Action) -> &'static str {
    match action {
        Action::New => "new",
        Action::Heal => "healed",
        Action::Overwrite => "overwrote",
        Action::Skip => "skipped",
    }
}
