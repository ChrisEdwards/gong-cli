use gong_cli::{
    api::{
        CallTranscript, ExtensiveCall, ExtensiveResponse, Sentence, TranscriptEntry,
        TranscriptResponse,
    },
    renderer::{MergedCall, PLACEHOLDER, render_call, sanitize_filename_title},
};
use regex::Regex;

const REFERENCE_CALL_ID: &str = "1860496513693944597";

fn reference_call() -> MergedCall {
    let extensive: ExtensiveResponse =
        serde_json::from_str(include_str!("fixtures/extensive_response.json")).unwrap();
    let transcripts: TranscriptResponse =
        serde_json::from_str(include_str!("fixtures/transcript_response.json")).unwrap();
    let call = extensive
        .calls
        .into_iter()
        .find(|call| call.metadata.id == REFERENCE_CALL_ID)
        .unwrap();
    let transcript = transcripts
        .call_transcripts
        .into_iter()
        .find(|transcript| transcript.call_id == REFERENCE_CALL_ID)
        .unwrap();
    MergedCall { call, transcript }
}

fn retention_calls() -> Vec<ExtensiveCall> {
    let response: ExtensiveResponse =
        serde_json::from_str(include_str!("fixtures/retention_response.json")).unwrap();
    response.calls
}

fn without_transcript(call: ExtensiveCall) -> MergedCall {
    MergedCall {
        transcript: CallTranscript {
            call_id: call.metadata.id.clone(),
            transcript: Vec::new(),
            extra: Default::default(),
        },
        call,
    }
}

#[test]
fn reference_call_renders_deterministically_for_retrieval_and_existing_consumers() {
    let merged = reference_call();
    let first = render_call(&merged).unwrap();
    let second = render_call(&merged).unwrap();

    assert_eq!(first, second);
    assert!(first.filename.starts_with("2026-05-19 - Fixture_"));
    assert!(!first.filename[13..].contains('/'));
    assert!(first.markdown.starts_with("---\ntitle: "));
    assert_eq!(first.markdown.matches("\n# ").count(), 1);
    assert!(first.markdown.contains("\n## Summary\n"));
    assert!(first.markdown.contains("\n## Outline\n"));
    assert!(first.markdown.contains("\n## Transcript\n"));
    assert!(first.markdown.contains("# Transcript"));
    assert!(first.markdown.contains("\nduration_minutes: "));
    let turn_pattern = Regex::new(r"(?m)^\d+:\d+\s*\|\s*\w+").unwrap();
    assert!(turn_pattern.is_match(&first.markdown));
    assert!(!first.markdown.contains(PLACEHOLDER));
    assert!(!first.markdown.contains("Current_ARR__c"));
    assert!(!first.markdown.contains("V_"));
    insta::assert_snapshot!("reference_call", first.markdown);
}

#[test]
fn missing_spotlight_uses_the_exact_placeholder_in_each_affected_subsection() {
    let call = retention_calls()
        .into_iter()
        .find(|call| call.content.is_none())
        .unwrap();
    let rendered = render_call(&without_transcript(call)).unwrap();

    assert_eq!(rendered.markdown.matches(PLACEHOLDER).count(), 3);
    assert!(!rendered.markdown.contains("\n## Outline\n"));
    assert!(
        !rendered
            .markdown
            .split("## Transcript")
            .nth(1)
            .unwrap()
            .contains("### ")
    );
    insta::assert_snapshot!("missing_spotlight", rendered.markdown);
}

#[test]
fn missing_account_is_omitted_instead_of_leaking_other_crm_fields() {
    let call = retention_calls()
        .into_iter()
        .find(|call| call.account_name().is_none() && call.content.is_some())
        .unwrap();
    let rendered = render_call(&without_transcript(call)).unwrap();

    assert!(
        !rendered
            .markdown
            .lines()
            .any(|line| line.starts_with("account:"))
    );
    assert!(!rendered.markdown.contains("Current_ARR__c"));
    assert!(!rendered.markdown.contains("V_"));
    insta::assert_snapshot!("missing_account", rendered.markdown);
}

#[test]
fn unknown_phone_party_is_included_and_renders_as_unknown_speaker() {
    let call = retention_calls()
        .into_iter()
        .find(|call| {
            call.parties
                .iter()
                .filter(|party| {
                    party.affiliation == "Unknown"
                        && party.name.is_none()
                        && party.email_address.is_none()
                })
                .count()
                >= 5
        })
        .unwrap();
    let speaker_id = call
        .parties
        .iter()
        .find(|party| party.affiliation == "Unknown" && party.name.is_none())
        .unwrap()
        .speaker_id
        .clone()
        .unwrap();
    let transcript = CallTranscript {
        call_id: call.metadata.id.clone(),
        transcript: vec![TranscriptEntry {
            speaker_id,
            topic: Some("ignored topic".to_owned()),
            sentences: vec![Sentence {
                start: 754_123,
                end: 756_000,
                text: "Synthetic phone contribution.".to_owned(),
                extra: Default::default(),
            }],
            extra: Default::default(),
        }],
        extra: Default::default(),
    };
    let rendered = render_call(&MergedCall { call, transcript }).unwrap();

    assert!(
        rendered
            .markdown
            .contains("12:34 | Unknown\nSynthetic phone contribution.")
    );
    assert!(rendered.markdown.contains("customer_contacts:\n  - {}"));
    insta::assert_snapshot!("unknown_phone_party", rendered.markdown);
}

#[test]
fn party_order_does_not_change_rendered_bytes() {
    let merged = reference_call();
    let expected = render_call(&merged).unwrap();
    let mut reordered = merged.clone();
    reordered.call.parties.reverse();

    assert_eq!(render_call(&reordered).unwrap(), expected);
}

#[test]
fn filename_sanitization_replaces_every_hostile_character_and_collapses_whitespace() {
    assert_eq!(
        sanitize_filename_title("  A<>:\"/\\|?*  “Curly”\tTitle  "),
        "A--------- “Curly” Title"
    );
}
