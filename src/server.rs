//! HTTP API consumed by the Studio frontend.

use std::sync::Arc;
use std::time::Instant;

use async_stream::stream;
use axum::body::{Body, Bytes};
use axum::extract::{DefaultBodyLimit, Query, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use base64::Engine as _;
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::engine::{RuntimeEngine, StreamEvent, SynthesisRequest};
use crate::sentence::Word;
use crate::VERSION;

const SCHEMA_VERSION: u8 = 1;
const MAX_BODY_BYTES: usize = 100_000;
const MAX_HEADER_WORDS: usize = 7_000;
const EXPOSED_HEADERS: &str = "x-ovml-duration-ms, x-ovml-sample-rate, x-ovml-mime, x-ovml-words, x-ovml-voice, x-ovml-engine, x-ovml-approx-words, x-ovml-words-truncated";

#[derive(Clone)]
pub struct AppState {
    pub engine: Arc<RuntimeEngine>,
    pub token: String,
    pub started_at: Instant,
}

#[derive(Deserialize)]
struct VoicesQuery {
    #[serde(default)]
    engine: Option<String>,
}

#[derive(Deserialize)]
struct EnsureRequest {
    engine: String,
    voice: String,
}

type ApiError = (StatusCode, Json<Value>);
type ApiResult<T> = Result<T, ApiError>;

pub fn create_router(state: AppState) -> Router {
    let protected = Router::new()
        .route("/v1/engines", get(engines))
        .route("/v1/voices", get(voices))
        .route("/v1/synthesize", post(synthesize))
        .route("/v1/synthesize/stream", post(synthesize_stream))
        .route("/v1/models/ensure", post(ensure_model))
        .route("/v1/cache", delete(clear_cache))
        .route_layer(middleware::from_fn_with_state(state.clone(), require_token));

    Router::new()
        .route("/v1/health", get(health))
        .route("/v1/pair", post(pair))
        .merge(protected)
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .layer(middleware::from_fn(cors))
        .with_state(state)
}

async fn health(State(state): State<AppState>) -> Json<Value> {
    let mut body = state.engine.health();
    body["ok"] = json!(true);
    body["schema"] = json!(SCHEMA_VERSION);
    body["version"] = json!(VERSION);
    body["pid"] = json!(std::process::id());
    body["uptime_s"] = json!(state.started_at.elapsed().as_secs_f64());
    Json(body)
}

async fn pair(State(state): State<AppState>, headers: HeaderMap) -> ApiResult<Json<Value>> {
    let origin = header_str(&headers, "origin");
    if !origin_allowed(origin) {
        return Err(error(StatusCode::FORBIDDEN, "origin not allowed"));
    }
    Ok(Json(
        json!({"schema": SCHEMA_VERSION, "token": state.token}),
    ))
}

async fn engines(State(state): State<AppState>) -> Json<Value> {
    Json(json!({"engines": state.engine.engine_ids()}))
}

async fn voices(State(state): State<AppState>, Query(query): Query<VoicesQuery>) -> Json<Value> {
    let mut result = Vec::new();
    if query.engine.as_deref() != Some("edge") {
        result.push(json!({"id": "piper", "voices": state.engine.piper_voices()}));
    }
    if query.engine.as_deref() != Some("piper") {
        result.push(json!({"id": "edge", "voices": state.engine.edge_voices().await}));
    }
    Json(json!({"engines": result}))
}

async fn synthesize(
    State(state): State<AppState>,
    Json(payload): Json<SynthesisRequest>,
) -> ApiResult<Response> {
    let result = state
        .engine
        .synthesize_all(payload)
        .await
        .map_err(|err| error(status_for(&err), &err))?;
    let (words, truncated) = words_header(&result.words);
    let mut response = Response::new(Body::from(result.audio));
    let headers = response.headers_mut();
    set_header(headers, "Content-Type", &result.mime);
    set_header(
        headers,
        "X-OVML-Duration-Ms",
        &result.duration_ms.to_string(),
    );
    set_header(
        headers,
        "X-OVML-Sample-Rate",
        &result.sample_rate.to_string(),
    );
    set_header(headers, "X-OVML-Mime", &result.mime);
    set_header(headers, "X-OVML-Voice", &result.voice);
    set_header(headers, "X-OVML-Engine", &result.engine);
    set_header(
        headers,
        "X-OVML-Approx-Words",
        if result.engine == "piper" { "1" } else { "0" },
    );
    set_header(headers, "Cache-Control", "no-store");
    if let Some(words) = words {
        set_header(headers, "X-OVML-Words", &words);
    }
    if truncated {
        set_header(headers, "X-OVML-Words-Truncated", "1");
    }
    Ok(response)
}

async fn synthesize_stream(
    State(state): State<AppState>,
    Json(payload): Json<SynthesisRequest>,
) -> ApiResult<Response> {
    if payload.text.trim().is_empty() {
        return Err(error(StatusCode::BAD_REQUEST, "text is required"));
    }
    state
        .engine
        .resolve(
            payload.engine.as_deref(),
            payload.voice.as_deref(),
            payload.language.as_deref(),
        )
        .await
        .map_err(|err| error(status_for(&err), &err))?;

    let engine = Arc::clone(&state.engine);
    let body = Body::from_stream(stream! {
        let mut inner = Box::pin(engine.stream(payload));
        while let Some(item) = inner.next().await {
            match item {
                Ok(StreamEvent::Chunk(chunk)) => {
                    let line = json!({
                        "index": chunk.index,
                        "text": chunk.text,
                        "engine": chunk.engine,
                        "voice": chunk.voice,
                        "audio": base64::engine::general_purpose::STANDARD.encode(&chunk.audio),
                        "mime": chunk.mime,
                        "sample_rate": chunk.sample_rate,
                        "duration_ms": chunk.duration_ms,
                        "words": chunk.words,
                        "approx_words": chunk.approx_words,
                        "done": false,
                    });
                    yield Ok::<Bytes, std::convert::Infallible>(ndjson(&line));
                }
                Ok(StreamEvent::Done { index, engine, voice }) => {
                    let line = json!({
                        "index": index,
                        "done": true,
                        "engine": engine,
                        "voice": voice,
                    });
                    yield Ok(ndjson(&line));
                }
                Err(err) => {
                    let line = json!({"error": err, "done": true});
                    yield Ok(ndjson(&line));
                    break;
                }
            }
        }
    });
    let mut response = Response::new(body);
    let headers = response.headers_mut();
    set_header(headers, "Content-Type", "application/x-ndjson");
    set_header(headers, "Cache-Control", "no-store");
    set_header(headers, "X-Accel-Buffering", "no");
    Ok(response)
}

async fn ensure_model(
    State(state): State<AppState>,
    Json(payload): Json<EnsureRequest>,
) -> ApiResult<Json<Value>> {
    state
        .engine
        .ensure(&payload.engine, &payload.voice)
        .await
        .map_err(|err| error(status_for(&err), &err))?;
    Ok(Json(json!({
        "ok": true,
        "engine": payload.engine,
        "voice": payload.voice,
    })))
}

async fn clear_cache(State(state): State<AppState>) -> Json<Value> {
    state.engine.clear_cache();
    Json(json!({"ok": true}))
}

async fn require_token(
    State(state): State<AppState>,
    headers: HeaderMap,
    request: axum::extract::Request,
    next: Next,
) -> Response {
    let token = header_str(&headers, "x-ovml-token")
        .map(str::to_string)
        .or_else(|| {
            header_str(&headers, "authorization")
                .filter(|value| value.to_ascii_lowercase().starts_with("bearer "))
                .map(|value| value[7..].trim().to_string())
        });
    match token {
        Some(token) if tokens_equal(&token, &state.token) => next.run(request).await,
        _ => error(StatusCode::UNAUTHORIZED, "unauthorized").into_response(),
    }
}

async fn cors(request: axum::extract::Request, next: Next) -> Response {
    let origin = header_str(request.headers(), "origin").map(str::to_string);
    let preflight = request.method() == Method::OPTIONS;
    if preflight {
        return match origin.as_deref() {
            Some(origin) if origin_allowed(Some(origin)) => {
                let mut response = StatusCode::NO_CONTENT.into_response();
                apply_cors(response.headers_mut(), origin, true);
                response
            }
            _ => error(StatusCode::FORBIDDEN, "origin not allowed").into_response(),
        };
    }
    let mut response = next.run(request).await;
    if let Some(origin) = origin.as_deref() {
        if origin_allowed(Some(origin)) {
            apply_cors(response.headers_mut(), origin, false);
        }
    }
    response
}

fn apply_cors(headers: &mut HeaderMap, origin: &str, preflight: bool) {
    set_header(headers, "Access-Control-Allow-Origin", origin);
    set_header(
        headers,
        "Access-Control-Allow-Methods",
        "GET, POST, OPTIONS, DELETE",
    );
    set_header(
        headers,
        "Access-Control-Allow-Headers",
        "content-type, x-ovml-token, authorization",
    );
    set_header(headers, "Access-Control-Expose-Headers", EXPOSED_HEADERS);
    set_header(headers, "Access-Control-Max-Age", "600");
    let current = headers
        .get("vary")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    set_header(
        headers,
        "Vary",
        &match current {
            Some(value) if !value.is_empty() => format!("{value}, Origin"),
            _ => "Origin".into(),
        },
    );
    if preflight {
        set_header(headers, "Access-Control-Allow-Private-Network", "true");
    }
}

fn origin_allowed(origin: Option<&str>) -> bool {
    let Some(origin) = origin else {
        return false;
    };
    if matches!(
        origin,
        "https://openvml.com"
            | "https://www.openvml.com"
            | "tauri://localhost"
            | "https://tauri.localhost"
    ) {
        return true;
    }
    let (scheme, rest) = if let Some(rest) = origin.strip_prefix("https://") {
        ("https", rest)
    } else if let Some(rest) = origin.strip_prefix("http://") {
        ("http", rest)
    } else {
        return false;
    };
    let authority = rest.split('/').next().unwrap_or(rest);
    let host = authority
        .split(':')
        .next()
        .unwrap_or(authority)
        .to_ascii_lowercase();
    if scheme == "https" {
        return host == "openvml.com" || host.ends_with(".openvml.com");
    }
    matches!(host.as_str(), "127.0.0.1" | "localhost")
}

fn words_header(words: &[Word]) -> (Option<String>, bool) {
    if words.is_empty() {
        return (None, false);
    }
    let total = words.len();
    let mut current = total;
    while current > 0 {
        let encoded = serde_json::to_vec(&words[..current]).unwrap_or_default();
        let value = base64::engine::general_purpose::STANDARD.encode(encoded);
        if value.len() <= MAX_HEADER_WORDS {
            return (Some(value), current < total);
        }
        if current == 1 {
            return (None, true);
        }
        current = ((current as f64) * 0.8).max(1.0) as usize;
    }
    (None, true)
}

fn ndjson(value: &Value) -> Bytes {
    let mut line = serde_json::to_vec(value).unwrap_or_default();
    line.push(b'\n');
    Bytes::from(line)
}

fn header_str<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

fn set_header(headers: &mut HeaderMap, name: &'static str, value: &str) {
    if let Ok(value) = HeaderValue::from_str(value) {
        headers.insert(name, value);
    }
}

fn tokens_equal(left: &str, right: &str) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.bytes()
        .zip(right.bytes())
        .fold(0u8, |acc, (a, b)| acc | (a ^ b))
        == 0
}

fn error(status: StatusCode, message: &str) -> ApiError {
    (status, Json(json!({"detail": message})))
}

fn status_for(message: &str) -> StatusCode {
    let lower = message.to_ascii_lowercase();
    if lower.contains("required")
        || lower.contains("unknown voice")
        || lower.contains("no voices")
        || lower.contains("is not available")
        || lower.contains("unknown engine")
    {
        StatusCode::BAD_REQUEST
    } else {
        StatusCode::BAD_GATEWAY
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_allowlist_works() {
        assert!(origin_allowed(Some("https://openvml.com")));
        assert!(origin_allowed(Some("https://foo.openvml.com")));
        assert!(origin_allowed(Some("http://127.0.0.1:5173")));
        assert!(origin_allowed(Some("http://localhost:3000")));
        assert!(!origin_allowed(Some("https://evil.com")));
        assert!(!origin_allowed(Some("http://openvml.com")));
        assert!(!origin_allowed(None));
    }

    #[test]
    fn token_comparison_is_exact() {
        assert!(tokens_equal("abc", "abc"));
        assert!(!tokens_equal("abc", "abd"));
        assert!(!tokens_equal("abc", "ab"));
    }

    #[test]
    fn words_header_truncates_long_lists() {
        let words: Vec<Word> = (0..2000)
            .map(|index| Word {
                text: format!("word{index}"),
                start_ms: index,
                end_ms: index + 1,
            })
            .collect();
        let (value, truncated) = words_header(&words);
        assert!(value.is_none() || value.unwrap().len() <= MAX_HEADER_WORDS);
        assert!(truncated);
    }
}
