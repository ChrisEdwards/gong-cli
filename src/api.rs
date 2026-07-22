use chrono::{DateTime, Duration, SecondsFormat, Utc};
use reqwest::{Response, StatusCode, header::RETRY_AFTER};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Map, Value, json};
use std::{collections::HashSet, sync::Arc, time::Duration as StdDuration};
use thiserror::Error;
use tokio::{sync::Mutex, time::Instant};

fn deserialize_null_default<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(Option::<Vec<T>>::deserialize(deserializer)?.unwrap_or_default())
}

const REQUEST_INTERVAL: StdDuration = StdDuration::from_millis(400);
const MAX_429_RETRIES: u32 = 4;
const MAX_BACKOFF: StdDuration = StdDuration::from_secs(10);

#[derive(Debug, Clone)]
pub struct GongClient {
    http: reqwest::Client,
    base_url: String,
    access_key: String,
    access_key_secret: String,
    pacer: Arc<RequestPacer>,
}

#[derive(Debug)]
struct RequestPacer {
    next_request: Mutex<Instant>,
    interval: StdDuration,
}

#[derive(Debug, Error)]
pub enum ApiError {
    #[error("could not reach Gong: {0}")]
    Request(#[from] reqwest::Error),
    #[error("Gong returned HTTP {status}: {message}")]
    Http { status: StatusCode, message: String },
    #[error("Gong returned an invalid {endpoint} response: {source}")]
    InvalidResponse {
        endpoint: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("Gong repeated pagination cursor {0}; refusing an infinite pagination loop")]
    PaginationLoop(String),
    #[error("Gong returned no Call with id {0}")]
    CallNotFound(String),
    #[error("Gong returned no Transcript for Call {0}; it may not be ready yet")]
    TranscriptNotFound(String),
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ExtensiveResponse {
    #[serde(default)]
    pub records: Records,
    #[serde(default, deserialize_with = "deserialize_null_default")]
    pub calls: Vec<ExtensiveCall>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Records {
    pub total_records: usize,
    pub current_page_size: usize,
    pub current_page_number: usize,
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ExtensiveCall {
    #[serde(rename = "metaData")]
    pub metadata: CallMetadata,
    #[serde(default, deserialize_with = "deserialize_null_default")]
    pub parties: Vec<Party>,
    #[serde(default, deserialize_with = "deserialize_null_default")]
    pub context: Vec<CrmContext>,
    pub content: Option<CallContent>,
    #[serde(default, flatten)]
    pub extra: Map<String, Value>,
}

impl ExtensiveCall {
    pub fn is_customer_call(&self) -> bool {
        !self
            .parties
            .iter()
            .all(|party| party.affiliation == "Internal")
    }

    pub fn account_name(&self) -> Option<&str> {
        self.context
            .iter()
            .flat_map(|context| &context.objects)
            .find(|object| object.object_type == "Account")
            .and_then(|account| {
                account
                    .fields
                    .iter()
                    .find(|field| field.name == "Name")
                    .and_then(|field| field.value.as_str())
            })
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CallMetadata {
    // Gong IDs can exceed JavaScript's safe integer range and must remain strings.
    // The legacy exporter rounded ...944597 to ...944600 by passing one through f64.
    pub id: String,
    pub title: String,
    pub started: String,
    pub duration: u64,
    pub primary_user_id: Option<String>,
    pub system: Option<String>,
    pub url: Option<String>,
    #[serde(default, flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Party {
    pub id: Option<String>,
    pub name: Option<String>,
    pub email_address: Option<String>,
    pub title: Option<String>,
    pub affiliation: String,
    pub speaker_id: Option<String>,
    pub user_id: Option<String>,
    #[serde(default, flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CrmContext {
    #[serde(default, deserialize_with = "deserialize_null_default")]
    pub objects: Vec<CrmObject>,
    #[serde(default, flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CrmObject {
    pub object_type: String,
    #[serde(default, deserialize_with = "deserialize_null_default")]
    pub fields: Vec<CrmField>,
    #[serde(default, flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CrmField {
    pub name: String,
    pub value: Value,
    #[serde(default, flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CallContent {
    pub brief: Option<String>,
    pub key_points: Option<Vec<TextItem>>,
    pub highlights: Option<Vec<Highlight>>,
    pub outline: Option<Vec<OutlineSection>>,
    #[serde(default, flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TextItem {
    pub text: String,
    #[serde(default, flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Highlight {
    pub title: String,
    #[serde(default, deserialize_with = "deserialize_null_default")]
    pub items: Vec<HighlightItem>,
    #[serde(default, flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HighlightItem {
    pub text: String,
    #[serde(default, deserialize_with = "deserialize_null_default")]
    pub start_times: Vec<f64>,
    #[serde(default, flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutlineSection {
    pub section: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_time: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<f64>,
    #[serde(default, deserialize_with = "deserialize_null_default")]
    pub items: Vec<OutlineItem>,
    #[serde(default, flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutlineItem {
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_time: Option<f64>,
    #[serde(default, flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptResponse {
    #[serde(default, deserialize_with = "deserialize_null_default")]
    pub call_transcripts: Vec<CallTranscript>,
    #[serde(default, flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CallTranscript {
    pub call_id: String,
    #[serde(default, deserialize_with = "deserialize_null_default")]
    pub transcript: Vec<TranscriptEntry>,
    #[serde(default, flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptEntry {
    pub speaker_id: String,
    pub topic: Option<String>,
    #[serde(default, deserialize_with = "deserialize_null_default")]
    pub sentences: Vec<Sentence>,
    #[serde(default, flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Sentence {
    pub start: u64,
    pub end: u64,
    pub text: String,
    #[serde(default, flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Serialize)]
struct ExtensiveRequest {
    // Gong's OpenAPI schema places the next-page cursor at the request root,
    // even though date/owner criteria live under `filter`.
    #[serde(skip_serializing_if = "Option::is_none")]
    cursor: Option<String>,
    filter: CallsFilter,
    #[serde(rename = "contentSelector")]
    content_selector: Value,
}

#[derive(Debug, Clone, Serialize)]
struct CallsFilter {
    #[serde(rename = "fromDateTime", skip_serializing_if = "Option::is_none")]
    from_date_time: Option<String>,
    #[serde(rename = "toDateTime", skip_serializing_if = "Option::is_none")]
    to_date_time: Option<String>,
    #[serde(rename = "callIds", skip_serializing_if = "Option::is_none")]
    call_ids: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
struct TranscriptRequest {
    filter: TranscriptFilter,
}

#[derive(Debug, Serialize)]
struct TranscriptFilter {
    #[serde(rename = "callIds")]
    call_ids: Vec<String>,
}

impl RequestPacer {
    fn new(interval: StdDuration) -> Self {
        Self {
            next_request: Mutex::new(Instant::now()),
            interval,
        }
    }

    async fn wait(&self) {
        let mut next_request = self.next_request.lock().await;
        let now = Instant::now();
        if *next_request > now {
            tokio::time::sleep_until(*next_request).await;
        }
        *next_request = Instant::now() + self.interval;
    }
}

impl GongClient {
    pub fn new(base_url: String, access_key: String, access_key_secret: String) -> Self {
        Self {
            http: reqwest::Client::new(),
            base_url,
            access_key,
            access_key_secret,
            pacer: Arc::new(RequestPacer::new(REQUEST_INTERVAL)),
        }
    }

    pub async fn verify_credentials(&self) -> Result<(), ApiError> {
        let to = Utc::now();
        let from = to - Duration::days(1);
        self.fetch_extensive_page(CallsFilter::dates(from, to), None)
            .await
            .map(|_| ())
    }

    pub async fn list_calls(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<ExtensiveCall>, ApiError> {
        self.list_calls_with_filter(CallsFilter::dates(from, to))
            .await
    }

    pub async fn list_all_calls(&self) -> Result<Vec<ExtensiveCall>, ApiError> {
        self.list_calls_with_filter(CallsFilter::all()).await
    }

    async fn list_calls_with_filter(
        &self,
        filter: CallsFilter,
    ) -> Result<Vec<ExtensiveCall>, ApiError> {
        let mut calls = Vec::new();
        let mut cursor = None;
        let mut seen_cursors = HashSet::new();

        loop {
            let page = self.fetch_extensive_page(filter.clone(), cursor).await?;
            calls.extend(page.calls);
            let Some(next_cursor) = page.records.cursor else {
                break;
            };
            if !seen_cursors.insert(next_cursor.clone()) {
                return Err(ApiError::PaginationLoop(next_cursor));
            }
            cursor = Some(next_cursor);
        }
        Ok(calls)
    }

    pub async fn get_call(&self, call_id: &str) -> Result<ExtensiveCall, ApiError> {
        let page = self
            .fetch_extensive_page(CallsFilter::call_id(call_id), None)
            .await?;
        page.calls
            .into_iter()
            .find(|call| call.metadata.id == call_id)
            .ok_or_else(|| ApiError::CallNotFound(call_id.to_owned()))
    }

    pub async fn get_transcript(&self, call_id: &str) -> Result<CallTranscript, ApiError> {
        self.get_transcripts(&[call_id.to_owned()])
            .await?
            .into_iter()
            .find(|transcript| transcript.call_id == call_id)
            .ok_or_else(|| ApiError::TranscriptNotFound(call_id.to_owned()))
    }

    pub async fn get_transcripts(
        &self,
        call_ids: &[String],
    ) -> Result<Vec<CallTranscript>, ApiError> {
        let request = TranscriptRequest {
            filter: TranscriptFilter {
                call_ids: call_ids.to_vec(),
            },
        };
        let response: TranscriptResponse = self
            .post_json("/v2/calls/transcript", &request, "Call Transcript")
            .await?;
        Ok(response.call_transcripts)
    }

    async fn fetch_extensive_page(
        &self,
        filter: CallsFilter,
        cursor: Option<String>,
    ) -> Result<ExtensiveResponse, ApiError> {
        let request = ExtensiveRequest {
            cursor,
            filter,
            content_selector: extensive_content_selector(),
        };
        self.post_json("/v2/calls/extensive", &request, "extensive Call")
            .await
    }

    async fn post_json<RequestBody, ResponseBody>(
        &self,
        path: &'static str,
        request: &RequestBody,
        response_name: &'static str,
    ) -> Result<ResponseBody, ApiError>
    where
        RequestBody: Serialize + ?Sized,
        ResponseBody: DeserializeOwned,
    {
        let mut retry = 0;
        loop {
            self.pacer.wait().await;
            let response = self
                .http
                .post(format!("{}{path}", self.base_url))
                .basic_auth(&self.access_key, Some(&self.access_key_secret))
                .json(request)
                .send()
                .await?;

            if response.status() == StatusCode::TOO_MANY_REQUESTS && retry < MAX_429_RETRIES {
                let delay = retry_delay(&response, retry);
                tracing::warn!(?delay, retry, path, "Gong rate limit reached; backing off");
                tokio::time::sleep(delay).await;
                retry += 1;
                continue;
            }

            let status = response.status();
            let body = response.text().await?;
            if !status.is_success() {
                return Err(ApiError::Http {
                    status,
                    message: gong_error_message(&body),
                });
            }
            return serde_json::from_str(&body).map_err(|source| ApiError::InvalidResponse {
                endpoint: response_name,
                source,
            });
        }
    }
}

impl CallsFilter {
    fn all() -> Self {
        Self {
            from_date_time: None,
            to_date_time: None,
            call_ids: None,
        }
    }

    fn dates(from: DateTime<Utc>, to: DateTime<Utc>) -> Self {
        Self {
            from_date_time: Some(from.to_rfc3339_opts(SecondsFormat::Secs, true)),
            to_date_time: Some(to.to_rfc3339_opts(SecondsFormat::Secs, true)),
            call_ids: None,
        }
    }

    fn call_id(call_id: &str) -> Self {
        Self {
            from_date_time: None,
            to_date_time: None,
            call_ids: Some(vec![call_id.to_owned()]),
        }
    }
}

fn extensive_content_selector() -> Value {
    json!({
        "context": "Extended",
        "exposedFields": {
            "parties": true,
            "content": {
                "structure": true,
                "topics": true,
                "trackers": false,
                "pointsOfInterest": true,
                "brief": true,
                "outline": true,
                "highlights": true,
                "callOutcome": true,
                "keyPoints": true
            },
            "interaction": {
                "speakers": true,
                "video": false,
                "personInteractionStats": false,
                "questions": false
            },
            "collaboration": {"publicComments": false},
            "media": false
        }
    })
}

fn retry_delay(response: &Response, retry: u32) -> StdDuration {
    let retry_after = response
        .headers()
        .get(RETRY_AFTER)
        .and_then(|value| value.to_str().ok());
    if let Some(seconds) = retry_after.and_then(|value| value.parse::<u64>().ok()) {
        return StdDuration::from_secs(seconds);
    }
    if let Some(delay) = retry_after
        .and_then(|value| DateTime::parse_from_rfc2822(value).ok())
        .and_then(|retry_at| (retry_at.with_timezone(&Utc) - Utc::now()).to_std().ok())
    {
        return delay;
    }

    StdDuration::from_millis(250 * 2_u64.pow(retry)).min(MAX_BACKOFF)
}

fn gong_error_message(body: &str) -> String {
    let Ok(value) = serde_json::from_str::<Value>(body) else {
        return body.trim().to_owned();
    };
    let Some(errors) = value.get("errors").and_then(Value::as_array) else {
        return body.trim().to_owned();
    };

    errors
        .iter()
        .map(|error| {
            error
                .as_str()
                .or_else(|| error.get("message").and_then(Value::as_str))
                .map(str::to_owned)
                .unwrap_or_else(|| error.to_string())
        })
        .collect::<Vec<_>>()
        .join("; ")
}
