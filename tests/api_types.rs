use gong_cli::api::ExtensiveResponse;

#[test]
fn nineteen_digit_call_ids_remain_exact_strings() {
    let response: ExtensiveResponse =
        serde_json::from_str(
            r#"{"calls":[{"metaData":{"id":"1860496513693944597","title":"Fixture","started":"2026-01-01T00:00:00Z","duration":60}}]}"#,
        )
        .unwrap();

    assert_eq!(response.calls[0].metadata.id, "1860496513693944597");
    assert_ne!(response.calls[0].metadata.id, "1860496513693944600");
}
