//! Kimi (Moonshot AI) adapter implementation for LLM Meter.

pub mod subscription;

use llm_meter_core::*;

pub fn manifest() -> ProviderManifest {
    ProviderManifest {
        provider_id: "kimi".into(),
        display_name: "Kimi".into(),
        adapter_version: env!("CARGO_PKG_VERSION").into(),
        connection_types: vec![ConnectionTypeManifest {
            id: "kimi_code_subscription".into(),
            display_name: "Kimi Code".into(),
            auth_schemes: vec![AuthScheme::OAuthDeviceCode],
        }],
    }
}
