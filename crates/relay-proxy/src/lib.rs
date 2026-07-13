use axum::{
    Router,
    body::{Body, Bytes},
    extract::{Request, State},
    http::{HeaderMap, HeaderName, StatusCode},
    response::Response,
    routing::any,
};
use chrono::Utc;
use futures::StreamExt;
use llm_meter_core::{
    MeasuredAmount, MetricUnit, Provenance, SyncBatch, UsageEvent, pricing::estimate_text_tokens,
};
use llm_meter_storage::Repository;
use reqwest::Client;
use rust_decimal::Decimal;
use secrecy::{ExposeSecret, SecretString};
use serde_json::Value;
use std::{collections::BTreeMap, net::SocketAddr, sync::Arc};
use url::Url;
use uuid::Uuid;

#[derive(Clone)]
pub struct ProxyConfig {
    pub connection_id: Uuid,
    pub listen: SocketAddr,
    pub origin: Url,
    pub token_authenticator: Arc<dyn ProxyTokenAuthenticator>,
    pub upstream_token: SecretString,
}

pub trait ProxyTokenAuthenticator: Send + Sync {
    fn authenticate(&self, token: &str) -> Option<String>;
}

#[derive(Clone)]
struct AppState {
    config: ProxyConfig,
    client: Client,
    repository: Arc<Repository>,
}

#[derive(Clone)]
struct ClientIdentity(String);

pub async fn serve(
    config: ProxyConfig,
    repository: Arc<Repository>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if !config.listen.ip().is_loopback() {
        return Err("relay proxy must listen on loopback".into());
    }
    if config.origin.scheme() != "https"
        && !config.origin.host_str().is_some_and(|host| {
            host == "localhost"
                || host
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback())
        })
    {
        return Err("relay proxy origin must use HTTPS".into());
    }
    let state = AppState {
        config,
        client: Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()?,
        repository,
    };
    let listener = tokio::net::TcpListener::bind(state.config.listen).await?;
    axum::serve(
        listener,
        Router::new()
            .route("/{*path}", any(forward))
            .with_state(state),
    )
    .await?;
    Ok(())
}

async fn forward(State(state): State<AppState>, request: Request) -> Response {
    let Some(client_name) =
        authenticate(request.headers(), state.config.token_authenticator.as_ref())
    else {
        return status(StatusCode::UNAUTHORIZED);
    };
    let (parts, body) = request.into_parts();
    let body = match axum::body::to_bytes(body, 16 * 1024 * 1024).await {
        Ok(body) => body,
        Err(_) => return status(StatusCode::PAYLOAD_TOO_LARGE),
    };
    let path = parts
        .uri
        .path_and_query()
        .map(|value| value.as_str())
        .unwrap_or("/");
    let target = match state.config.origin.join(path.trim_start_matches('/')) {
        Ok(url) => url,
        Err(_) => return status(StatusCode::BAD_GATEWAY),
    };
    let mut upstream = state
        .client
        .request(parts.method.clone(), target)
        .bearer_auth(state.config.upstream_token.expose_secret())
        .body(body);
    for (name, value) in &parts.headers {
        if forward_header(name) {
            upstream = upstream.header(name, value);
        }
    }
    let upstream = match upstream.send().await {
        Ok(response) => response,
        Err(_) => return status(StatusCode::BAD_GATEWAY),
    };
    let status_code = upstream.status();
    let headers = upstream.headers().clone();
    let statistical = status_code.is_success() && statistical_path(parts.uri.path());
    let content_type = headers
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_owned();
    let mut response = Response::builder().status(status_code);
    for (name, value) in &headers {
        if forward_header(name) {
            response = response.header(name, value);
        }
    }
    if statistical && content_type.contains("text/event-stream") {
        let connection_id = state.config.connection_id;
        let repository = state.repository.clone();
        let client_name = client_name.0;
        let stream =
            upstream
                .bytes_stream()
                .scan(SseUsageCollector::default(), move |collector, chunk| {
                    let event = chunk
                        .as_ref()
                        .ok()
                        .and_then(|bytes| collector.consume(connection_id, bytes, &client_name));
                    if let Some(event) = event {
                        commit_usage(&repository, connection_id, event);
                    }
                    futures::future::ready(Some(chunk))
                });
        return response
            .body(Body::from_stream(stream))
            .unwrap_or_else(|_| status(StatusCode::BAD_GATEWAY));
    }
    let bytes = match upstream.bytes().await {
        Ok(bytes) => bytes,
        Err(_) => return status(StatusCode::BAD_GATEWAY),
    };
    if statistical {
        if let Some(event) = parse_usage(
            state.config.connection_id,
            &bytes,
            &content_type,
            &client_name.0,
        ) {
            commit_usage(&state.repository, state.config.connection_id, event);
        }
    }
    response
        .body(Body::from(bytes))
        .unwrap_or_else(|_| status(StatusCode::BAD_GATEWAY))
}

fn authenticate(
    headers: &HeaderMap,
    authenticator: &dyn ProxyTokenAuthenticator,
) -> Option<ClientIdentity> {
    headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .and_then(|token| authenticator.authenticate(token))
        .map(ClientIdentity)
}
fn forward_header(name: &HeaderName) -> bool {
    !matches!(
        name.as_str(),
        "authorization"
            | "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
            | "host"
            | "content-length"
    )
}
fn statistical_path(path: &str) -> bool {
    matches!(
        path,
        "/v1/chat/completions" | "/v1/responses" | "/v1/embeddings"
    )
}
fn status(code: StatusCode) -> Response {
    Response::builder()
        .status(code)
        .body(Body::empty())
        .unwrap()
}

fn commit_usage(repository: &Repository, connection_id: Uuid, event: UsageEvent) {
    let _ = repository.commit_sync_batch(
        connection_id,
        "proxy",
        &SyncBatch {
            usage_events: vec![event],
            ..Default::default()
        },
    );
}

#[derive(Default)]
struct SseUsageCollector {
    pending: Vec<u8>,
}

impl SseUsageCollector {
    fn consume(
        &mut self,
        connection_id: Uuid,
        chunk: &Bytes,
        client_name: &str,
    ) -> Option<UsageEvent> {
        self.pending.extend_from_slice(chunk);
        let complete_len = self
            .pending
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map(|index| index + 1)?;
        let complete = self.pending.drain(..complete_len).collect::<Vec<_>>();
        parse_sse(&complete).and_then(|value| usage_event(connection_id, &value, client_name))
    }
}

pub fn parse_usage(
    connection_id: Uuid,
    body: &Bytes,
    content_type: &str,
    client_name: &str,
) -> Option<UsageEvent> {
    let value = if content_type.contains("text/event-stream") {
        parse_sse(body)?
    } else {
        serde_json::from_slice(body).ok()?
    };
    usage_event(connection_id, &value, client_name)
}

fn parse_sse(body: &[u8]) -> Option<Value> {
    std::str::from_utf8(body)
        .ok()?
        .lines()
        .filter_map(|line| line.strip_prefix("data:").map(str::trim))
        .filter(|data| *data != "[DONE]")
        .filter_map(|data| serde_json::from_str(data).ok())
        .rfind(|value: &Value| {
            value.get("usage").is_some() || value.pointer("/response/usage").is_some()
        })
}
fn usage_event(connection_id: Uuid, value: &Value, client_name: &str) -> Option<UsageEvent> {
    let root = value.get("response").unwrap_or(value);
    let usage = root.get("usage")?;
    let input = integer(usage, "prompt_tokens").or_else(|| integer(usage, "input_tokens"));
    let output = integer(usage, "completion_tokens").or_else(|| integer(usage, "output_tokens"));
    let cached = usage
        .pointer("/prompt_tokens_details/cached_tokens")
        .and_then(Value::as_i64)
        .or_else(|| {
            usage
                .pointer("/input_tokens_details/cached_tokens")
                .and_then(Value::as_i64)
        });
    let reasoning = usage
        .pointer("/completion_tokens_details/reasoning_tokens")
        .and_then(Value::as_i64)
        .or_else(|| {
            usage
                .pointer("/output_tokens_details/reasoning_tokens")
                .and_then(Value::as_i64)
        });
    let total = integer(usage, "total_tokens").or_else(|| match (input, output) {
        (Some(a), Some(b)) => Some(a + b),
        _ => None,
    });
    let charge = usage
        .get("cost")
        .or_else(|| root.get("cost"))
        .and_then(decimal)
        .map(|value| MeasuredAmount {
            value,
            unit: MetricUnit::Credit,
        });
    let model = root.get("model").and_then(Value::as_str).map(str::to_owned);
    let estimated_charge = model.as_deref().and_then(|model| {
        estimate_text_tokens(
            "openai",
            model,
            input.unwrap_or_default(),
            cached.unwrap_or_default(),
            output.unwrap_or_default(),
        )
    });
    let observed = Utc::now();
    let external_id = root
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let event = UsageEvent {
        id: Uuid::new_v4(),
        connection_id,
        external_id,
        occurred_at: observed,
        observed_at: observed,
        model,
        input_tokens: input,
        cached_input_tokens: cached,
        output_tokens: output,
        reasoning_tokens: reasoning,
        total_tokens: total,
        request_count: 1,
        actual_charge: charge,
        upstream_charge: None,
        estimated_charge: estimated_charge.map(|value| MeasuredAmount {
            value,
            unit: MetricUnit::Currency("USD".into()),
        }),
        credit_used: None,
        provenance: Provenance::LocallyObserved,
        source_event: "proxy.response".into(),
        dimensions: BTreeMap::from([("proxy_client".into(), client_name.into())]),
    };
    event.validate().ok().map(|_| event)
}
fn integer(value: &Value, key: &str) -> Option<i64> {
    value.get(key).and_then(Value::as_i64)
}
fn decimal(value: &Value) -> Option<Decimal> {
    match value {
        Value::String(value) => value.parse().ok(),
        Value::Number(value) => value.to_string().parse().ok(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_non_streaming_chat_usage() {
        let body = Bytes::from_static(br#"{"id":"chat-1","model":"gpt-test","usage":{"prompt_tokens":10,"completion_tokens":4,"total_tokens":14,"prompt_tokens_details":{"cached_tokens":3},"completion_tokens_details":{"reasoning_tokens":2},"cost":0.1}}"#);
        let event = parse_usage(Uuid::nil(), &body, "application/json", "Test Client").unwrap();
        assert_eq!(event.input_tokens, Some(10));
        assert_eq!(event.cached_input_tokens, Some(3));
        assert_eq!(event.reasoning_tokens, Some(2));
        assert_eq!(event.actual_charge.unwrap().value, Decimal::new(1, 1));
        assert_eq!(
            event.dimensions.get("proxy_client").map(String::as_str),
            Some("Test Client")
        );
    }

    #[test]
    fn estimates_known_model_cost_without_provider_charge() {
        let body = Bytes::from_static(br#"{"id":"resp-1","model":"gpt-5.6-sol","usage":{"input_tokens":1000000,"output_tokens":100000,"total_tokens":1100000,"input_tokens_details":{"cached_tokens":800000}}}"#);
        let event = parse_usage(Uuid::nil(), &body, "application/json", "Test Client").unwrap();
        assert_eq!(event.actual_charge, None);
        let estimate = event.estimated_charge.unwrap();
        assert_eq!(estimate.value, Decimal::new(44, 1));
        assert_eq!(estimate.unit, MetricUnit::Currency("USD".into()));
    }

    #[test]
    fn collects_usage_split_across_sse_chunks() {
        let mut collector = SseUsageCollector::default();
        assert!(
            collector
                .consume(
                    Uuid::nil(),
                    &Bytes::from_static(b"data: {\"type\":\"response.completed\",\"res"),
                    "Test Client"
                )
                .is_none()
        );
        let event = collector
            .consume(
                Uuid::nil(),
                &Bytes::from_static(b"ponse\":{\"id\":\"resp-2\",\"model\":\"gpt-5.6-luna\",\"usage\":{\"input_tokens\":10,\"output_tokens\":2}}}\n\n"),
                "Test Client",
            )
            .unwrap();
        assert_eq!(event.total_tokens, Some(12));
        assert!(event.estimated_charge.is_some());
    }

    #[test]
    fn parses_final_sse_usage_without_content() {
        let body = Bytes::from_static(b"data: {\"choices\":[{\"delta\":{\"content\":\"secret\"}}]}\n\ndata: {\"id\":\"chat-2\",\"usage\":{\"prompt_tokens\":2,\"completion_tokens\":1}}\n\ndata: [DONE]\n\n");
        let event = parse_usage(Uuid::nil(), &body, "text/event-stream", "Test Client").unwrap();
        assert_eq!(event.total_tokens, Some(3));
        assert!(!serde_json::to_string(&event).unwrap().contains("secret"));
    }

    #[test]
    fn rejects_non_loopback_listener() {
        let address = "0.0.0.0:18456".parse::<SocketAddr>().unwrap();
        assert!(!address.ip().is_loopback());
    }
}
