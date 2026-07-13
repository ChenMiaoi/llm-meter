use chrono::Utc;
use llm_meter_core::{
    AlertKind, AlertRule, AlertState, ConnectionStatus, MetricKey, cache_hit_ratio,
};
use llm_meter_storage::{Repository, StorageError};
use rust_decimal::Decimal;
use uuid::Uuid;

pub fn install_defaults(repo: &Repository, connection_id: Uuid) -> Result<(), StorageError> {
    for (kind, threshold) in [
        (AlertKind::QuotaRemaining, Decimal::new(20, 2)),
        (AlertKind::QuotaRemaining, Decimal::new(5, 2)),
        (AlertKind::AuthenticationRequired, Decimal::ZERO),
        (AlertKind::DataStale, Decimal::from(30)),
    ] {
        repo.upsert_alert(&AlertRule {
            id: Uuid::new_v4(),
            connection_id,
            kind,
            threshold,
            state: AlertState::Normal,
            last_triggered_at: None,
            suppressed_until: None,
        })?;
    }
    Ok(())
}

pub fn evaluate(repo: &Repository, connection_id: Uuid) -> Result<Vec<AlertRule>, StorageError> {
    let now = Utc::now();
    let connection = match repo.connection(connection_id)? {
        Some(value) => value,
        None => return Ok(Vec::new()),
    };
    let quotas = repo.quotas(connection_id)?;
    let metrics = repo.metrics(connection_id, None, 5000)?;
    let budget = repo.budget(connection_id)?;
    let mut changed = Vec::new();
    for mut rule in repo.alerts(Some(connection_id))? {
        if rule.suppressed_until.is_some_and(|value| value > now) {
            continue;
        }
        let triggered = match rule.kind {
            AlertKind::QuotaRemaining => quotas
                .iter()
                .filter_map(|quota| quota.remaining_ratio)
                .any(|value| value <= rule.threshold),
            AlertKind::QuotaResetSoon => {
                quotas
                    .iter()
                    .filter_map(|quota| quota.resets_at)
                    .any(|value| {
                        value >= now
                            && value
                                <= now
                                    + chrono::Duration::minutes(
                                        rule.threshold.to_string().parse().unwrap_or(15),
                                    )
                    })
            }
            AlertKind::AuthenticationRequired => {
                connection.status == ConnectionStatus::AuthRequired
            }
            AlertKind::DataStale => connection.last_success_at.is_none_or(|value| {
                now.signed_duration_since(value).num_minutes()
                    >= rule.threshold.to_string().parse().unwrap_or(30)
            }),
            AlertKind::TokenDaily => {
                metrics
                    .iter()
                    .filter(|metric| {
                        metric.metric_key.0 == MetricKey::TOKEN_TOTAL
                            && metric
                                .period_start
                                .is_some_and(|value| value.date_naive() == now.date_naive())
                    })
                    .map(|metric| metric.value)
                    .sum::<Decimal>()
                    >= rule.threshold
            }
            AlertKind::CostBudgetRatio => {
                if let Some(budget) = &budget {
                    let spent = metrics
                        .iter()
                        .filter(|metric| {
                            metric.metric_key.0 == MetricKey::COST_ACTUAL
                                && metric.period_start.is_some_and(|value| {
                                    value.format("%Y-%m").to_string()
                                        == now.format("%Y-%m").to_string()
                                })
                        })
                        .map(|metric| metric.value)
                        .sum::<Decimal>();
                    budget.amount > Decimal::ZERO && spent / budget.amount >= rule.threshold
                } else {
                    false
                }
            }
            AlertKind::CacheHitRatio => metrics
                .iter()
                .filter(|m| m.metric_key.0 == MetricKey::TOKEN_INPUT)
                .any(|input| {
                    metrics
                        .iter()
                        .filter(|m| m.metric_key.0 == MetricKey::TOKEN_CACHED_INPUT)
                        .filter_map(|cached| cache_hit_ratio(input, cached))
                        .any(|ratio| ratio < rule.threshold)
                }),
        };
        let state = if triggered {
            AlertState::Triggered
        } else {
            AlertState::Normal
        };
        if state != rule.state {
            rule.state = state;
            if triggered {
                rule.last_triggered_at = Some(now);
            }
            repo.upsert_alert(&rule)?;
            changed.push(rule);
        }
    }
    Ok(changed)
}
