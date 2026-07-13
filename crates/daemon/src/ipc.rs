use std::{io, os::unix::fs::PermissionsExt, path::Path, sync::Arc};

use llm_meter_core::{CORE_VERSION, IPC_VERSION, SCHEMA_VERSION};
use llm_meter_storage::Repository;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{UnixListener, UnixStream},
};

use crate::scheduler::ProviderRuntime;
use crate::snapshot;

#[derive(Debug, Deserialize)]
struct Request {
    jsonrpc: String,
    #[serde(default)]
    id: Value,
    method: String,
    #[serde(default)]
    params: Value,
}
#[derive(Debug, Serialize)]
struct Response {
    jsonrpc: &'static str,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<RpcError>,
}
#[derive(Debug, Serialize)]
struct RpcError {
    code: i32,
    message: String,
}

pub struct Server {
    repo: Arc<Repository>,
    runtime: Option<Arc<ProviderRuntime>>,
}
impl Server {
    pub fn new(repo: Arc<Repository>) -> Self {
        Self {
            repo,
            runtime: None,
        }
    }
    pub fn with_runtime(repo: Arc<Repository>, runtime: Arc<ProviderRuntime>) -> Self {
        Self {
            repo,
            runtime: Some(runtime),
        }
    }
    pub async fn serve(self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
            tokio::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700)).await?;
        }
        match tokio::fs::remove_file(path).await {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        let listener = UnixListener::bind(path)?;
        tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).await?;
        loop {
            let (stream, _) = listener.accept().await?;
            let repo = self.repo.clone();
            let runtime = self.runtime.clone();
            tokio::spawn(async move {
                if let Err(e) = handle(stream, repo, runtime).await {
                    tracing::warn!(component="ipc",error_code="client_io",error=%e,"IPC client disconnected")
                }
            });
        }
    }
}

async fn handle(
    stream: UnixStream,
    repo: Arc<Repository>,
    runtime: Option<Arc<ProviderRuntime>>,
) -> io::Result<()> {
    let (read, mut write) = stream.into_split();
    let mut lines = BufReader::new(read).lines();
    while let Some(line) = lines.next_line().await? {
        let started = std::time::Instant::now();
        let response = match serde_json::from_str::<Request>(&line) {
            Ok(r) => dispatch(&repo, runtime.clone(), r).await,
            Err(_) => Response {
                jsonrpc: "2.0",
                id: Value::Null,
                result: None,
                error: Some(RpcError {
                    code: -32700,
                    message: "parse error".into(),
                }),
            },
        };
        crate::telemetry::record_ipc(started.elapsed().as_millis() as u64);
        let mut bytes = serde_json::to_vec(&response).map_err(io::Error::other)?;
        bytes.push(b'\n');
        write.write_all(&bytes).await?;
    }
    Ok(())
}

async fn dispatch(
    repo: &Repository,
    runtime: Option<Arc<ProviderRuntime>>,
    r: Request,
) -> Response {
    let id = r.id;
    if r.jsonrpc != "2.0" {
        return err(id, -32600, "invalid request");
    }
    let result: Result<Value, (i32, &str)> = match r.method.as_str() {
        "system/version" => Ok(
            json!({"core_version":CORE_VERSION,"ipc_version":IPC_VERSION,"schema_version":repo.schema_version().unwrap_or(SCHEMA_VERSION)}),
        ),
        "system/health" => Ok(
            json!({"status":"ok","database":repo.schema_version().is_ok(),"operational_metrics":crate::telemetry::snapshot()}),
        ),
        "providers/list" => Ok(json!(
            runtime
                .as_ref()
                .map(|v| v.provider_manifests())
                .unwrap_or_default()
        )),
        "connections/list" => repo
            .list_connections()
            .map(|v| json!(v.into_iter().map(public_connection).collect::<Vec<_>>()))
            .map_err(|_| (-32603, "repository error")),
        "connections/capabilities" => param_id(&r.params)
            .and_then(|v| {
                repo.capabilities(v)
                    .map_err(|_| (-32603, "repository error"))
            })
            .map(|v| json!(v)),
        "snapshot/get" => snapshot::build_with_stale_after(
            repo,
            runtime
                .as_ref()
                .map(|v| v.stale_after_seconds())
                .unwrap_or(1800),
        )
        .map(|v| json!(v))
        .map_err(|_| (-32603, "snapshot error")),
        "metrics/query" => param_id(&r.params)
            .and_then(|v| {
                repo.metrics(
                    v,
                    r.params.get("metric_key").and_then(Value::as_str),
                    r.params.get("limit").and_then(Value::as_u64).unwrap_or(500) as usize,
                )
                .map_err(|_| (-32603, "repository error"))
            })
            .map(|v| json!(v)),
        "quotas/list" => param_id(&r.params)
            .and_then(|v| repo.quotas(v).map_err(|_| (-32603, "repository error")))
            .map(|v| json!(v)),
        "waybar/render" => snapshot::build_with_stale_after(
            repo,
            runtime
                .as_ref()
                .map(|v| v.stale_after_seconds())
                .unwrap_or(1800),
        )
        .map(|v| json!(snapshot::render_waybar(&v)))
        .map_err(|_| (-32603, "snapshot error")),
        "budgets/get" => param_id(&r.params)
            .and_then(|id| repo.budget(id).map_err(|_| (-32603, "repository error")))
            .map(|value| json!(value)),
        "budgets/set" => serde_json::from_value::<llm_meter_core::Budget>(r.params.clone())
            .map_err(|_| (-32602, "invalid budget"))
            .and_then(|budget| {
                repo.set_budget(&budget)
                    .map_err(|_| (-32603, "repository error"))?;
                let existing = repo
                    .alerts(Some(budget.connection_id))
                    .map_err(|_| (-32603, "repository error"))?;
                for threshold in [budget.warning_ratio, budget.critical_ratio] {
                    if !existing.iter().any(|v| {
                        v.kind == llm_meter_core::AlertKind::CostBudgetRatio
                            && v.threshold == threshold
                    }) {
                        repo.upsert_alert(&llm_meter_core::AlertRule {
                            id: uuid::Uuid::new_v4(),
                            connection_id: budget.connection_id,
                            kind: llm_meter_core::AlertKind::CostBudgetRatio,
                            threshold,
                            state: llm_meter_core::AlertState::Normal,
                            last_triggered_at: None,
                            suppressed_until: None,
                        })
                        .map_err(|_| (-32603, "repository error"))?;
                    }
                }
                let _ = crate::alerts::evaluate(repo, budget.connection_id);
                Ok(json!(budget))
            }),
        "alerts/list" => {
            let connection = r
                .params
                .get("connection_id")
                .and_then(Value::as_str)
                .map(uuid::Uuid::parse_str)
                .transpose()
                .map_err(|_| (-32602, "invalid connection_id"));
            connection
                .and_then(|id| repo.alerts(id).map_err(|_| (-32603, "repository error")))
                .map(|value| json!(value))
        }
        "alerts/upsert" => serde_json::from_value::<llm_meter_core::AlertRule>(r.params.clone())
            .map_err(|_| (-32602, "invalid alert"))
            .and_then(|alert| {
                repo.upsert_alert(&alert)
                    .map_err(|_| (-32603, "repository error"))?;
                Ok(json!(alert))
            }),
        "connections/refresh" => match (runtime.as_ref(), param_id(&r.params)) {
            (Some(runtime), Ok(id)) => runtime
                .force_sync(id)
                .await
                .map(|_| json!({"completed":true}))
                .map_err(|e| (-32010, e.code())),
            (None, _) => Err((-32001, "provider runtime unavailable")),
            (_, Err(e)) => Err(e),
        },
        "connections/refresh-all" => match runtime.as_ref() {
            Some(runtime) => {
                let connections = repo
                    .list_connections()
                    .map_err(|_| (-32603, "repository error"));
                match connections {
                    Ok(connections) => {
                        let mut refreshed = Vec::new();
                        let mut failed = Vec::new();
                        for connection in connections {
                            match runtime.force_sync(connection.id).await {
                                Ok(()) => refreshed.push(connection.id.to_string()),
                                Err(error) => failed.push(json!({
                                    "connection_id": connection.id,
                                    "error_code": error.code(),
                                })),
                            }
                        }
                        let _ = tokio::task::spawn_blocking(crate::local_codex::refresh).await;
                        Ok(json!({
                            "completed": true,
                            "refreshed": refreshed,
                            "failed": failed,
                        }))
                    }
                    Err(error) => Err(error),
                }
            }
            None => Err((-32001, "provider runtime unavailable")),
        },
        "connections/remove" => match (runtime.as_deref(), param_id(&r.params)) {
            (Some(runtime), Ok(id)) => runtime
                .remove(id)
                .await
                .map(|_| json!({"removed":true}))
                .map_err(|e| (-32010, e.code())),
            (None, _) => Err((-32001, "provider runtime unavailable")),
            (_, Err(e)) => Err(e),
        },
        "connections/add" => match runtime.as_deref() {
            Some(runtime) => {
                let provider = r.params.get("provider_id").and_then(Value::as_str);
                let request =
                    serde_json::from_value::<llm_meter_core::BeginAuthRequest>(r.params.clone());
                match (provider, request) {
                    (Some(provider), Ok(request)) => runtime
                        .begin_auth(request, provider)
                        .await
                        .map(|v| json!(v))
                        .map_err(|e| (-32010, e.code())),
                    _ => Err((-32602, "invalid connection auth request")),
                }
            }
            None => Err((-32001, "provider runtime unavailable")),
        },
        "connections/auth/complete" => match runtime.as_ref() {
            Some(runtime) => {
                let challenge = r.params.get("challenge_id").and_then(Value::as_str);
                let secret = r
                    .params
                    .get("secret")
                    .and_then(Value::as_str)
                    .map(|v| secrecy::SecretString::from(v.to_owned()));
                match challenge {
                    Some(id) => match runtime.complete_auth(id, secret).await {
                        Ok(connection) => {
                            // Authentication success should make useful data available without a
                            // second user action. The sync stays asynchronous so the auth response
                            // can update the UI immediately.
                            let _ = runtime.request_sync(connection.id, false);
                            Ok(json!(public_connection(connection)))
                        }
                        Err(error) => Err((-32010, error.code())),
                    },
                    None => Err((-32602, "challenge_id required")),
                }
            }
            None => Err((-32001, "provider runtime unavailable")),
        },
        _ => Err((-32601, "method not found")),
    };
    match result {
        Ok(v) => Response {
            jsonrpc: "2.0",
            id,
            result: Some(v),
            error: None,
        },
        Err((c, m)) => err(id, c, m),
    }
}
fn param_id(v: &Value) -> Result<uuid::Uuid, (i32, &'static str)> {
    v.get("connection_id")
        .and_then(Value::as_str)
        .ok_or((-32602, "connection_id required"))
        .and_then(|v| uuid::Uuid::parse_str(v).map_err(|_| (-32602, "invalid connection_id")))
}
fn err(id: Value, code: i32, message: &str) -> Response {
    Response {
        jsonrpc: "2.0",
        id,
        result: None,
        error: Some(RpcError {
            code,
            message: message.into(),
        }),
    }
}
fn public_connection(c: llm_meter_core::Connection) -> Value {
    json!({"id":c.id,"provider_id":c.provider_id,"connection_type":c.connection_type,"display_name":c.display_name,"account_external_id":c.account_external_id,"status":c.status,"created_at":c.created_at,"updated_at":c.updated_at,"last_success_at":c.last_success_at,"last_error_code":c.last_error_code,"disabled_at":c.disabled_at})
}

pub async fn call(path: &Path, method: &str, params: Value) -> io::Result<Value> {
    Client::connect(path).await?.call(method, params).await
}

pub struct Client {
    read: BufReader<tokio::net::unix::OwnedReadHalf>,
    write: tokio::net::unix::OwnedWriteHalf,
    next_id: u64,
}

impl Client {
    pub async fn connect(path: &Path) -> io::Result<Self> {
        let stream = UnixStream::connect(path).await?;
        let (read, write) = stream.into_split();
        let mut client = Self {
            read: BufReader::new(read),
            write,
            next_id: 0,
        };
        let version = client.call("system/version", json!({})).await?;
        if version.get("ipc_version").and_then(Value::as_u64) != Some(IPC_VERSION as u64) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "incompatible daemon IPC version",
            ));
        }
        Ok(client)
    }
    pub async fn call(&mut self, method: &str, params: Value) -> io::Result<Value> {
        self.next_id += 1;
        let req = json!({"jsonrpc":"2.0","id":self.next_id,"method":method,"params":params});
        self.write
            .write_all(
                serde_json::to_string(&req)
                    .map_err(io::Error::other)?
                    .as_bytes(),
            )
            .await?;
        self.write.write_all(b"\n").await?;
        let mut line = String::new();
        self.read.read_line(&mut line).await?;
        if line.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "daemon closed IPC connection",
            ));
        }
        let v: Value = serde_json::from_str(&line).map_err(io::Error::other)?;
        if let Some(e) = v.get("error") {
            return Err(io::Error::other(e.to_string()));
        }
        Ok(v.get("result").cloned().unwrap_or(Value::Null))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use llm_meter_core::{Connection, ConnectionStatus, CredentialRef};
    use std::os::unix::fs::PermissionsExt;
    use uuid::Uuid;
    #[tokio::test]
    async fn private_socket_and_allowlisted_connection_dto() {
        let root = std::env::temp_dir().join(format!("llm-meter-ipc-{}", Uuid::new_v4()));
        let socket = root.join("daemon.sock");
        let repo = Arc::new(Repository::in_memory().unwrap());
        let credential = CredentialRef {
            id: Uuid::new_v4(),
            backend: "memory".into(),
            service_name: "secret-service".into(),
            secret_key: "must-not-leak".into(),
            created_at: Utc::now(),
        };
        let now = Utc::now();
        let connection = Connection {
            id: Uuid::new_v4(),
            provider_id: "mock".into(),
            connection_type: "fixture".into(),
            display_name: "Mock".into(),
            account_external_id: None,
            status: ConnectionStatus::Ready,
            credential_ref_id: Some(credential.id),
            created_at: now,
            updated_at: now,
            last_success_at: None,
            last_error_code: None,
            disabled_at: None,
        };
        repo.add_authenticated_connection(&connection, Some(&credential))
            .unwrap();
        let server = Server::new(repo);
        let path = socket.clone();
        let task = tokio::spawn(async move { server.serve(&path).await });
        for _ in 0..50 {
            if socket.exists() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert_eq!(
            std::fs::metadata(&root).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(&socket).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let list = call(&socket, "connections/list", json!({})).await.unwrap();
        assert!(list[0].get("credential_ref_id").is_none());
        assert!(!list.to_string().contains("must-not-leak"));
        task.abort();
        let _ = std::fs::remove_dir_all(root);
    }
}
