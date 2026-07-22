use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{header, method, path},
};

fn write_config(temp: &TempDir, base_url: &str) -> std::path::PathBuf {
    let output_dir = temp.path().join("calls");
    fs::create_dir(&output_dir).unwrap();
    let config_path = temp.path().join("config.toml");
    fs::write(
        &config_path,
        format!(
            "access_key = \"key-from-file\"\n\
             access_key_secret = \"secret-from-file\"\n\
             base_url = {base_url:?}\n\
             output_dir = {:?}\n",
            output_dir.display().to_string()
        ),
    )
    .unwrap();
    config_path
}

#[tokio::test(flavor = "multi_thread")]
async fn check_reports_each_successful_verification() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/calls/extensive"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "records": {"totalRecords": 0},
            "calls": []
        })))
        .mount(&server)
        .await;

    let temp = TempDir::new().unwrap();
    let config = write_config(&temp, &server.uri());

    Command::cargo_bin("gong")
        .unwrap()
        .args(["--config", config.to_str().unwrap(), "check"])
        .assert()
        .success()
        .stdout(predicate::str::contains("[PASS] config"))
        .stdout(predicate::str::contains("[PASS] credentials"))
        .stdout(predicate::str::contains("[PASS] output directory"));
}

#[tokio::test(flavor = "multi_thread")]
async fn check_surfaces_gong_error_text_for_invalid_credentials() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/calls/extensive"))
        .respond_with(ResponseTemplate::new(401).set_body_json(serde_json::json!({
            "errors": [{"message": "The supplied Gong credentials are invalid"}]
        })))
        .mount(&server)
        .await;

    let temp = TempDir::new().unwrap();
    let config = write_config(&temp, &server.uri());

    Command::cargo_bin("gong")
        .unwrap()
        .args(["--config", config.to_str().unwrap(), "check"])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains(
            "The supplied Gong credentials are invalid",
        ))
        .stderr(predicate::str::contains("access_key_secret"));
}

#[tokio::test(flavor = "multi_thread")]
async fn environment_overrides_every_file_setting() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/calls/extensive"))
        .and(header("authorization", "Basic ZW52LWtleTplbnYtc2VjcmV0"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "calls": []
        })))
        .mount(&server)
        .await;

    let temp = TempDir::new().unwrap();
    let config = write_config(&temp, "http://127.0.0.1:1");
    let file_output = temp.path().join("calls");
    fs::remove_dir(&file_output).unwrap();
    let env_output = temp.path().join("environment-output");
    fs::create_dir(&env_output).unwrap();

    Command::cargo_bin("gong")
        .unwrap()
        .args(["--config", config.to_str().unwrap(), "check"])
        .env("GONG_ACCESS_KEY", "env-key")
        .env("GONG_ACCESS_KEY_SECRET", "env-secret")
        .env("GONG_BASE_URL", server.uri())
        .env("GONG_OUTPUT_DIR", &env_output)
        .assert()
        .success()
        .stdout(predicate::str::contains(env_output.display().to_string()));
}

#[tokio::test(flavor = "multi_thread")]
async fn flags_override_every_environment_setting() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/calls/extensive"))
        .and(header(
            "authorization",
            "Basic ZmxhZy1rZXk6ZmxhZy1zZWNyZXQ=",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "calls": []
        })))
        .mount(&server)
        .await;

    let temp = TempDir::new().unwrap();
    let config = write_config(&temp, "http://127.0.0.1:1");
    let flag_output = temp.path().join("flag-output");
    fs::create_dir(&flag_output).unwrap();

    Command::cargo_bin("gong")
        .unwrap()
        .args([
            "--config",
            config.to_str().unwrap(),
            "--access-key",
            "flag-key",
            "--access-key-secret",
            "flag-secret",
            "--base-url",
            &server.uri(),
            "--output-dir",
            flag_output.to_str().unwrap(),
            "check",
        ])
        .env("GONG_ACCESS_KEY", "wrong-env-key")
        .env("GONG_ACCESS_KEY_SECRET", "wrong-env-secret")
        .env("GONG_BASE_URL", "http://127.0.0.1:2")
        .env("GONG_OUTPUT_DIR", temp.path().join("missing-env-output"))
        .assert()
        .success()
        .stdout(predicate::str::contains(flag_output.display().to_string()));
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn permissive_config_is_a_warning_not_a_failure() {
    use std::os::unix::fs::PermissionsExt;

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/calls/extensive"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "calls": []
        })))
        .mount(&server)
        .await;
    let temp = TempDir::new().unwrap();
    let config = write_config(&temp, &server.uri());
    fs::set_permissions(&config, fs::Permissions::from_mode(0o644)).unwrap();

    Command::cargo_bin("gong")
        .unwrap()
        .args(["--config", config.to_str().unwrap(), "check"])
        .assert()
        .success()
        .stdout(predicate::str::contains("[WARN] config permissions"))
        .stdout(predicate::str::contains("chmod 600"));
}

#[test]
fn missing_secret_names_the_setting_and_each_way_to_fix_it() {
    let temp = TempDir::new().unwrap();
    let output_dir = temp.path().join("calls");
    fs::create_dir(&output_dir).unwrap();
    let config = temp.path().join("config.toml");
    fs::write(
        &config,
        format!(
            "access_key = \"key\"\nbase_url = \"http://127.0.0.1:1\"\noutput_dir = {:?}\n",
            output_dir.display().to_string()
        ),
    )
    .unwrap();

    Command::cargo_bin("gong")
        .unwrap()
        .args(["--config", config.to_str().unwrap(), "check"])
        .env_remove("GONG_ACCESS_KEY_SECRET")
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains(
            "missing setting access_key_secret",
        ))
        .stderr(predicate::str::contains("GONG_ACCESS_KEY_SECRET"))
        .stderr(predicate::str::contains("--access-key-secret"));
}
