use chrono::{DateTime, Local, NaiveDate, Utc};
use llm_meter_provider_openai::pricing::{PRICE_AS_OF, PRICE_SOURCE_URL, estimate_text_tokens};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    cmp::Reverse,
    collections::{BTreeMap, BTreeSet, HashMap},
    fs::{self, File},
    io::{BufRead, BufReader, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LocalCodexSnapshot {
    pub observed_at: DateTime<Utc>,
    pub active_sessions: Vec<LocalCodexSession>,
    pub models: Vec<LocalModelUsage>,
    pub estimated_cost_usd: Option<Decimal>,
    pub today_tokens: i64,
    pub today_estimated_cost_usd: Option<Decimal>,
    pub daily_usage: Vec<LocalDailyUsage>,
    pub pricing_as_of: String,
    pub pricing_source_url: String,
    pub weekly_quota_forecast: Option<QuotaForecast>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalDailyUsage {
    pub date: NaiveDate,
    pub models: Vec<LocalModelUsage>,
    pub total_tokens: i64,
    pub estimated_cost_usd: Option<Decimal>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalCodexSession {
    pub id: String,
    pub cwd: Option<String>,
    pub model: String,
    pub pids: Vec<u32>,
    pub input_tokens: i64,
    pub cached_input_tokens: i64,
    pub output_tokens: i64,
    pub total_tokens: i64,
    pub estimated_cost_usd: Option<Decimal>,
    pub last_activity_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LocalModelUsage {
    pub model: String,
    pub input_tokens: i64,
    pub cached_input_tokens: i64,
    pub output_tokens: i64,
    pub total_tokens: i64,
    pub estimated_cost_usd: Option<Decimal>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaForecast {
    pub used_percent: f64,
    pub resets_at: Option<DateTime<Utc>>,
    pub exhausts_at: Option<DateTime<Utc>>,
    pub sample_started_at: DateTime<Utc>,
    pub sample_ended_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, Default)]
struct TokenTotal {
    input: i64,
    cached: i64,
    output: i64,
    total: i64,
}

#[derive(Debug, Clone)]
struct QuotaSample {
    at: DateTime<Utc>,
    used: f64,
    resets_at: Option<DateTime<Utc>>,
    window_minutes: Option<i64>,
}

#[derive(Debug, Default)]
struct SessionState {
    id: String,
    cwd: Option<String>,
    model: String,
    previous: Option<TokenTotal>,
    models: BTreeMap<String, LocalModelUsage>,
    daily_models: BTreeMap<NaiveDate, BTreeMap<String, LocalModelUsage>>,
    quota_samples: Vec<QuotaSample>,
    last_activity_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Default)]
struct CachedFile {
    offset: u64,
    state: SessionState,
}

#[derive(Debug, Default)]
struct CollectorCache {
    files: HashMap<PathBuf, CachedFile>,
}

static CACHE: OnceLock<Mutex<CollectorCache>> = OnceLock::new();

/// Discover running Codex processes and incrementally ingest their session logs.
/// This is safe to call frequently: already consumed bytes are not parsed again.
pub fn refresh() {
    let _ = snapshot();
}

pub fn snapshot() -> LocalCodexSnapshot {
    let now = Utc::now();
    let active = active_session_files();
    let today = Local::now().date_naive();
    let history_start = today - chrono::Duration::days(6);
    let mut tracked_paths = active.keys().cloned().collect::<BTreeSet<_>>();
    tracked_paths.extend(recent_session_files());
    let mut cache = CACHE
        .get_or_init(|| Mutex::new(CollectorCache::default()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    cache.files.retain(|path, cached| {
        tracked_paths.contains(path)
            || cached
                .state
                .daily_models
                .keys()
                .any(|day| *day >= history_start)
    });

    for path in &tracked_paths {
        let entry = cache.files.entry(path.clone()).or_default();
        if let Ok(length) = fs::metadata(path).map(|metadata| metadata.len()) {
            if length < entry.offset {
                *entry = CachedFile::default();
            }
            if length > entry.offset {
                read_appended(path, entry);
            }
        }
    }

    let mut sessions = Vec::new();
    let mut models = BTreeMap::<String, LocalModelUsage>::new();
    let mut today_models = BTreeMap::<String, LocalModelUsage>::new();
    let mut daily_models = BTreeMap::<NaiveDate, BTreeMap<String, LocalModelUsage>>::new();
    let mut quota_samples = Vec::new();
    for (path, pids) in &active {
        let Some(cached) = cache.files.get(path) else {
            continue;
        };
        for usage in cached.state.models.values() {
            add_usage(
                models
                    .entry(usage.model.clone())
                    .or_insert_with(|| LocalModelUsage {
                        model: usage.model.clone(),
                        ..Default::default()
                    }),
                usage,
            );
        }
        quota_samples.extend(cached.state.quota_samples.iter().cloned());
        let mut session_total = LocalModelUsage::default();
        for usage in cached.state.models.values() {
            add_usage(&mut session_total, usage);
        }
        let estimated_cost_usd = cost_for_models(cached.state.models.values());
        sessions.push(LocalCodexSession {
            id: if cached.state.id.is_empty() {
                path.file_stem()
                    .and_then(|value| value.to_str())
                    .unwrap_or("codex-session")
                    .to_owned()
            } else {
                cached.state.id.clone()
            },
            cwd: cached.state.cwd.clone(),
            model: if cached.state.model.is_empty() {
                "unknown".into()
            } else {
                cached.state.model.clone()
            },
            pids: pids.iter().copied().collect(),
            input_tokens: session_total.input_tokens,
            cached_input_tokens: session_total.cached_input_tokens,
            output_tokens: session_total.output_tokens,
            total_tokens: session_total.total_tokens,
            estimated_cost_usd,
            last_activity_at: cached.state.last_activity_at,
        });
    }
    for cached in cache.files.values() {
        for (day, usages) in &cached.state.daily_models {
            if !(history_start..=today).contains(day) {
                continue;
            }
            for usage in usages.values() {
                add_usage(
                    daily_models
                        .entry(*day)
                        .or_default()
                        .entry(usage.model.clone())
                        .or_insert_with(|| LocalModelUsage {
                            model: usage.model.clone(),
                            ..Default::default()
                        }),
                    usage,
                );
            }
        }
    }
    if let Some(usages) = daily_models.get(&today) {
        today_models = usages.clone();
    }
    sessions.sort_by_key(|session| Reverse(session.last_activity_at));
    let mut models = models.into_values().collect::<Vec<_>>();
    for usage in &mut models {
        usage.estimated_cost_usd = estimate_text_tokens(
            &usage.model,
            usage.input_tokens,
            usage.cached_input_tokens,
            usage.output_tokens,
        );
    }
    models.sort_by_key(|usage| Reverse(usage.total_tokens));
    let estimated_cost_usd = cost_for_models(models.iter());
    let today_tokens = today_models.values().map(|usage| usage.total_tokens).sum();
    let today_estimated_cost_usd = cost_for_models(today_models.values());
    let daily_usage = (0..7)
        .map(|offset| {
            let date = history_start + chrono::Duration::days(offset);
            let mut models = daily_models
                .remove(&date)
                .unwrap_or_default()
                .into_values()
                .collect::<Vec<_>>();
            for usage in &mut models {
                usage.estimated_cost_usd = estimate_text_tokens(
                    &usage.model,
                    usage.input_tokens,
                    usage.cached_input_tokens,
                    usage.output_tokens,
                );
            }
            models.sort_by_key(|usage| Reverse(usage.total_tokens));
            LocalDailyUsage {
                date,
                total_tokens: models.iter().map(|usage| usage.total_tokens).sum(),
                estimated_cost_usd: cost_for_models(models.iter()),
                models,
            }
        })
        .collect();

    LocalCodexSnapshot {
        observed_at: now,
        active_sessions: sessions,
        models,
        estimated_cost_usd,
        today_tokens,
        today_estimated_cost_usd,
        daily_usage,
        pricing_as_of: PRICE_AS_OF.into(),
        pricing_source_url: PRICE_SOURCE_URL.into(),
        weekly_quota_forecast: forecast(quota_samples),
    }
}

fn active_session_files() -> BTreeMap<PathBuf, BTreeSet<u32>> {
    let mut result = BTreeMap::<PathBuf, BTreeSet<u32>>::new();
    let Ok(processes) = fs::read_dir("/proc") else {
        return result;
    };
    for process in processes.flatten() {
        let Some(pid) = process
            .file_name()
            .to_str()
            .and_then(|value| value.parse::<u32>().ok())
        else {
            continue;
        };
        let process_path = process.path();
        let is_codex = fs::read_to_string(process_path.join("comm"))
            .is_ok_and(|value| value.trim() == "codex");
        if !is_codex {
            continue;
        }
        let Ok(fds) = fs::read_dir(process_path.join("fd")) else {
            continue;
        };
        for fd in fds.flatten() {
            let Ok(target) = fs::read_link(fd.path()) else {
                continue;
            };
            if is_session_log(&target) {
                result.entry(target).or_default().insert(pid);
            }
        }
    }
    result
}

/// Include enough UTC session directories to cover the most recent seven local
/// calendar days, including timezone boundaries. Parsed files are cached, so
/// steady-state refreshes only consume appended bytes.
fn recent_session_files() -> BTreeSet<PathBuf> {
    let Some(root) = std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join(".codex/sessions"))
    else {
        return BTreeSet::new();
    };
    let mut result = BTreeSet::new();
    for offset in 0..=8 {
        let day = Utc::now().date_naive() - chrono::Duration::days(offset);
        let directory = root
            .join(day.format("%Y").to_string())
            .join(day.format("%m").to_string())
            .join(day.format("%d").to_string());
        let Ok(entries) = fs::read_dir(directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if is_session_log(&path) {
                result.insert(path);
            }
        }
    }
    result
}

fn is_session_log(path: &Path) -> bool {
    path.extension().is_some_and(|value| value == "jsonl")
        && path
            .components()
            .any(|component| component.as_os_str() == "sessions")
        && path
            .to_string_lossy()
            .contains(&format!("{}{}codex", std::path::MAIN_SEPARATOR, "."))
}

fn read_appended(path: &Path, cached: &mut CachedFile) {
    let Ok(file) = File::open(path) else {
        return;
    };
    let mut reader = BufReader::new(file);
    if reader.seek(SeekFrom::Start(cached.offset)).is_err() {
        return;
    }
    loop {
        let line_start = cached.offset;
        let mut line = Vec::new();
        let Ok(bytes) = reader.read_until(b'\n', &mut line) else {
            break;
        };
        if bytes == 0 {
            break;
        }
        if !line.ends_with(b"\n") {
            let _ = reader.seek(SeekFrom::Start(line_start));
            break;
        }
        cached.offset += bytes as u64;
        if let Ok(value) = serde_json::from_slice::<Value>(&line) {
            consume_event(&mut cached.state, &value);
        }
    }
}

fn consume_event(state: &mut SessionState, event: &Value) {
    let payload = &event["payload"];
    let event_type = if event["type"].as_str() == Some("event_msg") {
        payload["type"].as_str()
    } else {
        event["type"].as_str().or_else(|| payload["type"].as_str())
    };
    match event_type {
        Some("session_meta") => {
            state.id = payload["id"].as_str().unwrap_or_default().to_owned();
            state.cwd = payload["cwd"].as_str().map(str::to_owned);
        }
        Some("turn_context") => {
            if let Some(model) = payload["model"].as_str() {
                state.model = model.to_owned();
            }
            if state.cwd.is_none() {
                state.cwd = payload["cwd"].as_str().map(str::to_owned);
            }
        }
        Some("token_count") => consume_token_count(state, event),
        _ => {}
    }
    if let Some(at) = event["timestamp"]
        .as_str()
        .and_then(|value| value.parse::<DateTime<Utc>>().ok())
    {
        state.last_activity_at = Some(at);
    }
}

fn consume_token_count(state: &mut SessionState, event: &Value) {
    let event_at = event["timestamp"]
        .as_str()
        .and_then(|value| value.parse::<DateTime<Utc>>().ok());
    let total = &event["payload"]["info"]["total_token_usage"];
    if total.is_object() {
        let current = TokenTotal {
            input: total["input_tokens"].as_i64().unwrap_or_default(),
            cached: total["cached_input_tokens"].as_i64().unwrap_or_default(),
            output: total["output_tokens"].as_i64().unwrap_or_default(),
            total: total["total_tokens"].as_i64().unwrap_or_default(),
        };
        let previous = state.previous.unwrap_or_default();
        let reset = state.previous.is_some_and(|old| current.total < old.total);
        let delta = if reset {
            current
        } else {
            TokenTotal {
                input: current.input.saturating_sub(previous.input),
                cached: current.cached.saturating_sub(previous.cached),
                output: current.output.saturating_sub(previous.output),
                total: current.total.saturating_sub(previous.total),
            }
        };
        let model = if state.model.is_empty() {
            "unknown"
        } else {
            &state.model
        };
        let usage = state
            .models
            .entry(model.to_owned())
            .or_insert_with(|| LocalModelUsage {
                model: model.to_owned(),
                ..Default::default()
            });
        apply_delta(usage, delta);
        if let Some(day) = event_at.map(|at| at.with_timezone(&Local).date_naive()) {
            let daily = state.daily_models.entry(day).or_default();
            let usage = daily
                .entry(model.to_owned())
                .or_insert_with(|| LocalModelUsage {
                    model: model.to_owned(),
                    ..Default::default()
                });
            apply_delta(usage, delta);
        }
        state.previous = Some(current);
    }

    let primary = &event["payload"]["rate_limits"]["primary"];
    let Some(used) = primary["used_percent"].as_f64() else {
        return;
    };
    let Some(at) = event["timestamp"]
        .as_str()
        .and_then(|value| value.parse::<DateTime<Utc>>().ok())
    else {
        return;
    };
    let resets_at = primary["resets_at"]
        .as_i64()
        .and_then(|value| DateTime::from_timestamp(value, 0));
    state.quota_samples.push(QuotaSample {
        at,
        used,
        resets_at,
        window_minutes: primary["window_minutes"].as_i64(),
    });
}

fn add_usage(target: &mut LocalModelUsage, source: &LocalModelUsage) {
    target.input_tokens += source.input_tokens;
    target.cached_input_tokens += source.cached_input_tokens;
    target.output_tokens += source.output_tokens;
    target.total_tokens += source.total_tokens;
}

fn apply_delta(target: &mut LocalModelUsage, delta: TokenTotal) {
    target.input_tokens += delta.input;
    target.cached_input_tokens += delta.cached;
    target.output_tokens += delta.output;
    target.total_tokens += delta.total;
}

fn cost_for_models<'a>(models: impl Iterator<Item = &'a LocalModelUsage>) -> Option<Decimal> {
    let mut total = Decimal::ZERO;
    let mut priced = false;
    for usage in models {
        if let Some(cost) = estimate_text_tokens(
            &usage.model,
            usage.input_tokens,
            usage.cached_input_tokens,
            usage.output_tokens,
        ) {
            total += cost;
            priced = true;
        }
    }
    priced.then_some(total)
}

fn forecast(mut samples: Vec<QuotaSample>) -> Option<QuotaForecast> {
    samples.retain(|sample| sample.window_minutes == Some(7 * 24 * 60));
    samples.sort_by_key(|sample| sample.at);
    let latest = samples.last()?.clone();
    samples.retain(|sample| sample.resets_at == latest.resets_at && sample.used <= latest.used);
    let earliest = samples.iter().find(|sample| {
        latest.used - sample.used >= 1.0 && (latest.at - sample.at).num_minutes() >= 5
    })?;
    let elapsed_seconds = (latest.at - earliest.at).num_seconds();
    let consumed = latest.used - earliest.used;
    if elapsed_seconds <= 0 || consumed <= 0.0 {
        return None;
    }
    let remaining_seconds =
        ((100.0 - latest.used).max(0.0) / consumed * elapsed_seconds as f64).round() as i64;
    let candidate = latest.at + chrono::Duration::seconds(remaining_seconds);
    let exhausts_at = if latest.resets_at.is_some_and(|reset| candidate >= reset) {
        None
    } else {
        Some(candidate)
    };
    Some(QuotaForecast {
        used_percent: latest.used,
        resets_at: latest.resets_at,
        exhausts_at,
        sample_started_at: earliest.at,
        sample_ended_at: latest.at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_deltas_follow_model_switches() {
        let mut state = SessionState::default();
        for line in [
            r#"{"timestamp":"2026-07-13T00:00:00Z","payload":{"type":"turn_context","model":"gpt-5"}}"#,
            r#"{"timestamp":"2026-07-13T00:01:00Z","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":50,"output_tokens":20,"total_tokens":120}}}}"#,
            r#"{"timestamp":"2026-07-13T00:02:00Z","payload":{"type":"turn_context","model":"gpt-5-mini"}}"#,
            r#"{"timestamp":"2026-07-13T00:03:00Z","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":150,"cached_input_tokens":70,"output_tokens":30,"total_tokens":180}}}}"#,
        ] {
            consume_event(&mut state, &serde_json::from_str(line).unwrap());
        }
        assert_eq!(state.models["gpt-5"].total_tokens, 120);
        assert_eq!(state.models["gpt-5-mini"].total_tokens, 60);
    }
}
