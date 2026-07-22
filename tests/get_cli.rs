use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::{Value, json};
use std::fs;
use tempfile::TempDir;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_partial_json, method, path},
};

const REFERENCE_CALL_ID: &str = "1860496513693944597";

fn write_config(temp: &TempDir, base_url: &str) -> std::path::PathBuf {
    let output_dir = temp.path().join("calls");
    fs::create_dir(&output_dir).unwrap();
    let config_path = temp.path().join("config.toml");
    fs::write(
        &config_path,
        format!(
            "access_key = \"test-key\"\n\
             access_key_secret = \"test-secret\"\n\
             base_url = {base_url:?}\n\
             output_dir = {:?}\n",
            output_dir.display().to_string()
        ),
    )
    .unwrap();
    config_path
}

async fn mount_reference_call(server: &MockServer) {
    let extensive: Value =
        serde_json::from_str(include_str!("fixtures/extensive_response.json")).unwrap();
    let call = extensive["calls"]
        .as_array()
        .unwrap()
        .iter()
        .find(|call| call["metaData"]["id"] == REFERENCE_CALL_ID)
        .unwrap()
        .clone();
    Mock::given(method("POST"))
        .and(path("/v2/calls/extensive"))
        .and(body_partial_json(json!({
            "filter": {"callIds": [REFERENCE_CALL_ID]}
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "records": {"totalRecords": 1, "currentPageSize": 1, "currentPageNumber": 0},
            "calls": [call]
        })))
        .expect(1)
        .mount(server)
        .await;

    let transcript: Value =
        serde_json::from_str(include_str!("fixtures/transcript_response.json")).unwrap();
    Mock::given(method("POST"))
        .and(path("/v2/calls/transcript"))
        .and(body_partial_json(json!({
            "filter": {"callIds": [REFERENCE_CALL_ID]}
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(transcript))
        .expect(1)
        .mount(server)
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn get_fetches_merges_and_renders_one_call_to_stdout() {
    let server = MockServer::start().await;
    mount_reference_call(&server).await;
    let temp = TempDir::new().unwrap();
    let config = write_config(&temp, &server.uri());

    Command::cargo_bin("gong")
        .unwrap()
        .args([
            "--config",
            config.to_str().unwrap(),
            "get",
            REFERENCE_CALL_ID,
        ])
        .assert()
        .success()
        .stdout(predicate::str::starts_with("---\ntitle: "))
        .stdout(predicate::str::contains(format!(
            "gong_call_id: \"{REFERENCE_CALL_ID}\""
        )))
        .stdout(predicate::str::contains("\n## Outline\n"))
        .stdout(predicate::str::contains("\n## Transcript\n"));
}

#[tokio::test(flavor = "multi_thread")]
async fn get_output_writes_the_canonical_file_without_echoing_customer_data() {
    let server = MockServer::start().await;
    mount_reference_call(&server).await;
    let temp = TempDir::new().unwrap();
    let config = write_config(&temp, &server.uri());
    let output_path = temp.path().join("one-call.md");

    Command::cargo_bin("gong")
        .unwrap()
        .args([
            "--config",
            config.to_str().unwrap(),
            "get",
            REFERENCE_CALL_ID,
            "--output",
            output_path.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::is_empty());

    let markdown = fs::read_to_string(output_path).unwrap();
    assert!(markdown.starts_with("---\ntitle: "));
    assert!(markdown.contains(&format!("gong_call_id: \"{REFERENCE_CALL_ID}\"")));
    assert!(markdown.contains("\n## Transcript\n"));
}

#[tokio::test(flavor = "multi_thread")]
async fn get_json_preserves_unmodeled_fields_in_the_merged_debug_payload() {
    let server = MockServer::start().await;
    mount_reference_call(&server).await;
    let temp = TempDir::new().unwrap();
    let config = write_config(&temp, &server.uri());

    let output = Command::cargo_bin("gong")
        .unwrap()
        .args([
            "--config",
            config.to_str().unwrap(),
            "get",
            REFERENCE_CALL_ID,
            "--json",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let merged: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(merged["call"]["metaData"]["id"], REFERENCE_CALL_ID);
    assert_eq!(merged["transcript"]["callId"], REFERENCE_CALL_ID);
    assert!(merged["call"]["interaction"].is_object());
    assert!(merged["call"]["content"]["topics"].is_array());
    assert!(merged["call"]["metaData"]["direction"].is_string());
}

#[tokio::test(flavor = "multi_thread")]
async fn get_names_the_missing_call_and_does_not_request_a_transcript() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/calls/extensive"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "records": {"totalRecords": 0, "currentPageSize": 0, "currentPageNumber": 0},
            "calls": []
        })))
        .expect(1)
        .mount(&server)
        .await;
    let temp = TempDir::new().unwrap();
    let config = write_config(&temp, &server.uri());

    Command::cargo_bin("gong")
        .unwrap()
        .args([
            "--config",
            config.to_str().unwrap(),
            "get",
            REFERENCE_CALL_ID,
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(format!(
            "Gong returned no Call with id {REFERENCE_CALL_ID}"
        )));
}
