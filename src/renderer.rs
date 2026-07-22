use crate::api::{CallTranscript, ExtensiveCall, OutlineSection, Party};
use chrono::DateTime;
use serde::Serialize;
use std::{collections::HashMap, fmt::Write};
use thiserror::Error;

pub const PLACEHOLDER: &str = "_[Summary to be added]_";

#[derive(Debug, Clone, Serialize)]
pub struct MergedCall {
    pub call: ExtensiveCall,
    pub transcript: CallTranscript,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedCall {
    pub filename: String,
    pub markdown: String,
}

#[derive(Debug, Error)]
pub enum RenderError {
    #[error("Call {call_id} has invalid started timestamp {started:?}")]
    InvalidStarted { call_id: String, started: String },
    #[error("cannot merge Call {call_id} with transcript for {transcript_call_id}")]
    TranscriptMismatch {
        call_id: String,
        transcript_call_id: String,
    },
}

#[derive(Debug)]
struct Turn {
    speaker_id: String,
    speaker_name: String,
    start_ms: u64,
    text: String,
}

pub fn render_call(merged: &MergedCall) -> Result<RenderedCall, RenderError> {
    let call = &merged.call;
    if call.metadata.id != merged.transcript.call_id {
        return Err(RenderError::TranscriptMismatch {
            call_id: call.metadata.id.clone(),
            transcript_call_id: merged.transcript.call_id.clone(),
        });
    }
    let filename = canonical_filename(call)?;
    let date = filename[..10].to_owned();
    let turns = build_turns(call, &merged.transcript);
    let mut markdown = String::new();
    emit_frontmatter(&mut markdown, call, &date);
    emit_body(&mut markdown, call, &turns);

    Ok(RenderedCall { filename, markdown })
}

pub fn canonical_filename(call: &ExtensiveCall) -> Result<String, RenderError> {
    let started = DateTime::parse_from_rfc3339(&call.metadata.started).map_err(|_| {
        RenderError::InvalidStarted {
            call_id: call.metadata.id.clone(),
            started: call.metadata.started.clone(),
        }
    })?;
    Ok(format!(
        "{} - {}.md",
        started.format("%Y-%m-%d"),
        sanitize_filename_title(&call.metadata.title)
    ))
}

pub fn sanitize_filename_title(title: &str) -> String {
    let replaced: String = title
        .chars()
        .map(|character| match character {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '-',
            other => other,
        })
        .collect();
    replaced.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn emit_frontmatter(output: &mut String, call: &ExtensiveCall, date: &str) {
    writeln!(output, "---").unwrap();
    yaml_string(output, "title", &call.metadata.title);
    if let Some(account) = call.account_name() {
        yaml_string(output, "account", account);
    }
    yaml_string(output, "date", date);
    yaml_string(output, "started", &call.metadata.started);
    writeln!(output, "duration_minutes: {}", call.metadata.duration / 60).unwrap();
    yaml_string(
        output,
        "meeting_provider",
        call.metadata.system.as_deref().unwrap_or_default(),
    );
    if let Some(host) = host_name(call) {
        yaml_string(output, "host", host);
    }
    yaml_string(output, "gong_call_id", &call.metadata.id);
    yaml_string(
        output,
        "gong_url",
        call.metadata.url.as_deref().unwrap_or_default(),
    );
    yaml_string(output, "document_type", "meeting-notes");
    yaml_string(output, "source_type", "gong-transcript");

    let mut customer_contacts: Vec<&Party> = call
        .parties
        .iter()
        .filter(|party| party.affiliation != "Internal")
        .collect();
    let mut internal_attendees: Vec<&Party> = call
        .parties
        .iter()
        .filter(|party| party.affiliation == "Internal")
        .collect();
    sort_parties(&mut customer_contacts);
    sort_parties(&mut internal_attendees);
    yaml_parties(output, "customer_contacts", &customer_contacts);
    yaml_parties(output, "internal_attendees", &internal_attendees);
    writeln!(output, "---\n").unwrap();
}

fn emit_body(output: &mut String, call: &ExtensiveCall, turns: &[Turn]) {
    writeln!(output, "# {}\n", call.metadata.title).unwrap();
    writeln!(output, "## Summary\n").unwrap();
    let brief = call
        .content
        .as_ref()
        .and_then(|content| content.brief.as_deref())
        .map(str::trim)
        .filter(|brief| !brief.is_empty());
    writeln!(output, "{}\n", brief.unwrap_or(PLACEHOLDER)).unwrap();

    writeln!(output, "### Key points\n").unwrap();
    let key_points = call
        .content
        .as_ref()
        .and_then(|content| content.key_points.as_deref())
        .unwrap_or_default();
    emit_bullets(output, key_points.iter().map(|item| item.text.as_str()));

    writeln!(output, "### Next steps\n").unwrap();
    let next_steps = call
        .content
        .as_ref()
        .and_then(|content| content.highlights.as_deref())
        .and_then(|highlights| {
            highlights
                .iter()
                .find(|highlight| highlight.title.eq_ignore_ascii_case("Next steps"))
        })
        .map(|highlight| highlight.items.as_slice())
        .unwrap_or_default();
    emit_bullets(output, next_steps.iter().map(|item| item.text.as_str()));

    let mut outline: Vec<&OutlineSection> = call
        .content
        .as_ref()
        .and_then(|content| content.outline.as_deref())
        .unwrap_or_default()
        .iter()
        .filter(|section| section.start_time.is_some())
        .collect();
    outline.sort_by(|left, right| {
        left.start_time
            .unwrap()
            .total_cmp(&right.start_time.unwrap())
    });

    if !outline.is_empty() {
        writeln!(output, "## Outline\n").unwrap();
        for section in &outline {
            writeln!(
                output,
                "- [{}] {}",
                format_seconds(section.start_time.unwrap()),
                section.section
            )
            .unwrap();
        }
        writeln!(output).unwrap();
    }

    writeln!(output, "## Transcript\n").unwrap();
    if outline.is_empty() {
        emit_turns(output, turns.iter());
        return;
    }

    let mut section_turns: Vec<Vec<&Turn>> = vec![Vec::new(); outline.len()];
    for turn in turns {
        let seconds = turn.start_ms as f64 / 1000.0;
        let section_index = outline
            .iter()
            .rposition(|section| section.start_time.unwrap() <= seconds)
            .unwrap_or(0);
        section_turns[section_index].push(turn);
    }
    for (section, turns) in outline.into_iter().zip(section_turns) {
        writeln!(
            output,
            "### {} [{}]\n",
            section.section,
            format_seconds(section.start_time.unwrap())
        )
        .unwrap();
        emit_turns(output, turns.into_iter());
    }
}

fn build_turns(call: &ExtensiveCall, transcript: &CallTranscript) -> Vec<Turn> {
    let names: HashMap<&str, &str> = call
        .parties
        .iter()
        .filter_map(|party| Some((party.speaker_id.as_deref()?, party.name.as_deref()?)))
        .collect();
    let mut turns: Vec<Turn> = Vec::new();

    for entry in &transcript.transcript {
        let Some(first_sentence) = entry.sentences.first() else {
            continue;
        };
        let text = entry
            .sentences
            .iter()
            .map(|sentence| sentence.text.trim())
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        if text.is_empty() {
            continue;
        }
        if let Some(previous) = turns.last_mut()
            && previous.speaker_id == entry.speaker_id
        {
            previous.text.push(' ');
            previous.text.push_str(&text);
            continue;
        }
        turns.push(Turn {
            speaker_id: entry.speaker_id.clone(),
            speaker_name: names
                .get(entry.speaker_id.as_str())
                .copied()
                .unwrap_or("Unknown")
                .to_owned(),
            start_ms: first_sentence.start,
            text,
        });
    }
    turns
}

fn emit_bullets<'a>(output: &mut String, items: impl Iterator<Item = &'a str>) {
    let items: Vec<&str> = items
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .collect();
    if items.is_empty() {
        writeln!(output, "{}\n", PLACEHOLDER).unwrap();
    } else {
        for item in items {
            writeln!(output, "- {item}").unwrap();
        }
        writeln!(output).unwrap();
    }
}

fn emit_turns<'a>(output: &mut String, turns: impl Iterator<Item = &'a Turn>) {
    for turn in turns {
        writeln!(
            output,
            "{} | {}\n{}\n",
            format_milliseconds(turn.start_ms),
            turn.speaker_name,
            turn.text
        )
        .unwrap();
    }
}

fn yaml_string(output: &mut String, key: &str, value: &str) {
    writeln!(output, "{key}: \"{}\"", escape_yaml(value)).unwrap();
}

fn yaml_parties(output: &mut String, key: &str, parties: &[&Party]) {
    if parties.is_empty() {
        writeln!(output, "{key}: []").unwrap();
        return;
    }
    writeln!(output, "{key}:").unwrap();
    for party in parties {
        let mut fields = Vec::new();
        if let Some(name) = party.name.as_deref() {
            fields.push(("name", name));
        }
        if let Some(title) = party.title.as_deref() {
            fields.push(("title", title));
        }
        if let Some(email) = party.email_address.as_deref() {
            fields.push(("email", email));
        }
        let Some((first_key, first_value)) = fields.first() else {
            writeln!(output, "  - {{}}").unwrap();
            continue;
        };
        writeln!(output, "  - {first_key}: \"{}\"", escape_yaml(first_value)).unwrap();
        for (field, value) in fields.iter().skip(1) {
            writeln!(output, "    {field}: \"{}\"", escape_yaml(value)).unwrap();
        }
    }
}

fn sort_parties(parties: &mut [&Party]) {
    parties.sort_by(|left, right| {
        left.name
            .as_deref()
            .unwrap_or_default()
            .cmp(right.name.as_deref().unwrap_or_default())
            .then_with(|| {
                left.email_address
                    .as_deref()
                    .unwrap_or_default()
                    .cmp(right.email_address.as_deref().unwrap_or_default())
            })
    });
}

fn host_name(call: &ExtensiveCall) -> Option<&str> {
    let primary_user_id = call.metadata.primary_user_id.as_deref()?;
    call.parties
        .iter()
        .find(|party| party.user_id.as_deref() == Some(primary_user_id))?
        .name
        .as_deref()
}

fn escape_yaml(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

fn format_milliseconds(milliseconds: u64) -> String {
    let seconds = milliseconds / 1000;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

fn format_seconds(seconds: f64) -> String {
    let seconds = seconds.max(0.0).floor() as u64;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}
