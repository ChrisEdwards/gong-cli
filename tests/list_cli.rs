use assert_cmd::Command;
use serde_json::{Value, json};
use std::{
    fs,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
use tempfile::TempDir;
use wiremock::{
    Match, Mock, MockServer, Request, ResponseTemplate,
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

fn extensive_fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/extensive_response.json")).unwrap()
}

fn response_with_calls(calls: Vec<Value>, cursor: Option<&str>) -> Value {
    let mut records = json!({
        "totalRecords": calls.len(),
        "currentPageSize": calls.len(),
        "currentPageNumber": 0
    });
    if let Some(cursor) = cursor {
        records["cursor"] = Value::String(cursor.to_owned());
    }
    json!({"records": records, "calls": calls})
}

#[tokio::test(flavor = "multi_thread")]
async fn list_json_emits_the_documented_shape_from_a_real_call() {
    let fixture = extensive_fixture();
    let call = fixture["calls"]
        .as_array()
        .unwrap()
        .iter()
        .find(|call| call["metaData"]["id"] == REFERENCE_CALL_ID)
        .unwrap()
        .clone();
    let expected_started = call["metaData"]["started"].as_str().unwrap().to_owned();
    let expected_duration = call["metaData"]["duration"].as_u64().unwrap() / 60;

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
            ResponseTemplate::new(200).set_body_json(response_with_calls(vec![call], None)),
        )
        .mount(&server)
        .await;
    let temp = TempDir::new().unwrap();
    let config = write_config(&temp, &server.uri());

    let output = Command::cargo_bin("gong")
        .unwrap()
        .args([
            "--config",
            config.to_str().unwrap(),
            "list",
            "--from",
            "2026-05-19",
            "--to",
            "2026-05-19",
            "--json",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let rows: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 1);
    assert_eq!(rows[0]["id"], REFERENCE_CALL_ID);
    assert_eq!(rows[0]["started"], expected_started);
    assert_eq!(rows[0]["duration_minutes"], expected_duration);
    assert!(rows[0]["title"].as_str().unwrap().starts_with("Fixture_"));
    assert!(rows[0]["account"].as_str().unwrap().starts_with("A_"));
    assert_eq!(rows[0].as_object().unwrap().len(), 5);
}

#[tokio::test(flavor = "multi_thread")]
async fn list_excludes_only_all_internal_calls_and_keeps_unknown_phone_parties() {
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
    let phone_call_id = phone_call["metaData"]["id"].as_str().unwrap().to_owned();
    let mut internal_call = fixture["calls"][0].clone();
    for party in internal_call["parties"].as_array_mut().unwrap() {
        party["affiliation"] = Value::String("Internal".to_owned());
    }
    let internal_call_id = internal_call["metaData"]["id"].as_str().unwrap().to_owned();

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/calls/extensive"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(response_with_calls(vec![internal_call, phone_call], None)),
        )
        .mount(&server)
        .await;
    let temp = TempDir::new().unwrap();
    let config = write_config(&temp, &server.uri());

    let output = Command::cargo_bin("gong")
        .unwrap()
        .args([
            "--config",
            config.to_str().unwrap(),
            "list",
            "--from",
            "2025-10-01",
            "--json",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let rows: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 1);
    assert_eq!(rows[0]["id"], phone_call_id);
    assert_ne!(rows[0]["id"], internal_call_id);
}

#[derive(Debug)]
struct CursorAtRequestRoot(Option<&'static str>);

impl Match for CursorAtRequestRoot {
    fn matches(&self, request: &Request) -> bool {
        let Ok(body) = serde_json::from_slice::<Value>(&request.body) else {
            return false;
        };
        let root_cursor = body.get("cursor").and_then(Value::as_str);
        root_cursor == self.0 && body["filter"].get("cursor").is_none()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn list_follows_the_official_root_level_pagination_cursor() {
    let calls = extensive_fixture()["calls"].as_array().unwrap().clone();
    let first_id = calls[0]["metaData"]["id"].clone();
    let second_id = calls[1]["metaData"]["id"].clone();
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v2/calls/extensive"))
        .and(CursorAtRequestRoot(None))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(response_with_calls(
                vec![calls[0].clone()],
                Some("next-page"),
            )),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v2/calls/extensive"))
        .and(CursorAtRequestRoot(Some("next-page")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(response_with_calls(vec![calls[1].clone()], None)),
        )
        .expect(1)
        .mount(&server)
        .await;

    let temp = TempDir::new().unwrap();
    let config = write_config(&temp, &server.uri());
    let started = std::time::Instant::now();
    let output = Command::cargo_bin("gong")
        .unwrap()
        .args([
            "--config",
            config.to_str().unwrap(),
            "list",
            "--from",
            "2026-05-19",
            "--json",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        started.elapsed() >= std::time::Duration::from_millis(350),
        "paginated requests were not paced below Gong's 3 rps limit"
    );
    let rows: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 2);
    assert_eq!(rows[0]["id"], first_id);
    assert_eq!(rows[1]["id"], second_id);
}

#[tokio::test(flavor = "multi_thread")]
async fn list_honors_retry_after_and_recovers_from_a_429() {
    let calls = extensive_fixture()["calls"].as_array().unwrap().clone();
    let body = response_with_calls(vec![calls[0].clone()], None);
    let requests = Arc::new(AtomicUsize::new(0));
    let responder_count = Arc::clone(&requests);
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v2/calls/extensive"))
        .respond_with(move |_request: &Request| {
            if responder_count.fetch_add(1, Ordering::SeqCst) == 0 {
                ResponseTemplate::new(429)
                    .insert_header("Retry-After", "0")
                    .set_body_json(json!({"errors": ["shared API limit reached"]}))
            } else {
                ResponseTemplate::new(200).set_body_json(&body)
            }
        })
        .expect(2)
        .mount(&server)
        .await;

    let temp = TempDir::new().unwrap();
    let config = write_config(&temp, &server.uri());
    Command::cargo_bin("gong")
        .unwrap()
        .args([
            "--config",
            config.to_str().unwrap(),
            "list",
            "--from",
            "2026-05-19",
            "--json",
        ])
        .assert()
        .success();
    assert_eq!(requests.load(Ordering::SeqCst), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn human_table_preserves_the_calls_own_date_time_and_full_id() {
    let calls = extensive_fixture()["calls"].as_array().unwrap().clone();
    let call = calls
        .into_iter()
        .find(|call| call["metaData"]["id"] == REFERENCE_CALL_ID)
        .unwrap();
    let started = call["metaData"]["started"].as_str().unwrap().to_owned();
    let expected_date = &started[..10];
    let expected_time = &started[11..16];
    assert!(started.contains('-') || started.ends_with('Z'));

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/calls/extensive"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(response_with_calls(vec![call], None)),
        )
        .mount(&server)
        .await;
    let temp = TempDir::new().unwrap();
    let config = write_config(&temp, &server.uri());

    Command::cargo_bin("gong")
        .unwrap()
        .args([
            "--config",
            config.to_str().unwrap(),
            "list",
            "--from",
            "2026-05-19",
        ])
        .assert()
        .success()
        .stdout(predicates::str::contains("DATE\tTIME\tID\tTITLE\tACCOUNT"))
        .stdout(predicates::str::contains(expected_date.to_owned()))
        .stdout(predicates::str::contains(expected_time.to_owned()))
        .stdout(predicates::str::contains(REFERENCE_CALL_ID));
}

#[test]
fn list_rejects_an_inverted_date_range_before_calling_gong() {
    let temp = TempDir::new().unwrap();
    let config = write_config(&temp, "http://127.0.0.1:1");

    Command::cargo_bin("gong")
        .unwrap()
        .args([
            "--config",
            config.to_str().unwrap(),
            "list",
            "--from",
            "2026-05-20",
            "--to",
            "2026-05-19",
        ])
        .assert()
        .failure()
        .stderr(predicates::str::contains(
            "--to must be the same as or later than --from",
        ));
}

#[tokio::test(flavor = "multi_thread")]
async fn list_can_run_from_environment_without_a_config_file() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/calls/extensive"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(response_with_calls(Vec::new(), None)),
        )
        .mount(&server)
        .await;
    let temp = TempDir::new().unwrap();
    let output_dir = temp.path().join("calls");
    fs::create_dir(&output_dir).unwrap();

    Command::cargo_bin("gong")
        .unwrap()
        .args(["list", "--from", "2026-05-19", "--json"])
        .env("HOME", temp.path())
        .env("GONG_ACCESS_KEY", "env-key")
        .env("GONG_ACCESS_KEY_SECRET", "env-secret")
        .env("GONG_BASE_URL", server.uri())
        .env("GONG_OUTPUT_DIR", output_dir)
        .assert()
        .success()
        .stdout("[]\n");
}
