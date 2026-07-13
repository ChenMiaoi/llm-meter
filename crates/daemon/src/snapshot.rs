use chrono::{DateTime, Duration, NaiveDate, Utc};
use llm_meter_core::{
    AccountRecord, AlertRule, Budget, ConnectionStatus, MetricSample, ProductRecord, QuotaWindow,
    RateLimitResetCredits,
};
use llm_meter_storage::{Repository, StorageError};
use serde::{Deserialize, Serialize};

use crate::local_codex::LocalCodexSnapshot;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub generated_at: DateTime<Utc>,
    pub connections: Vec<ConnectionSnapshot>,
    pub local_codex: LocalCodexSnapshot,
}

/// Explicit allowlist DTO: credential references and provider raw responses are
/// intentionally absent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionSnapshot {
    pub id: String,
    pub provider_id: String,
    pub connection_type: String,
    pub display_name: String,
    pub status: ConnectionStatus,
    pub last_success_at: Option<DateTime<Utc>>,
    pub stale: bool,
    pub capabilities: Vec<String>,
    pub accounts: Vec<AccountRecord>,
    pub products: Vec<ProductRecord>,
    pub metrics: Vec<MetricSample>,
    pub quota_windows: Vec<QuotaWindow>,
    pub rate_limit_reset_credits: Option<RateLimitResetCredits>,
    pub budget: Option<Budget>,
    pub alerts: Vec<AlertRule>,
}

pub fn build(repo: &Repository) -> Result<Snapshot, StorageError> {
    build_with_stale_after(repo, 1800)
}

pub fn build_with_stale_after(
    repo: &Repository,
    stale_after_seconds: i64,
) -> Result<Snapshot, StorageError> {
    let now = Utc::now();
    let mut out = Vec::new();
    for c in repo.list_connections()? {
        let capabilities = repo
            .capabilities(c.id)?
            .map(|v| {
                v.capabilities
                    .names()
                    .into_iter()
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        let stale = matches!(
            c.status,
            ConnectionStatus::Stale | ConnectionStatus::Offline
        ) || c
            .last_success_at
            .is_some_and(|t| now.signed_duration_since(t).num_seconds() > stale_after_seconds);
        out.push(ConnectionSnapshot {
            id: c.id.to_string(),
            provider_id: c.provider_id,
            connection_type: c.connection_type,
            display_name: c.display_name,
            status: c.status,
            last_success_at: c.last_success_at,
            stale,
            capabilities,
            accounts: repo.accounts(c.id)?,
            products: repo.products(c.id)?,
            metrics: repo.metrics(c.id, None, 500)?,
            quota_windows: repo.quotas(c.id)?,
            rate_limit_reset_credits: repo.rate_limit_reset_credits(c.id)?,
            budget: repo.budget(c.id)?,
            alerts: repo.alerts(Some(c.id))?,
        });
    }
    let oldest_age = out
        .iter()
        .filter_map(|connection| connection.last_success_at)
        .map(|value| now.signed_duration_since(value).num_seconds().max(0) as u64)
        .max()
        .unwrap_or(0);
    crate::telemetry::record_snapshot_age(oldest_age);
    Ok(Snapshot {
        generated_at: now,
        connections: out,
        local_codex: crate::local_codex::snapshot(),
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WaybarOutput {
    pub text: String,
    pub tooltip: String,
    pub class: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub percentage: Option<u8>,
    pub account_label: Option<String>,
    pub today_tokens: i64,
    pub today_estimated_cost_usd: Option<rust_decimal::Decimal>,
    pub active_codex_sessions: usize,
    pub trend: Option<String>,
}

pub fn render_waybar(s: &Snapshot) -> WaybarOutput {
    let mut classes = Vec::<String>::new();
    if s.connections.iter().any(|c| c.provider_id == "openai") {
        classes.push("provider-openai".into());
    }
    let problematic = s.connections.iter().find(|c| {
        matches!(
            c.status,
            ConnectionStatus::AuthRequired
                | ConnectionStatus::ProviderError
                | ConnectionStatus::Offline
                | ConnectionStatus::Stale
                | ConnectionStatus::RateLimited
        )
    });
    if let Some(c) = problematic {
        let class = match c.status {
            ConnectionStatus::AuthRequired => "auth-required",
            ConnectionStatus::Offline => "offline",
            ConnectionStatus::Stale => "sync-stale",
            ConnectionStatus::RateLimited => "rate-limited",
            _ => "provider-error",
        };
        classes.push(class.into());
    }
    if s.connections.iter().any(|c| c.stale) && !classes.iter().any(|v| v == "sync-stale") {
        classes.push("sync-stale".into());
    }
    let quota = s
        .connections
        .iter()
        .flat_map(|c| c.quota_windows.iter().map(move |q| (c, q)))
        .filter_map(|(c, q)| q.remaining_ratio.map(|v| (c, q, v)))
        .min_by(|a, b| a.2.cmp(&b.2));
    let (mut text, percentage) = if let Some((c, _q, ratio)) = quota {
        let p = (ratio * rust_decimal::Decimal::ONE_HUNDRED)
            .round()
            .to_string()
            .parse::<u8>()
            .unwrap_or(100)
            .min(100);
        if p == 0 {
            classes.push("quota-exhausted".into())
        } else if p < 20 {
            classes.push("quota-critical".into())
        } else if p <= 50 {
            classes.push("quota-warning".into())
        } else {
            classes.push("quota-ok".into())
        };
        (format!("{} {}%", short(&c.display_name), p), Some(p))
    } else if let Some(c) = problematic {
        (
            format!("{} {}", short(&c.display_name), status_label(c.status)),
            None,
        )
    } else if s.connections.is_empty() {
        classes.push("empty".into());
        ("LLM --".into(), None)
    } else {
        classes.push("quota-ok".into());
        ("LLM Meter".into(), None)
    };
    let trend = token_sparkline(s);
    if let Some(sparkline) = &trend {
        text.push(' ');
        text.push_str(sparkline);
    }
    let mut tooltip = s
        .connections
        .iter()
        .map(|c| {
            let age = c
                .last_success_at
                .map(|v| format!("last sync {}", v.to_rfc3339()))
                .unwrap_or_else(|| "not synchronized".into());
            format!("{} · {} · {}", c.display_name, status_label(c.status), age)
        })
        .collect::<Vec<_>>()
        .join("\n");
    if !s.local_codex.active_sessions.is_empty() {
        let models = s
            .local_codex
            .models
            .iter()
            .map(|usage| usage.model.as_str())
            .collect::<Vec<_>>()
            .join(" · ");
        let local = format!(
            "本地 Codex · {} 个运行中{}",
            s.local_codex.active_sessions.len(),
            if models.is_empty() {
                String::new()
            } else {
                format!(" · {models}")
            }
        );
        if !tooltip.is_empty() {
            tooltip.push('\n');
        }
        tooltip.push_str(&local);
    }
    WaybarOutput {
        text,
        tooltip,
        class: classes,
        percentage,
        account_label: s
            .connections
            .first()
            .map(|connection| connection.display_name.clone()),
        today_tokens: s.local_codex.today_tokens,
        today_estimated_cost_usd: s.local_codex.today_estimated_cost_usd,
        active_codex_sessions: s.local_codex.active_sessions.len(),
        trend,
    }
}

/// Render the most recent seven days as a compact bar-friendly trend. Local
/// Codex history is preferred because it is refreshed continuously; provider
/// daily totals remain the fallback for installations without local history.
fn token_sparkline(s: &Snapshot) -> Option<String> {
    use llm_meter_core::MetricKey;
    use rust_decimal::Decimal;
    use std::collections::BTreeMap;

    let today = s.generated_at.date_naive();
    let start = today - Duration::days(6);
    let mut days = BTreeMap::<NaiveDate, Decimal>::new();

    let local_start = s.local_codex.daily_usage.len().saturating_sub(7);
    let local_values = s.local_codex.daily_usage[local_start..]
        .iter()
        .map(|usage| Decimal::from(usage.total_tokens))
        .collect::<Vec<_>>();
    if local_values.iter().any(|value| *value > Decimal::ZERO) {
        return sparkline(local_values);
    }

    for connection in &s.connections {
        let daily_totals = connection.metrics.iter().filter(|metric| {
            metric.metric_key.0 == MetricKey::TOKEN_TOTAL && metric.period_start.is_some()
        });
        let has_daily_totals = daily_totals.clone().any(|metric| {
            metric
                .period_start
                .is_some_and(|time| (start..=today).contains(&time.date_naive()))
        });
        let metrics = connection.metrics.iter().filter(|metric| {
            if has_daily_totals {
                metric.metric_key.0 == MetricKey::TOKEN_TOTAL
            } else {
                metric.metric_key.0 == MetricKey::TOKEN_INPUT
                    || metric.metric_key.0 == MetricKey::TOKEN_OUTPUT
            }
        });
        for metric in metrics {
            let Some(day) = metric.period_start.map(|time| time.date_naive()) else {
                continue;
            };
            if (start..=today).contains(&day) {
                *days.entry(day).or_default() += metric.value;
            }
        }
    }

    if days.is_empty() {
        return None;
    }
    let values = (0..7)
        .map(|offset| {
            days.get(&(start + Duration::days(offset)))
                .copied()
                .unwrap_or_default()
        })
        .collect::<Vec<_>>();
    sparkline(values)
}

fn sparkline(values: Vec<rust_decimal::Decimal>) -> Option<String> {
    use rust_decimal::Decimal;

    let max = values.iter().copied().max().unwrap_or_default();
    if max <= Decimal::ZERO {
        return None;
    }
    const BARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    Some(
        values
            .into_iter()
            .map(|value| {
                let index = ((value * Decimal::from(7)) / max)
                    .round()
                    .to_string()
                    .parse::<usize>()
                    .unwrap_or_default()
                    .min(7);
                BARS[index]
            })
            .collect(),
    )
}
fn short(v: &str) -> &str {
    if v.len() > 24 { "LLM" } else { v }
}
fn status_label(s: ConnectionStatus) -> &'static str {
    match s {
        ConnectionStatus::Connecting => "connecting",
        ConnectionStatus::Ready => "ready",
        ConnectionStatus::Syncing => "syncing",
        ConnectionStatus::Stale => "stale",
        ConnectionStatus::AuthRequired => "authentication required",
        ConnectionStatus::RateLimited => "rate limited",
        ConnectionStatus::Offline => "offline",
        ConnectionStatus::ProviderError => "provider error",
        ConnectionStatus::Disabled => "disabled",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local_codex::LocalDailyUsage;
    #[test]
    fn unknown_quota_omits_percentage() {
        let s = Snapshot {
            generated_at: Utc::now(),
            connections: vec![],
            local_codex: LocalCodexSnapshot::default(),
        };
        assert_eq!(render_waybar(&s).percentage, None);
    }

    #[test]
    fn sparkline_uses_refreshed_local_daily_history() {
        let today = Utc::now().date_naive();
        let local_codex = LocalCodexSnapshot {
            daily_usage: (0..7)
                .map(|offset| LocalDailyUsage {
                    date: today - Duration::days(6 - offset),
                    models: Vec::new(),
                    total_tokens: (offset + 1) * 100,
                    estimated_cost_usd: None,
                })
                .collect(),
            ..Default::default()
        };
        let snapshot = Snapshot {
            generated_at: Utc::now(),
            connections: Vec::new(),
            local_codex,
        };

        let trend = token_sparkline(&snapshot).expect("local trend");
        assert_eq!(trend.chars().count(), 7);
        assert!(trend.ends_with('█'));
    }
}
