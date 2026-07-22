use assert_cmd::Command;
use gong_cli::renderer::sanitize_filename_title;
use predicates::prelude::*;
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::{
    collections::{HashMap, HashSet},
    fs,
    process::Stdio,
    time::Duration,
};
use tempfile::TempDir;
use wiremock::{
    Mock, MockServer, Request, ResponseTemplate,
    matchers::{body_partial_json, method, path},
};

const REFERENCE_CALL_ID: &str = "1860496513693944597";

fn write_config(
    temp: &TempDir,
    base_url: &str,
    output_dir: &std::path::Path,
) -> std::path::PathBuf {
    let config_path = temp.path().join("config.toml");
    fs::write(
        &config_path,
        format!(
            "access_key = \"test-key\"\n\
             access_key_secret = \"test-secret\"\n\
             base_url = {base_url:?}\n\
             output_dir = {:?}\n\
             [sync]\n\
             overlap_days = 3\n",
            output_dir.display().to_string()
        ),
    )
    .unwrap();
    config_path
}

fn reference_call() -> Value {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/extensive_response.json")).unwrap();
    fixture["calls"]
        .as_array()
        .unwrap()
        .iter()
        .find(|call| call["metaData"]["id"] == REFERENCE_CALL_ID)
        .unwrap()
        .clone()
}

fn extensive_response(calls: Vec<Value>) -> Value {
    json!({
        "records": {
            "totalRecords": calls.len(),
            "currentPageSize": calls.len(),
            "currentPageNumber": 0
        },
        "calls": calls
    })
}

fn empty_transcript(call_id: &str) -> Value {
    json!({
        "callTranscripts": [{"callId": call_id, "transcript": []}]
    })
}

fn status_values(path: &std::path::Path) -> HashMap<String, String> {
    fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|line| {
            let (key, value) = line.split_once('=').unwrap();
            (key.to_owned(), value.to_owned())
        })
        .collect()
}

fn assert_rfc3339_utc(value: &str) {
    assert_eq!(value.len(), 20);
    assert!(value.ends_with('Z'));
    assert_eq!(&value[4..5], "-");
    assert_eq!(&value[10..11], "T");
}

async fn mount_transcript(server: &MockServer, call_id: &str, status: u16) {
    let mut response = ResponseTemplate::new(status);
    if status < 400 {
        response = response.set_body_json(empty_transcript(call_id));
    } else {
        response = response.set_body_json(json!({"errors": ["synthetic transcript failure"]}));
    }
    Mock::given(method("POST"))
        .and(path("/v2/calls/transcript"))
        .and(body_partial_json(json!({"filter": {"callIds": [call_id]}})))
        .respond_with(response)
        .expect(1)
        .mount(server)
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn sync_seeds_then_skips_without_changing_file_bytes() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/calls/extensive"))
        .and(body_partial_json(json!({
            "filter": {
                "fromDateTime": "2026-05-19T00:00:00Z",
                "toDateTime": "2026-05-20T00:00:00Z"
            }
        })))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(extensive_response(vec![reference_call()])),
        )
        .expect(2)
        .mount(&server)
        .await;
    let transcript: Value =
        serde_json::from_str(include_str!("fixtures/transcript_response.json")).unwrap();
    Mock::given(method("POST"))
        .and(path("/v2/calls/transcript"))
        .respond_with(ResponseTemplate::new(200).set_body_json(transcript))
        .expect(1)
        .mount(&server)
        .await;

    let temp = TempDir::new().unwrap();
    let output_dir = temp.path().join("calls");
    fs::create_dir(&output_dir).unwrap();
    let config = write_config(&temp, &server.uri(), &output_dir);

    Command::cargo_bin("gong")
        .unwrap()
        .args([
            "--config",
            config.to_str().unwrap(),
            "sync",
            "--from",
            "2026-05-19",
            "--to",
            "2026-05-19",
        ])
        .assert()
        .success()
        .stdout("synced 1 new, healed 0, skipped 0, failed 0\n");

    let files: Vec<_> = fs::read_dir(&output_dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(files.len(), 1);
    let first_bytes = fs::read(&files[0]).unwrap();
    assert!(
        String::from_utf8_lossy(&first_bytes)
            .contains(&format!("gong_call_id: \"{REFERENCE_CALL_ID}\""))
    );

    Command::cargo_bin("gong")
        .unwrap()
        .args([
            "--config",
            config.to_str().unwrap(),
            "sync",
            "--from",
            "2026-05-19",
            "--to",
            "2026-05-19",
            "--quiet",
        ])
        .assert()
        .success()
        .stdout("synced 0 new, healed 0, skipped 1, failed 0\n")
        .stderr(predicate::str::is_empty());

    assert_eq!(fs::read(&files[0]).unwrap(), first_bytes);
}

#[tokio::test(flavor = "multi_thread")]
async fn sync_heals_a_placeholder_then_skips_the_complete_file() {
    let complete_call = reference_call();
    let mut missing_spotlight = complete_call.clone();
    missing_spotlight["content"] = Value::Null;
    let listings = Arc::new(AtomicUsize::new(0));
    let listing_count = Arc::clone(&listings);
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/calls/extensive"))
        .respond_with(move |_request: &Request| {
            let call = if listing_count.fetch_add(1, Ordering::SeqCst) == 0 {
                missing_spotlight.clone()
            } else {
                complete_call.clone()
            };
            ResponseTemplate::new(200).set_body_json(extensive_response(vec![call]))
        })
        .expect(3)
        .mount(&server)
        .await;
    let transcript: Value =
        serde_json::from_str(include_str!("fixtures/transcript_response.json")).unwrap();
    Mock::given(method("POST"))
        .and(path("/v2/calls/transcript"))
        .respond_with(ResponseTemplate::new(200).set_body_json(transcript))
        .expect(2)
        .mount(&server)
        .await;

    let temp = TempDir::new().unwrap();
    let output_dir = temp.path().join("calls");
    fs::create_dir(&output_dir).unwrap();
    let config = write_config(&temp, &server.uri(), &output_dir);
    let args = [
        "--config",
        config.to_str().unwrap(),
        "sync",
        "--from",
        "2026-05-19",
        "--to",
        "2026-05-19",
    ];

    Command::cargo_bin("gong")
        .unwrap()
        .args(args)
        .assert()
        .success()
        .stdout("synced 1 new, healed 0, skipped 0, failed 0\n");
    let path = fs::read_dir(&output_dir)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let placeholder_bytes = fs::read(&path).unwrap();
    assert!(String::from_utf8_lossy(&placeholder_bytes).contains("_[Summary to be added]_"));

    Command::cargo_bin("gong")
        .unwrap()
        .args(args)
        .assert()
        .success()
        .stdout("synced 0 new, healed 1, skipped 0, failed 0\n");
    let healed_bytes = fs::read(&path).unwrap();
    assert_ne!(healed_bytes, placeholder_bytes);
    assert!(!String::from_utf8_lossy(&healed_bytes).contains("_[Summary to be added]_"));

    Command::cargo_bin("gong")
        .unwrap()
        .args(args)
        .assert()
        .success()
        .stdout("synced 0 new, healed 0, skipped 1, failed 0\n");
    assert_eq!(fs::read(path).unwrap(), healed_bytes);
}

#[tokio::test(flavor = "multi_thread")]
async fn sync_excludes_all_internal_but_writes_unknown_phone_parties() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/retention_response.json")).unwrap();
    let phone_call = fixture["calls"]
        .as_array()
        .unwrap()
        .iter()
        .find(|call| {
            call["parties"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|party| {
                    party["affiliation"] == "Unknown"
                        && party["name"].is_null()
                        && party["emailAddress"].is_null()
                })
                .count()
                >= 5
        })
        .unwrap()
        .clone();
    let phone_id = phone_call["metaData"]["id"].as_str().unwrap().to_owned();
    let mut internal = fixture["calls"][0].clone();
    for party in internal["parties"].as_array_mut().unwrap() {
        party["affiliation"] = Value::String("Internal".to_owned());
    }

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/calls/extensive"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(extensive_response(vec![internal, phone_call])),
        )
        .mount(&server)
        .await;
    mount_transcript(&server, &phone_id, 200).await;
    let temp = TempDir::new().unwrap();
    let output_dir = temp.path().join("calls");
    fs::create_dir(&output_dir).unwrap();
    let config = write_config(&temp, &server.uri(), &output_dir);

    Command::cargo_bin("gong")
        .unwrap()
        .args([
            "--config",
            config.to_str().unwrap(),
            "sync",
            "--from",
            "2025-09-29",
        ])
        .assert()
        .success()
        .stdout("synced 1 new, healed 0, skipped 0, failed 0\n");

    let files: Vec<_> = fs::read_dir(output_dir).unwrap().collect();
    assert_eq!(files.len(), 1);
    let markdown = fs::read_to_string(files[0].as_ref().unwrap().path()).unwrap();
    assert!(markdown.contains(&format!("gong_call_id: \"{phone_id}\"")));
    assert!(markdown.contains("customer_contacts:\n  - {}"));
}

#[tokio::test(flavor = "multi_thread")]
async fn sync_assigns_deterministic_start_time_suffixes_to_title_collisions() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/retention_response.json")).unwrap();
    let mut groups = std::collections::HashMap::<(&str, &str), Vec<Value>>::new();
    for call in fixture["calls"].as_array().unwrap() {
        let started = call["metaData"]["started"].as_str().unwrap();
        let title = call["metaData"]["title"].as_str().unwrap();
        groups
            .entry((&started[..10], title))
            .or_default()
            .push(call.clone());
    }
    let mut calls = groups.into_values().find(|calls| calls.len() == 2).unwrap();
    let complete_content = reference_call()["content"].clone();
    for call in &mut calls {
        call["content"] = complete_content.clone();
    }
    calls.sort_by(|left, right| {
        left["metaData"]["started"]
            .as_str()
            .cmp(&right["metaData"]["started"].as_str())
    });
    let date = calls[0]["metaData"]["started"].as_str().unwrap()[..10].to_owned();
    let title = calls[0]["metaData"]["title"].as_str().unwrap().to_owned();
    let later_time = calls[1]["metaData"]["started"].as_str().unwrap()[11..16].replace(':', "-");
    let ids: Vec<String> = calls
        .iter()
        .map(|call| call["metaData"]["id"].as_str().unwrap().to_owned())
        .collect();

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/calls/extensive"))
        .respond_with(ResponseTemplate::new(200).set_body_json(extensive_response(calls.clone())))
        .expect(2)
        .mount(&server)
        .await;
    for id in &ids {
        mount_transcript(&server, id, 200).await;
    }
    let temp = TempDir::new().unwrap();
    let output_dir = temp.path().join("calls");
    fs::create_dir(&output_dir).unwrap();
    let config = write_config(&temp, &server.uri(), &output_dir);
    let args = [
        "--config",
        config.to_str().unwrap(),
        "sync",
        "--from",
        &date,
        "--to",
        &date,
    ];

    Command::cargo_bin("gong")
        .unwrap()
        .args(args)
        .assert()
        .success()
        .stdout("synced 2 new, healed 0, skipped 0, failed 0\n");
    let base = format!("{} - {}.md", date, sanitize_filename_title(&title));
    let suffixed = format!(
        "{} - {} ({}).md",
        date,
        sanitize_filename_title(&title),
        later_time
    );
    assert!(output_dir.join(&base).is_file());
    assert!(output_dir.join(&suffixed).is_file());

    Command::cargo_bin("gong")
        .unwrap()
        .args(args)
        .assert()
        .success()
        .stdout("synced 0 new, healed 0, skipped 2, failed 0\n");
    let names: HashSet<_> = fs::read_dir(output_dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(names.len(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn sync_dry_run_previews_actions_without_transcripts_or_writes() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/calls/extensive"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(extensive_response(vec![reference_call()])),
        )
        .mount(&server)
        .await;
    let temp = TempDir::new().unwrap();
    let output_dir = temp.path().join("calls");
    fs::create_dir(&output_dir).unwrap();
    let config = write_config(&temp, &server.uri(), &output_dir);
    let status_file = temp.path().join("status");

    Command::cargo_bin("gong")
        .unwrap()
        .args([
            "--config",
            config.to_str().unwrap(),
            "sync",
            "--from",
            "2026-05-19",
            "--dry-run",
            "--status-file",
            status_file.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout("synced 1 new, healed 0, skipped 0, failed 0\n")
        .stderr(predicate::str::contains(format!(
            "would new {REFERENCE_CALL_ID}"
        )));
    assert_eq!(fs::read_dir(output_dir).unwrap().count(), 0);
    assert!(!status_file.exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn one_transcript_failure_is_partial_and_does_not_block_other_calls() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/extensive_response.json")).unwrap();
    let calls = fixture["calls"].as_array().unwrap()[..2].to_vec();
    let good_id = calls[0]["metaData"]["id"].as_str().unwrap().to_owned();
    let failed_id = calls[1]["metaData"]["id"].as_str().unwrap().to_owned();
    let failed_title = calls[1]["metaData"]["title"].as_str().unwrap().to_owned();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/calls/extensive"))
        .respond_with(ResponseTemplate::new(200).set_body_json(extensive_response(calls.clone())))
        .mount(&server)
        .await;
    mount_transcript(&server, &good_id, 200).await;
    mount_transcript(&server, &failed_id, 500).await;
    let temp = TempDir::new().unwrap();
    let output_dir = temp.path().join("calls");
    fs::create_dir(&output_dir).unwrap();
    let config = write_config(&temp, &server.uri(), &output_dir);
    let status_file = temp.path().join("status");
    let prior_success = "2026-07-01T12:00:00Z";
    fs::write(
        &status_file,
        format!("last_state=success\nlast_success_at={prior_success}\n"),
    )
    .unwrap();

    Command::cargo_bin("gong")
        .unwrap()
        .args([
            "--config",
            config.to_str().unwrap(),
            "sync",
            "--from",
            "2026-05-19",
            "--status-file",
            status_file.to_str().unwrap(),
        ])
        .assert()
        .code(2)
        .stdout("synced 1 new, healed 0, skipped 0, failed 1\n")
        .stderr(predicate::str::contains(&failed_id))
        .stderr(predicate::str::contains(&failed_title))
        .stderr(predicate::str::contains("synthetic transcript failure"));
    assert_eq!(fs::read_dir(output_dir).unwrap().count(), 1);
    let status = status_values(&status_file);
    assert_eq!(status["last_state"], "partial");
    assert_eq!(status["last_success_at"], prior_success);
    assert_rfc3339_utc(&status["last_run_at"]);
    assert_rfc3339_utc(&status["last_failure_at"]);
    assert_eq!(
        status["last_message"],
        "synced 1 new, healed 0, skipped 0, failed 1"
    );
    assert_eq!(status["new"], "1");
    assert_eq!(status["healed"], "0");
    assert_eq!(status["skipped"], "0");
    assert_eq!(status["failed"], "1");
}

#[test]
fn empty_output_directory_without_from_fails_before_network_with_a_hint() {
    let temp = TempDir::new().unwrap();
    let output_dir = temp.path().join("calls");
    fs::create_dir(&output_dir).unwrap();
    let config = write_config(&temp, "http://127.0.0.1:1", &output_dir);
    let status_file = temp.path().join("status");
    let prior_success = "2026-07-01T12:00:00Z";
    fs::write(&status_file, format!("last_success_at={prior_success}\n")).unwrap();

    Command::cargo_bin("gong")
        .unwrap()
        .args([
            "--config",
            config.to_str().unwrap(),
            "sync",
            "--status-file",
            status_file.to_str().unwrap(),
        ])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("pass --from YYYY-MM-DD"))
        .stderr(predicate::str::contains("sync --full"));
    let status = status_values(&status_file);
    assert_eq!(status["last_state"], "failure");
    assert_eq!(status["last_success_at"], prior_success);
    assert_rfc3339_utc(&status["last_failure_at"]);
    assert!(status["last_message"].contains("pass --from YYYY-MM-DD"));
    assert_eq!(status["new"], "0");
    assert_eq!(status["failed"], "0");
}

#[tokio::test(flavor = "multi_thread")]
async fn status_file_is_running_during_work_then_records_success_exactly() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/calls/extensive"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(Duration::from_millis(1_200))
                .set_body_json(extensive_response(Vec::new())),
        )
        .mount(&server)
        .await;
    let temp = TempDir::new().unwrap();
    let output_dir = temp.path().join("calls");
    fs::create_dir(&output_dir).unwrap();
    let config = write_config(&temp, &server.uri(), &output_dir);
    let status_file = temp.path().join("status");
    let child = std::process::Command::new(assert_cmd::cargo::cargo_bin("gong"))
        .args([
            "--config",
            config.to_str().unwrap(),
            "sync",
            "--from",
            "2026-05-19",
            "--status-file",
            status_file.to_str().unwrap(),
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let mut running = None;
    for _ in 0..100 {
        if status_file.is_file() {
            let values = status_values(&status_file);
            if values.get("last_state").map(String::as_str) == Some("running") {
                running = Some(values);
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let running = running.expect("status file never entered running state");
    assert_eq!(running["last_message"], "sync running");
    assert_eq!(running["new"], "0");
    assert_rfc3339_utc(&running["last_run_at"]);

    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "synced 0 new, healed 0, skipped 0, failed 0\n"
    );
    let success = status_values(&status_file);
    assert_eq!(success["last_state"], "success");
    assert_eq!(success["last_run_at"], running["last_run_at"]);
    assert_rfc3339_utc(&success["last_success_at"]);
    assert_eq!(success["last_failure_at"], "");
    assert_eq!(
        success["last_message"],
        "synced 0 new, healed 0, skipped 0, failed 0"
    );
    assert_eq!(success.len(), 9);
}

#[tokio::test(flavor = "multi_thread")]
async fn sync_derives_high_water_mark_and_overlap_from_call_filenames_only() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/calls/extensive"))
        .and(body_partial_json(json!({
            "filter": {
                "fromDateTime": "2026-05-16T00:00:00Z",
                "toDateTime": "2026-05-21T00:00:00Z"
            }
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(extensive_response(Vec::new())))
        .expect(1)
        .mount(&server)
        .await;
    let temp = TempDir::new().unwrap();
    let output_dir = temp.path().join("calls");
    fs::create_dir(&output_dir).unwrap();
    fs::write(
        output_dir.join("2026-05-19 - Existing.md"),
        "---\ngong_call_id: \"1234567890123456789\"\n---\n# Existing\n",
    )
    .unwrap();
    fs::write(output_dir.join("CLAUDE.md"), "not a Call File").unwrap();
    fs::create_dir(output_dir.join("2027-01-01 - directory.md")).unwrap();
    let config = write_config(&temp, &server.uri(), &output_dir);

    Command::cargo_bin("gong")
        .unwrap()
        .args([
            "--config",
            config.to_str().unwrap(),
            "sync",
            "--to",
            "2026-05-20",
        ])
        .assert()
        .success()
        .stdout("synced 0 new, healed 0, skipped 0, failed 0\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn sync_uses_filename_fallback_for_a_float_corrupted_legacy_id() {
    let call = reference_call();
    let started = call["metaData"]["started"].as_str().unwrap();
    let title = call["metaData"]["title"].as_str().unwrap();
    let filename = format!("{} - {}.md", &started[..10], sanitize_filename_title(title));
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/calls/extensive"))
        .respond_with(ResponseTemplate::new(200).set_body_json(extensive_response(vec![call])))
        .expect(1)
        .mount(&server)
        .await;
    let temp = TempDir::new().unwrap();
    let output_dir = temp.path().join("calls");
    fs::create_dir(&output_dir).unwrap();
    let path = output_dir.join(filename);
    let legacy = b"---\ngong_call_id: \"1860496513693944600\"\n---\n# Legacy complete file\n";
    fs::write(&path, legacy).unwrap();
    let config = write_config(&temp, &server.uri(), &output_dir);

    Command::cargo_bin("gong")
        .unwrap()
        .args([
            "--config",
            config.to_str().unwrap(),
            "sync",
            "--from",
            "2026-05-19",
            "--to",
            "2026-05-19",
        ])
        .assert()
        .success()
        .stdout("synced 0 new, healed 0, skipped 1, failed 0\n");
    assert_eq!(fs::read(path).unwrap(), legacy);
}

#[tokio::test(flavor = "multi_thread")]
async fn full_force_dry_run_reports_preflight_and_orphans_without_writing() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/extensive_response.json")).unwrap();
    let calls = fixture["calls"].as_array().unwrap()[..2].to_vec();
    let matched = &calls[0];
    let matched_id = matched["metaData"]["id"].as_str().unwrap();
    let matched_date = &matched["metaData"]["started"].as_str().unwrap()[..10];
    let matched_title = matched["metaData"]["title"].as_str().unwrap();
    let matched_filename = format!(
        "{} - {}.md",
        matched_date,
        sanitize_filename_title(matched_title)
    );
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/calls/extensive"))
        .respond_with(ResponseTemplate::new(200).set_body_json(extensive_response(calls.clone())))
        .expect(1)
        .mount(&server)
        .await;
    let temp = TempDir::new().unwrap();
    let output_dir = temp.path().join("calls");
    fs::create_dir(&output_dir).unwrap();
    let matched_path = output_dir.join(matched_filename);
    let original =
        format!("---\ngong_call_id: \"{matched_id}\"\n---\n# Existing complete Call File\n");
    fs::write(&matched_path, &original).unwrap();
    let orphan_path = output_dir.join("2020-01-01 - Orphan.md");
    fs::write(&orphan_path, "orphan bytes").unwrap();
    let config = write_config(&temp, &server.uri(), &output_dir);

    Command::cargo_bin("gong")
        .unwrap()
        .args([
            "--config",
            config.to_str().unwrap(),
            "sync",
            "--full",
            "--force",
            "--dry-run",
        ])
        .assert()
        .success()
        .stdout("synced 1 new, healed 1, skipped 0, failed 0\n")
        .stderr(predicate::str::contains(
            "preflight: overwrite 1, new 1, orphans 1",
        ))
        .stderr(predicate::str::contains(orphan_path.display().to_string()));
    assert_eq!(fs::read_to_string(matched_path).unwrap(), original);
    assert_eq!(fs::read_to_string(orphan_path).unwrap(), "orphan bytes");
    assert_eq!(fs::read_dir(output_dir).unwrap().count(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn full_force_disambiguates_a_case_equivalent_orphan_path() {
    let call = reference_call();
    let id = call["metaData"]["id"].as_str().unwrap().to_owned();
    let started = call["metaData"]["started"].as_str().unwrap().to_owned();
    let title = call["metaData"]["title"].as_str().unwrap();
    let canonical_filename = format!("{} - {}.md", &started[..10], sanitize_filename_title(title));
    let orphan_filename = canonical_filename.to_lowercase();
    assert_ne!(orphan_filename, canonical_filename);

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/calls/extensive"))
        .respond_with(ResponseTemplate::new(200).set_body_json(extensive_response(vec![call])))
        .expect(1)
        .mount(&server)
        .await;
    mount_transcript(&server, &id, 200).await;

    let temp = TempDir::new().unwrap();
    let output_dir = temp.path().join("calls");
    fs::create_dir(&output_dir).unwrap();
    let orphan_path = output_dir.join(orphan_filename);
    let orphan_bytes = b"legacy Call whose filename differs only by case";
    fs::write(&orphan_path, orphan_bytes).unwrap();
    let config = write_config(&temp, &server.uri(), &output_dir);

    Command::cargo_bin("gong")
        .unwrap()
        .args([
            "--config",
            config.to_str().unwrap(),
            "sync",
            "--full",
            "--force",
            "--yes",
        ])
        .assert()
        .success()
        .stdout("synced 1 new, healed 0, skipped 0, failed 0\n")
        .stderr(predicate::str::contains(
            "preflight: overwrite 0, new 1, orphans 1",
        ));

    let started = chrono::DateTime::parse_from_rfc3339(&started).unwrap();
    let stem = canonical_filename.strip_suffix(".md").unwrap();
    let disambiguated = output_dir.join(format!("{} ({}).md", stem, started.format("%H-%M")));
    assert_eq!(fs::read(&orphan_path).unwrap(), orphan_bytes);
    assert!(
        fs::read_to_string(disambiguated)
            .unwrap()
            .contains(&format!("gong_call_id: \"{id}\""))
    );
    assert_eq!(fs::read_dir(output_dir).unwrap().count(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn force_honors_negative_confirmation_and_yes_bypasses_it() {
    let call = reference_call();
    let id = call["metaData"]["id"].as_str().unwrap().to_owned();
    let started = call["metaData"]["started"].as_str().unwrap();
    let title = call["metaData"]["title"].as_str().unwrap();
    let filename = format!("{} - {}.md", &started[..10], sanitize_filename_title(title));
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/calls/extensive"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(extensive_response(vec![call.clone()])),
        )
        .expect(2)
        .mount(&server)
        .await;
    mount_transcript(&server, &id, 200).await;
    let temp = TempDir::new().unwrap();
    let output_dir = temp.path().join("calls");
    fs::create_dir(&output_dir).unwrap();
    let path = output_dir.join(filename);
    let original = format!("---\ngong_call_id: \"{id}\"\n---\n# Old bytes\n");
    fs::write(&path, &original).unwrap();
    let config = write_config(&temp, &server.uri(), &output_dir);
    let base_args = [
        "--config",
        config.to_str().unwrap(),
        "sync",
        "--from",
        "2026-05-19",
        "--to",
        "2026-05-19",
        "--force",
    ];

    Command::cargo_bin("gong")
        .unwrap()
        .args(base_args)
        .write_stdin("n\n")
        .assert()
        .success()
        .stdout("synced 0 new, healed 0, skipped 0, failed 0\n")
        .stderr(predicate::str::contains(
            "Proceed with forced Call File overwrites?",
        ))
        .stderr(predicate::str::contains("sync cancelled"));
    assert_eq!(fs::read_to_string(&path).unwrap(), original);

    Command::cargo_bin("gong")
        .unwrap()
        .args(base_args)
        .arg("--yes")
        .assert()
        .success()
        .stdout("synced 0 new, healed 1, skipped 0, failed 0\n")
        .stderr(predicate::str::contains(
            "preflight: overwrite 1, new 0, orphans 0",
        ))
        .stderr(predicate::str::contains("Proceed with forced").not());
    let rewritten = fs::read_to_string(path).unwrap();
    assert_ne!(rewritten, original);
    assert!(rewritten.contains(&format!("gong_call_id: \"{id}\"")));
}

fn directory_bytes(directory: &std::path::Path) -> std::collections::BTreeMap<String, Vec<u8>> {
    fs::read_dir(directory)
        .unwrap()
        .filter_map(|entry| {
            let entry = entry.unwrap();
            entry.file_type().unwrap().is_file().then(|| {
                (
                    entry.file_name().to_string_lossy().into_owned(),
                    fs::read(entry.path()).unwrap(),
                )
            })
        })
        .collect()
}

#[derive(Debug)]
struct NoDateFilter;

impl wiremock::Match for NoDateFilter {
    fn matches(&self, request: &Request) -> bool {
        let Ok(body) = serde_json::from_slice::<Value>(&request.body) else {
            return false;
        };
        body["filter"].get("fromDateTime").is_none() && body["filter"].get("toDateTime").is_none()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn full_force_batches_transcripts_reports_rename_orphan_and_converges() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/extensive_response.json")).unwrap();
    let mut calls = fixture["calls"].as_array().unwrap()[..2].to_vec();
    calls.sort_by(|left, right| {
        let left =
            chrono::DateTime::parse_from_rfc3339(left["metaData"]["started"].as_str().unwrap())
                .unwrap();
        let right =
            chrono::DateTime::parse_from_rfc3339(right["metaData"]["started"].as_str().unwrap())
                .unwrap();
        left.cmp(&right)
    });
    let ids: Vec<String> = calls
        .iter()
        .map(|call| call["metaData"]["id"].as_str().unwrap().to_owned())
        .collect();
    let transcript_response = json!({
        "callTranscripts": ids
            .iter()
            .map(|id| json!({"callId": id, "transcript": []}))
            .collect::<Vec<_>>()
    });
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/calls/extensive"))
        .and(NoDateFilter)
        .respond_with(ResponseTemplate::new(200).set_body_json(extensive_response(calls.clone())))
        .expect(2)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v2/calls/transcript"))
        .and(body_partial_json(json!({"filter": {"callIds": ids}})))
        .respond_with(ResponseTemplate::new(200).set_body_json(transcript_response))
        .expect(2)
        .mount(&server)
        .await;

    let temp = TempDir::new().unwrap();
    let output_dir = temp.path().join("calls");
    fs::create_dir(&output_dir).unwrap();
    let first = &calls[0];
    let first_path = output_dir.join(format!(
        "{} - {}.md",
        &first["metaData"]["started"].as_str().unwrap()[..10],
        sanitize_filename_title(first["metaData"]["title"].as_str().unwrap())
    ));
    fs::write(
        &first_path,
        format!(
            "---\ngong_call_id: \"{}\"\n---\n# Old first Call\n",
            first["metaData"]["id"].as_str().unwrap()
        ),
    )
    .unwrap();
    let renamed_orphan = output_dir.join("2026-05-19 - Old Renamed Title.md");
    let orphan_bytes = b"legacy renamed Call with no trustworthy id";
    fs::write(&renamed_orphan, orphan_bytes).unwrap();
    let config = write_config(&temp, &server.uri(), &output_dir);
    let args = [
        "--config",
        config.to_str().unwrap(),
        "sync",
        "--full",
        "--force",
        "--yes",
    ];

    Command::cargo_bin("gong")
        .unwrap()
        .args(args)
        .assert()
        .success()
        .stdout("synced 1 new, healed 1, skipped 0, failed 0\n")
        .stderr(predicate::str::contains(
            "preflight: overwrite 1, new 1, orphans 1",
        ))
        .stderr(predicate::str::contains(
            renamed_orphan.display().to_string(),
        ));
    let first_render = directory_bytes(&output_dir);
    assert_eq!(first_render.len(), 3);
    assert_eq!(fs::read(&renamed_orphan).unwrap(), orphan_bytes);

    Command::cargo_bin("gong")
        .unwrap()
        .args(args)
        .assert()
        .success()
        .stdout("synced 0 new, healed 2, skipped 0, failed 0\n")
        .stderr(predicate::str::contains(
            "preflight: overwrite 2, new 0, orphans 1",
        ));
    assert_eq!(directory_bytes(&output_dir), first_render);
    assert_eq!(fs::read(renamed_orphan).unwrap(), orphan_bytes);
}
