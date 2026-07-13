//! OpenAI adapter implementation (subscription and Platform Admin).

pub mod platform;
pub mod pricing;
pub mod standard;
pub mod subscription;

use llm_meter_core::*;

pub fn manifest() -> ProviderManifest {
    ProviderManifest {
        provider_id: "openai".into(),
        display_name: "OpenAI".into(),
        adapter_version: env!("CARGO_PKG_VERSION").into(),
        connection_types: vec![
            ConnectionTypeManifest {
                id: "chatgpt_subscription".into(),
                display_name: "ChatGPT Subscription".into(),
                auth_schemes: vec![AuthScheme::OAuthBrowser, AuthScheme::OAuthDeviceCode],
            },
            ConnectionTypeManifest {
                id: "platform_admin".into(),
                display_name: "OpenAI Platform Admin".into(),
                auth_schemes: vec![AuthScheme::AdminApiKey],
            },
            ConnectionTypeManifest {
                id: "platform_standard".into(),
                display_name: "OpenAI Platform API".into(),
                auth_schemes: vec![AuthScheme::ApiKey, AuthScheme::LocalProxy],
            },
        ],
    }
}
