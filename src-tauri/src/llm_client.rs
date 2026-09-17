use crate::screenshot::CapturedScreenshot;
use crate::settings::PostProcessProvider;
use log::{debug, error, info};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE, REFERER, USER_AGENT};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::error::Error as StdError;
use std::sync::{Mutex, OnceLock};

#[derive(Debug, Serialize)]
struct ChatMessage {
    role: String,
    content: MessageContent,
}

/// OpenAI-compatible message content: a plain string for text-only requests
/// (what every endpoint understands) or content parts when a screenshot is
/// attached.
#[derive(Debug, Serialize)]
#[serde(untagged)]
enum MessageContent {
    Text(String),
    Parts(Vec<ContentPart>),
}

impl MessageContent {
    fn with_images(text: String, image_data_urls: Vec<String>) -> Self {
        let mut parts = Vec::with_capacity(image_data_urls.len() + 1);
        parts.push(ContentPart::Text { text });
        parts.extend(
            image_data_urls
                .into_iter()
                .map(|url| ContentPart::ImageUrl {
                    image_url: ImageUrl { url },
                }),
        );
        Self::Parts(parts)
    }
}

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
enum ContentPart {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image_url")]
    ImageUrl { image_url: ImageUrl },
}

#[derive(Debug, Serialize)]
struct ImageUrl {
    url: String,
}

#[derive(Debug, Serialize)]
struct JsonSchema {
    name: String,
    strict: bool,
    schema: Value,
}

#[derive(Debug, Serialize)]
struct ResponseFormat {
    #[serde(rename = "type")]
    format_type: String,
    json_schema: JsonSchema,
}

#[derive(Debug, Serialize, Clone, Default, PartialEq)]
struct ReasoningConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    exclude: Option<bool>,
}

/// Request fields used to ask an endpoint to skip reasoning/thinking.
/// Providers disagree on the field name and accepted values, so at most one of
/// these is set per request (see `reasoning_disable_params`).
#[derive(Debug, Serialize, Clone, Default, PartialEq)]
struct ReasoningParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning: Option<ReasoningConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking: Option<Value>,
}

impl ReasoningParams {
    fn is_empty(&self) -> bool {
        self.reasoning_effort.is_none() && self.reasoning.is_none() && self.thinking.is_none()
    }
}

/// Pick the reasoning-disable request fields an endpoint understands.
/// Unknown endpoints get the common OpenAI-style field; if they reject it,
/// the request is retried without it (see `send_chat_completion_with_schema`).
fn reasoning_disable_params(provider: &PostProcessProvider) -> ReasoningParams {
    let base_url = provider.base_url.to_lowercase();
    if base_url.contains("api.deepseek.com") {
        // DeepSeek rejects reasoning_effort "none" and uses its own field:
        // https://api-docs.deepseek.com/guides/thinking_mode
        ReasoningParams {
            thinking: Some(serde_json::json!({ "type": "disabled" })),
            ..Default::default()
        }
    } else if provider.id == "openrouter" {
        // OpenRouter nested object; exclude:true also keeps reasoning text out
        // of the response so it can't pollute structured-output JSON parsing
        ReasoningParams {
            reasoning: Some(ReasoningConfig {
                effort: Some("none".to_string()),
                exclude: Some(true),
            }),
            ..Default::default()
        }
    } else {
        ReasoningParams {
            reasoning_effort: Some("none".to_string()),
            ..Default::default()
        }
    }
}

/// Optional request features an endpoint may reject with a 4xx.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum RequestFeature {
    /// The reasoning-disable fields picked by `reasoning_disable_params`.
    ReasoningDisable,
    /// The screenshot `image_url` content part.
    Image,
}

/// Endpoints (base_url|model) that rejected a request feature with a 4xx.
/// Remembered for the lifetime of the process so every dictation after the
/// first skips the doomed attempt and goes straight to a request the endpoint
/// accepts.
fn feature_rejections() -> &'static Mutex<HashSet<(RequestFeature, String)>> {
    static REJECTED: OnceLock<Mutex<HashSet<(RequestFeature, String)>>> = OnceLock::new();
    REJECTED.get_or_init(|| Mutex::new(HashSet::new()))
}

fn endpoint_key(provider: &PostProcessProvider, model: &str) -> String {
    format!("{}|{}", provider.base_url.trim_end_matches('/'), model)
}

fn is_known_rejected(feature: RequestFeature, key: &str) -> bool {
    feature_rejections()
        .lock()
        .map(|set| set.contains(&(feature, key.to_string())))
        .unwrap_or(false)
}

fn remember_rejection(feature: RequestFeature, key: &str) {
    if let Ok(mut set) = feature_rejections().lock() {
        set.insert((feature, key.to_string()));
    }
}

/// Status codes an endpoint uses to refuse a request field or content type it
/// does not understand (as opposed to auth, rate-limit or server errors).
fn is_payload_rejection(status: StatusCode) -> bool {
    matches!(status.as_u16(), 400 | 415 | 422)
}

#[derive(Debug, Serialize)]
struct ChatCompletionRequest {
    model: String,
    messages: Vec<ChatMessage>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<ResponseFormat>,
    #[serde(flatten)]
    reasoning: ReasoningParams,
}

impl ChatCompletionRequest {
    fn has_image(&self) -> bool {
        self.messages
            .iter()
            .any(|message| matches!(message.content, MessageContent::Parts(_)))
    }

    /// Collapse multimodal messages back to their text part.
    fn strip_image(&mut self) {
        for message in &mut self.messages {
            if let MessageContent::Parts(parts) = &mut message.content {
                let text = parts
                    .drain(..)
                    .filter_map(|part| match part {
                        ContentPart::Text { text } => Some(text),
                        ContentPart::ImageUrl { .. } => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                message.content = MessageContent::Text(text);
            }
        }
    }
}

#[derive(Debug, Deserialize)]
struct ChatCompletionResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatMessageResponse,
}

#[derive(Debug, Deserialize)]
struct ChatMessageResponse {
    content: Option<String>,
}

/// Build headers for API requests based on provider type
fn build_headers(provider: &PostProcessProvider, api_key: &str) -> Result<HeaderMap, String> {
    let mut headers = HeaderMap::new();

    // Common headers
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(
        REFERER,
        HeaderValue::from_static("https://github.com/cjpais/Handy"),
    );
    headers.insert(
        USER_AGENT,
        HeaderValue::from_static("Handy/1.0 (+https://github.com/cjpais/Handy)"),
    );
    headers.insert("X-Title", HeaderValue::from_static("Handy"));

    // Provider-specific auth headers
    if !api_key.is_empty() {
        if provider.id == "anthropic" {
            headers.insert(
                "x-api-key",
                HeaderValue::from_str(api_key)
                    .map_err(|e| format!("Invalid API key header value: {}", e))?,
            );
            headers.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));
        } else {
            headers.insert(
                AUTHORIZATION,
                HeaderValue::from_str(&format!("Bearer {}", api_key))
                    .map_err(|e| format!("Invalid authorization header value: {}", e))?,
            );
        }
    }

    Ok(headers)
}

/// Create an HTTP client with provider-specific headers
fn create_client(provider: &PostProcessProvider, api_key: &str) -> Result<reqwest::Client, String> {
    let headers = build_headers(provider, api_key)?;
    reqwest::Client::builder()
        .default_headers(headers)
        .build()
        .map_err(|e| report_reqwest_error("Failed to build HTTP client", &e))
}

/// Format a bounded error source chain.
///
/// `reqwest::Error`'s Display implementation intentionally gives only a short
/// summary. Nested causes contain the useful transport details, such as a
/// certificate validation failure, an HTTP/2 error, or a connection reset.
/// Callers must skip source types whose Display text can quote payload data.
fn error_source_chain(error: &(dyn StdError + 'static)) -> Vec<String> {
    let mut causes = Vec::new();
    let mut source = error.source();

    // Defensive cap in case a third-party error exposes a cyclic source chain.
    for _ in 0..16 {
        let Some(cause) = source else {
            break;
        };
        causes.push(cause.to_string());
        source = cause.source();
    }

    causes
}

fn reqwest_error_kinds(error: &reqwest::Error) -> String {
    let mut kinds = Vec::new();

    if error.is_builder() {
        kinds.push("builder");
    }
    if error.is_connect() {
        kinds.push("connect");
    }
    if error.is_request() {
        kinds.push("request");
    }
    if error.is_redirect() {
        kinds.push("redirect");
    }
    if error.is_timeout() {
        kinds.push("timeout");
    }
    if error.is_status() {
        kinds.push("status");
    }
    if error.is_body() {
        kinds.push("body");
    }
    if error.is_decode() {
        kinds.push("decode");
    }
    if error.is_upgrade() {
        kinds.push("upgrade");
    }

    if kinds.is_empty() {
        "unknown".to_string()
    } else {
        kinds.join(", ")
    }
}

fn sanitized_url(url: &reqwest::Url) -> String {
    let mut url = url.clone();

    // Custom endpoints should not contain credentials or query-string tokens,
    // but omit them from diagnostics in case one does.
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.set_query(None);
    url.set_fragment(None);

    url.to_string()
}

fn sanitized_url_for_log(url: &str) -> String {
    reqwest::Url::parse(url)
        .map(|url| sanitized_url(&url))
        // Do not echo an invalid URL: the parse failure might have been caused
        // by sensitive data entered in the custom endpoint field.
        .unwrap_or_else(|_| "<invalid URL>".to_string())
}

fn report_reqwest_error(context: &str, error: &reqwest::Error) -> String {
    let kinds = reqwest_error_kinds(error);
    let url = error
        .url()
        .map(sanitized_url)
        .map(|url| format!(", url: {url}"))
        .unwrap_or_default();

    // serde_json's error text can quote values from a malformed response. That
    // response may contain transcription content, so retain the useful decode
    // classification but never put its nested source in logs or UI errors.
    let causes = if error.is_decode() {
        Vec::new()
    } else {
        error_source_chain(error)
    };
    let cause_details = if !causes.is_empty() {
        format!(": caused by: {}", causes.join(" -> "))
    } else if error.url().is_none() {
        // Reqwest's short Display text is safe when it cannot append a raw URL.
        format!(": {error}")
    } else {
        // The sanitized URL is already included above. Avoid formatting the
        // original error because its Display implementation includes the raw URL.
        String::new()
    };

    let details = format!("{context} (kind: {kinds}{url}){cause_details}");
    error!("{details}");
    details
}

async fn post_json(
    client: &reqwest::Client,
    url: &str,
    body: &ChatCompletionRequest,
    context: &str,
) -> Result<reqwest::Response, String> {
    let response = client
        .post(url)
        .json(body)
        .send()
        .await
        .map_err(|e| report_reqwest_error(context, &e))?;
    debug!(
        "Chat completion response received with status {} over {:?} from {}",
        response.status(),
        response.version(),
        sanitized_url(response.url())
    );
    Ok(response)
}

async fn read_error_text(response: reqwest::Response, context: &str) -> String {
    response
        .text()
        .await
        .unwrap_or_else(|e| report_reqwest_error(context, &e))
}

/// Send a chat completion request to an OpenAI-compatible API
/// Returns Ok(Some(content)) on success, Ok(None) if response has no content,
/// or Err on actual errors (HTTP, parsing, etc.)
pub async fn send_chat_completion(
    provider: &PostProcessProvider,
    api_key: String,
    model: &str,
    prompt: String,
    disable_reasoning: bool,
    screenshot: Option<&CapturedScreenshot>,
) -> Result<Option<String>, String> {
    send_chat_completion_with_schema(
        provider,
        api_key,
        model,
        prompt,
        None,
        None,
        disable_reasoning,
        screenshot,
    )
    .await
}

/// Send a chat completion request with structured output support.
/// When json_schema is provided, uses structured outputs mode.
/// system_prompt is used as the system message when provided.
///
/// When a screenshot is provided, the user message is sent as content parts
/// (text + one `image_url` per monitor). When disable_reasoning is set, the request
/// carries the reasoning-disable fields the endpoint is expected to understand.
///
/// Not every OpenAI-compatible endpoint accepts either (text-only models
/// reject image parts; DeepSeek, Gemini's compat layer, and some OpenRouter
/// upstreams reject the reasoning fields with 400), so a 400/415/422 answer
/// triggers at most two retries: first without the image, then without the
/// reasoning fields. Each retry removes exactly one thing, so a success pins
/// the rejection on what was just removed and it is remembered per
/// (base_url, model) so later requests skip the failing attempt entirely.
#[allow(clippy::too_many_arguments)]
pub async fn send_chat_completion_with_schema(
    provider: &PostProcessProvider,
    api_key: String,
    model: &str,
    user_content: String,
    system_prompt: Option<String>,
    json_schema: Option<Value>,
    disable_reasoning: bool,
    screenshot: Option<&CapturedScreenshot>,
) -> Result<Option<String>, String> {
    let base_url = provider.base_url.trim_end_matches('/');
    let url = format!("{}/chat/completions", base_url);

    debug!(
        "Sending chat completion request to: {}",
        sanitized_url_for_log(&url)
    );

    let client = create_client(provider, &api_key)?;
    let key = endpoint_key(provider, model);

    // Build messages vector
    let mut messages = Vec::new();

    // Add system prompt if provided
    if let Some(system) = system_prompt {
        messages.push(ChatMessage {
            role: "system".to_string(),
            content: MessageContent::Text(system),
        });
    }

    // Add user message, with the screenshot attached unless this endpoint is
    // already known to refuse images.
    let screenshot = screenshot.filter(|_| {
        let accepted = !is_known_rejected(RequestFeature::Image, &key);
        if !accepted {
            debug!("Skipping screenshot: endpoint previously rejected image content");
        }
        accepted
    });
    messages.push(ChatMessage {
        role: "user".to_string(),
        content: match screenshot {
            Some(screenshot) => {
                MessageContent::with_images(user_content, screenshot.llm_data_urls())
            }
            None => MessageContent::Text(user_content),
        },
    });

    // Build response_format if schema is provided
    let response_format = json_schema.map(|schema| ResponseFormat {
        format_type: "json_schema".to_string(),
        json_schema: JsonSchema {
            name: "transcription_output".to_string(),
            strict: true,
            schema,
        },
    });

    let reasoning =
        if disable_reasoning && !is_known_rejected(RequestFeature::ReasoningDisable, &key) {
            reasoning_disable_params(provider)
        } else {
            ReasoningParams::default()
        };

    let mut request_body = ChatCompletionRequest {
        model: model.to_string(),
        messages,
        stream: false,
        response_format,
        reasoning,
    };

    let mut response = post_json(&client, &url, &request_body, "HTTP request failed").await?;
    let mut status = response.status();

    // A text-only model refuses the image content part with a 4xx. Retry once
    // without it so post-processing still lands as text.
    if is_payload_rejection(status) && request_body.has_image() {
        let error_text = read_error_text(response, "Failed to read image rejection response").await;
        info!(
            "Endpoint rejected request with screenshot (status {}): {}. Retrying without the image",
            status, error_text
        );

        request_body.strip_image();
        response = post_json(
            &client,
            &url,
            &request_body,
            "HTTP retry without image failed",
        )
        .await?;
        status = response.status();

        if status.is_success() {
            info!(
                "Retry without the screenshot succeeded; '{}' (model '{}') will get text-only requests from now on",
                sanitized_url_for_log(base_url), model
            );
            remember_rejection(RequestFeature::Image, &key);
        }
    }

    // A 4xx on a request carrying reasoning-disable fields is almost always
    // the endpoint rejecting those fields — retry once without them.
    if is_payload_rejection(status) && !request_body.reasoning.is_empty() {
        let error_text =
            read_error_text(response, "Failed to read reasoning rejection response").await;
        info!(
            "Endpoint rejected request with reasoning disabled (status {}): {}. Retrying without reasoning fields",
            status, error_text
        );

        request_body.reasoning = ReasoningParams::default();
        response = post_json(&client, &url, &request_body, "HTTP retry failed").await?;
        status = response.status();

        if status.is_success() {
            info!(
                "Retry without reasoning fields succeeded; '{}' (model '{}') will skip them from now on",
                sanitized_url_for_log(base_url), model
            );
            remember_rejection(RequestFeature::ReasoningDisable, &key);
        }
    }

    if !status.is_success() {
        let error_text = read_error_text(response, "Failed to read API error response").await;
        return Err(format!(
            "API request failed with status {}: {}",
            status, error_text
        ));
    }

    let completion: ChatCompletionResponse = response
        .json()
        .await
        .map_err(|e| report_reqwest_error("Failed to parse API response", &e))?;

    Ok(completion
        .choices
        .first()
        .and_then(|choice| choice.message.content.clone()))
}

/// Fetch available models from an OpenAI-compatible API
/// Returns a list of model IDs
pub async fn fetch_models(
    provider: &PostProcessProvider,
    api_key: String,
) -> Result<Vec<String>, String> {
    let base_url = provider.base_url.trim_end_matches('/');
    let url = format!("{}/models", base_url);

    debug!("Fetching models from: {}", sanitized_url_for_log(&url));

    let client = create_client(provider, &api_key)?;

    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| report_reqwest_error("Failed to fetch models", &e))?;

    let status = response.status();
    debug!(
        "Model list response received with status {} over {:?} from {}",
        status,
        response.version(),
        sanitized_url(response.url())
    );
    if !status.is_success() {
        let error_text = response
            .text()
            .await
            .unwrap_or_else(|e| report_reqwest_error("Failed to read model list error", &e));
        return Err(format!(
            "Model list request failed ({}): {}",
            status, error_text
        ));
    }

    let parsed: serde_json::Value = response
        .json()
        .await
        .map_err(|e| report_reqwest_error("Failed to parse model list response", &e))?;

    let mut models = Vec::new();

    // Handle OpenAI format: { data: [ { id: "..." }, ... ] }
    if let Some(data) = parsed.get("data").and_then(|d| d.as_array()) {
        for entry in data {
            if let Some(id) = entry.get("id").and_then(|i| i.as_str()) {
                models.push(id.to_string());
            } else if let Some(name) = entry.get("name").and_then(|n| n.as_str()) {
                models.push(name.to_string());
            }
        }
    }
    // Handle array format: [ "model1", "model2", ... ]
    else if let Some(array) = parsed.as_array() {
        for entry in array {
            if let Some(model) = entry.as_str() {
                models.push(model.to_string());
            }
        }
    }

    Ok(models)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt;
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[derive(Debug)]
    struct TestError {
        message: &'static str,
        source: Option<Box<TestError>>,
    }

    impl fmt::Display for TestError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str(self.message)
        }
    }

    impl StdError for TestError {
        fn source(&self) -> Option<&(dyn StdError + 'static)> {
            self.source
                .as_deref()
                .map(|source| source as &(dyn StdError + 'static))
        }
    }

    fn provider(id: &str, base_url: &str) -> PostProcessProvider {
        PostProcessProvider {
            id: id.to_string(),
            label: id.to_string(),
            base_url: base_url.to_string(),
            allow_base_url_edit: true,
            models_endpoint: None,
            supports_structured_output: false,
        }
    }

    fn request_json(reasoning: ReasoningParams) -> Value {
        serde_json::to_value(request(MessageContent::Text("hi".to_string()), reasoning)).unwrap()
    }

    fn request(content: MessageContent, reasoning: ReasoningParams) -> ChatCompletionRequest {
        ChatCompletionRequest {
            model: "test-model".to_string(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content,
            }],
            stream: false,
            response_format: None,
            reasoning,
        }
    }

    fn screenshot() -> CapturedScreenshot {
        let image = image::RgbaImage::from_pixel(4, 4, image::Rgba([10, 20, 30, 255]));
        crate::screenshot::encode_screenshot(image).unwrap()
    }

    const COMPLETION_OK: &str = r#"{"choices":[{"message":{"content":"cleaned"}}]}"#;

    /// Serve `responses` in order, one connection each, recording every
    /// request body so tests can assert what each retry actually sent.
    async fn serve_responses(
        responses: Vec<(&'static str, &'static str)>,
    ) -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let bodies = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&bodies);

        tokio::spawn(async move {
            for (status, body) in responses {
                let (mut stream, _) = listener.accept().await.unwrap();
                let request_body = read_request_body(&mut stream).await;
                recorded.lock().unwrap().push(request_body);
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).await.unwrap();
            }
        });

        (format!("http://{address}"), bodies)
    }

    async fn read_request_body(stream: &mut tokio::net::TcpStream) -> String {
        let mut buffer = Vec::new();
        let mut chunk = [0_u8; 4096];
        let header_end = loop {
            let read = stream.read(&mut chunk).await.unwrap();
            if read == 0 {
                return String::new();
            }
            buffer.extend_from_slice(&chunk[..read]);
            if let Some(position) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
                break position + 4;
            }
        };

        let headers = String::from_utf8_lossy(&buffer[..header_end]).to_ascii_lowercase();
        let content_length = headers
            .lines()
            .find_map(|line| line.strip_prefix("content-length:"))
            .and_then(|value| value.trim().parse::<usize>().ok())
            .unwrap_or(0);
        while buffer.len() < header_end + content_length {
            let read = stream.read(&mut chunk).await.unwrap();
            if read == 0 {
                break;
            }
            buffer.extend_from_slice(&chunk[..read]);
        }

        String::from_utf8_lossy(&buffer[header_end..]).into_owned()
    }

    async fn serve_one_response(status: &str, body: &str) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );

        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 2048];
            let _ = stream.read(&mut request).await.unwrap();
            stream.write_all(response.as_bytes()).await.unwrap();
        });

        format!("http://{address}")
    }

    #[test]
    fn error_source_chain_includes_all_nested_causes() {
        let error = TestError {
            message: "request failed",
            source: Some(Box::new(TestError {
                message: "TLS handshake failed",
                source: Some(Box::new(TestError {
                    message: "unknown certificate authority",
                    source: None,
                })),
            })),
        };

        assert_eq!(
            error_source_chain(&error),
            vec!["TLS handshake failed", "unknown certificate authority"]
        );
    }

    #[test]
    fn log_url_sanitization_removes_credentials_and_tokens() {
        let url = "https://user:password@example.com/v1/models?api_key=secret#private";
        assert_eq!(sanitized_url_for_log(url), "https://example.com/v1/models");
    }

    #[test]
    fn invalid_log_urls_are_not_echoed() {
        assert_eq!(
            sanitized_url_for_log("not a URL containing secret"),
            "<invalid URL>"
        );
    }

    #[tokio::test]
    async fn decode_error_does_not_echo_response_values() {
        let base_url =
            serve_one_response("200 OK", r#"{"choices":"PRIVATE TRANSCRIPTION CONTENT"}"#).await;
        let error = reqwest::get(base_url)
            .await
            .unwrap()
            .json::<ChatCompletionResponse>()
            .await
            .unwrap_err();

        let details = report_reqwest_error("Failed to parse API response", &error);
        assert!(details.contains("kind: decode"));
        assert!(!details.contains("PRIVATE TRANSCRIPTION CONTENT"));
    }

    #[tokio::test]
    async fn raw_error_url_is_not_reintroduced_without_a_source() {
        let base_url = serve_one_response("400 Bad Request", "bad request").await;
        let error = reqwest::get(format!(
            "{base_url}/private?api_key=SECRET_QUERY_TOKEN#private"
        ))
        .await
        .unwrap()
        .error_for_status()
        .unwrap_err();

        let details = report_reqwest_error("Request failed", &error);
        assert!(details.contains(&format!("url: {base_url}/private")));
        assert!(!details.contains("SECRET_QUERY_TOKEN"));
        assert!(!details.contains("#private"));
    }

    #[test]
    fn requests_explicitly_disable_streaming() {
        let json = request_json(ReasoningParams::default());
        assert_eq!(json["stream"], false);
    }

    #[test]
    fn default_reasoning_params_serialize_to_no_fields() {
        let json = request_json(ReasoningParams::default());
        assert!(json.get("reasoning_effort").is_none());
        assert!(json.get("reasoning").is_none());
        assert!(json.get("thinking").is_none());
    }

    #[test]
    fn custom_provider_uses_top_level_reasoning_effort() {
        let params = reasoning_disable_params(&provider("custom", "http://localhost:11434/v1"));
        let json = request_json(params);
        assert_eq!(json["reasoning_effort"], "none");
        assert!(json.get("reasoning").is_none());
        assert!(json.get("thinking").is_none());
    }

    #[test]
    fn openrouter_uses_nested_reasoning_object() {
        let params =
            reasoning_disable_params(&provider("openrouter", "https://openrouter.ai/api/v1"));
        let json = request_json(params);
        assert!(json.get("reasoning_effort").is_none());
        assert_eq!(json["reasoning"]["effort"], "none");
        assert_eq!(json["reasoning"]["exclude"], true);
        assert!(json.get("thinking").is_none());
    }

    #[test]
    fn deepseek_base_url_uses_thinking_disabled() {
        let params = reasoning_disable_params(&provider("custom", "https://api.deepseek.com"));
        let json = request_json(params);
        assert!(json.get("reasoning_effort").is_none());
        assert!(json.get("reasoning").is_none());
        assert_eq!(json["thinking"]["type"], "disabled");
    }

    #[test]
    fn reasoning_params_is_empty_tracks_all_fields() {
        assert!(ReasoningParams::default().is_empty());
        assert!(!ReasoningParams {
            reasoning_effort: Some("none".to_string()),
            ..Default::default()
        }
        .is_empty());
        assert!(!ReasoningParams {
            thinking: Some(serde_json::json!({ "type": "disabled" })),
            ..Default::default()
        }
        .is_empty());
    }

    #[test]
    fn rejection_memo_is_keyed_by_feature_base_url_and_model() {
        let deepseek = provider("custom", "https://api.deepseek.com/");
        let key = endpoint_key(&deepseek, "deepseek-chat");
        assert_eq!(key, "https://api.deepseek.com|deepseek-chat");
        assert!(!is_known_rejected(RequestFeature::ReasoningDisable, &key));
        remember_rejection(RequestFeature::ReasoningDisable, &key);
        assert!(is_known_rejected(RequestFeature::ReasoningDisable, &key));
        // Each feature and each model on the same endpoint is tracked separately
        assert!(!is_known_rejected(RequestFeature::Image, &key));
        assert!(!is_known_rejected(
            RequestFeature::ReasoningDisable,
            &endpoint_key(&deepseek, "other-model")
        ));
    }

    #[test]
    fn text_only_messages_serialize_as_plain_strings() {
        let json = request_json(ReasoningParams::default());
        assert_eq!(json["messages"][0]["content"], "hi");
    }

    #[test]
    fn screenshot_messages_serialize_as_content_parts() {
        let content = MessageContent::with_images(
            "hi".to_string(),
            vec![screenshot().data_url(), screenshot().data_url()],
        );
        let json = serde_json::to_value(request(content, ReasoningParams::default())).unwrap();

        let parts = json["messages"][0]["content"].as_array().unwrap();
        assert_eq!(parts.len(), 3);
        assert_eq!(
            parts[0],
            serde_json::json!({ "type": "text", "text": "hi" })
        );
        assert_eq!(parts[1]["type"], "image_url");
        let url = parts[1]["image_url"]["url"].as_str().unwrap();
        assert!(url.starts_with("data:image/jpeg;base64,"));
        assert_eq!(parts[2]["type"], "image_url");
    }

    #[test]
    fn strip_image_restores_plain_text_content() {
        let content = MessageContent::with_images(
            "hi".to_string(),
            vec![screenshot().data_url(), screenshot().data_url()],
        );
        let mut request = request(content, ReasoningParams::default());
        assert!(request.has_image());

        request.strip_image();

        assert!(!request.has_image());
        let json = serde_json::to_value(&request).unwrap();
        assert_eq!(json["messages"][0]["content"], "hi");
    }

    #[test]
    fn payload_rejection_statuses() {
        for status in [400, 415, 422] {
            assert!(is_payload_rejection(StatusCode::from_u16(status).unwrap()));
        }
        for status in [401, 403, 404, 429, 500] {
            assert!(!is_payload_rejection(StatusCode::from_u16(status).unwrap()));
        }
    }

    #[tokio::test]
    async fn image_rejection_retries_once_without_the_image_and_is_remembered() {
        let (base_url, bodies) = serve_responses(vec![
            ("400 Bad Request", "no images"),
            ("200 OK", COMPLETION_OK),
        ])
        .await;
        let custom = provider("custom", &base_url);

        let result = send_chat_completion(
            &custom,
            String::new(),
            "text-model",
            "hi".to_string(),
            false,
            Some(&screenshot()),
        )
        .await;

        assert_eq!(result, Ok(Some("cleaned".to_string())));
        let bodies = bodies.lock().unwrap();
        assert_eq!(bodies.len(), 2);
        assert!(bodies[0].contains("image_url"));
        assert!(!bodies[1].contains("image_url"));
        assert!(is_known_rejected(
            RequestFeature::Image,
            &endpoint_key(&custom, "text-model")
        ));
    }

    #[tokio::test]
    async fn fallback_is_bounded_when_image_and_reasoning_are_both_rejected() {
        let (base_url, bodies) = serve_responses(vec![
            ("400 Bad Request", "no images"),
            ("422 Unprocessable Entity", "no reasoning_effort"),
            ("200 OK", COMPLETION_OK),
        ])
        .await;
        let custom = provider("custom", &base_url);

        let result = send_chat_completion(
            &custom,
            String::new(),
            "strict-model",
            "hi".to_string(),
            true,
            Some(&screenshot()),
        )
        .await;

        assert_eq!(result, Ok(Some("cleaned".to_string())));
        let bodies = bodies.lock().unwrap();
        assert_eq!(bodies.len(), 3);
        assert!(bodies[0].contains("image_url") && bodies[0].contains("reasoning_effort"));
        assert!(!bodies[1].contains("image_url") && bodies[1].contains("reasoning_effort"));
        assert!(!bodies[2].contains("image_url") && !bodies[2].contains("reasoning_effort"));
        let key = endpoint_key(&custom, "strict-model");
        assert!(is_known_rejected(RequestFeature::ReasoningDisable, &key));
        // Only the reasoning retry flipped failure to success, so the image
        // is not blamed yet.
        assert!(!is_known_rejected(RequestFeature::Image, &key));
    }

    #[tokio::test]
    async fn plain_text_rejection_is_not_retried() {
        let (base_url, bodies) = serve_responses(vec![("400 Bad Request", "bad prompt")]).await;
        let custom = provider("custom", &base_url);

        let result = send_chat_completion(
            &custom,
            String::new(),
            "plain-model",
            "hi".to_string(),
            false,
            None,
        )
        .await;

        assert!(result.unwrap_err().contains("400"));
        assert_eq!(bodies.lock().unwrap().len(), 1);
    }
}
