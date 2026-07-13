use llm_meter_core::{AlertKind, AlertRule, AlertState};

pub fn deliver(connection_name: &str, alerts: &[AlertRule]) {
    for alert in alerts.iter().filter(|v| v.state == AlertState::Triggered) {
        let title = match alert.kind {
            AlertKind::QuotaRemaining => "LLM quota is running low",
            AlertKind::QuotaResetSoon => "LLM quota resets soon",
            AlertKind::TokenDaily => "Daily token threshold reached",
            AlertKind::CostBudgetRatio => "Local budget threshold reached",
            AlertKind::CacheHitRatio => "Cache efficiency is below target",
            AlertKind::AuthenticationRequired => "Connection needs authentication",
            AlertKind::DataStale => "LLM Meter data is stale",
        };
        let body = format!("{connection_name} · {title}");
        if notify_rust::Notification::new()
            .summary("LLM Meter")
            .body(&body)
            .appname("LLM Meter")
            .show()
            .is_err()
        {
            tracing::debug!(
                component = "notifications",
                operation = "deliver",
                result = "unavailable",
                error_code = "notification_service_unavailable"
            );
        }
    }
}
