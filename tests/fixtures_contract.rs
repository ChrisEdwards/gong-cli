use serde_json::Value;
use std::collections::{HashMap, HashSet};

const REFERENCE_CALL_ID: &str = "1860496513693944597";

fn fixture(name: &str) -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name);
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

fn call_documents() -> Vec<Value> {
    [
        "extensive_response.json",
        "retention_response.json",
        "january_response.json",
    ]
    .into_iter()
    .map(fixture)
    .collect()
}

#[test]
fn committed_fixtures_preserve_all_required_quirk_classes() {
    let documents = call_documents();
    let calls: Vec<&Value> = documents
        .iter()
        .flat_map(|document| document["calls"].as_array().unwrap())
        .collect();
    assert_eq!(calls.len(), 141);

    let parties: Vec<&Value> = calls
        .iter()
        .flat_map(|call| call["parties"].as_array().unwrap())
        .collect();
    assert!(
        parties
            .iter()
            .filter(|party| {
                party["affiliation"] == "Unknown"
                    && party["name"].is_null()
                    && party["emailAddress"].is_null()
            })
            .count()
            >= 5
    );
    assert!(
        parties
            .iter()
            .any(|party| { party["name"].is_null() && party["emailAddress"].as_str().is_some() })
    );
    assert!(
        parties
            .iter()
            .any(|party| { party["name"].as_str().is_some() && party["emailAddress"].is_null() })
    );

    let calls_without_spotlight = calls
        .iter()
        .filter(|call| {
            call["content"].is_null()
                || ["brief", "keyPoints", "highlights"]
                    .into_iter()
                    .all(|key| call["content"].get(key).is_none())
        })
        .count();
    assert!(calls_without_spotlight >= 2);

    let ids: Vec<&str> = calls
        .iter()
        .map(|call| call["metaData"]["id"].as_str().unwrap())
        .collect();
    assert!(ids.contains(&REFERENCE_CALL_ID));
    assert!(
        ids.iter()
            .all(|id| id.chars().all(|character| character.is_ascii_digit()))
    );

    let mut same_day_titles = HashMap::<(&str, &str), usize>::new();
    let mut title_dates = HashMap::<&str, HashSet<&str>>::new();
    for call in &calls {
        let metadata = &call["metaData"];
        let date = &metadata["started"].as_str().unwrap()[..10];
        let title = metadata["title"].as_str().unwrap();
        *same_day_titles.entry((date, title)).or_default() += 1;
        title_dates.entry(title).or_default().insert(date);
    }
    assert!(same_day_titles.values().any(|count| *count > 1));
    assert!(title_dates.values().any(|dates| dates.len() > 1));

    let titles = calls
        .iter()
        .map(|call| call["metaData"]["title"].as_str().unwrap())
        .collect::<String>();
    for character in ['<', '>', ':', '/', '|', '“'] {
        assert!(
            titles.contains(character),
            "missing hostile title character {character}"
        );
    }

    let reference = calls
        .iter()
        .find(|call| call["metaData"]["id"] == REFERENCE_CALL_ID)
        .unwrap();
    assert_eq!(
        reference["content"]["outline"].as_array().unwrap().len(),
        25
    );
    assert!(
        reference["content"]["outline"]
            .as_array()
            .unwrap()
            .iter()
            .all(|section| section["startTime"].is_number())
    );

    let account_objects: Vec<&Value> = calls
        .iter()
        .flat_map(|call| call["context"].as_array().into_iter().flatten())
        .flat_map(|context| context["objects"].as_array().into_iter().flatten())
        .filter(|object| object["objectType"] == "Account")
        .collect();
    assert!(!account_objects.is_empty());
    let field_names: HashSet<&str> = account_objects
        .iter()
        .flat_map(|object| object["fields"].as_array().unwrap())
        .filter_map(|field| field["name"].as_str())
        .collect();
    assert!(field_names.contains("Name"));
    assert!(field_names.iter().any(|name| name.contains("ARR")));
}

#[test]
fn fixture_identities_are_synthetic_and_transcript_joins_survive() {
    let documents = call_documents();
    let calls: Vec<&Value> = documents
        .iter()
        .flat_map(|document| document["calls"].as_array().unwrap())
        .collect();
    for party in calls
        .iter()
        .flat_map(|call| call["parties"].as_array().unwrap())
    {
        if let Some(email) = party["emailAddress"].as_str() {
            let domain = email.rsplit_once('@').unwrap().1;
            assert!(
                domain == "internal-example.com"
                    || (domain.starts_with("customer-") && domain.ends_with(".example")),
                "non-synthetic fixture email domain"
            );
        }
    }

    let reference = calls
        .iter()
        .find(|call| call["metaData"]["id"] == REFERENCE_CALL_ID)
        .unwrap();
    let party_speakers: HashSet<&str> = reference["parties"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|party| party["speakerId"].as_str())
        .collect();
    let transcript = fixture("transcript_response.json");
    assert_eq!(
        transcript["callTranscripts"][0]["callId"],
        REFERENCE_CALL_ID
    );
    assert!(
        transcript["callTranscripts"][0]["transcript"]
            .as_array()
            .unwrap()
            .iter()
            .all(|entry| party_speakers.contains(entry["speakerId"].as_str().unwrap()))
    );
}
