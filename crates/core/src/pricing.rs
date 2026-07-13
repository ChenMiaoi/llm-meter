//! Static pricing catalog for common LLM models.
//!
//! Values are USD per one million tokens. RMB prices are converted to USD at
//! a fixed rate (see [`CNY_TO_USD`]). Actual invoices can differ because of
//! service tier, regional processing, tools, fine-tuning, or future price
//! changes, so callers must expose results as estimates.

use rust_decimal::Decimal;
use std::str::FromStr;

/// Fixed conversion rate used for RMB prices: 1 CNY ≈ 0.1389 USD.
pub const CNY_TO_USD: Decimal = Decimal::from_parts(1389, 0, 0, false, 4);

/// A single model price entry.
#[derive(Debug, Clone, Copy)]
pub struct ModelPrice {
    /// Provider identifier, e.g. `"openai"` or `"kimi"`.
    pub provider_id: &'static str,
    /// Canonical model identifier used for lookup.
    pub model: &'static str,
    /// Input price (cache miss) per 1M tokens, as a decimal string.
    pub input: &'static str,
    /// Cached-input price per 1M tokens, as a decimal string.
    ///
    /// `None` means no cached-input discount is known for the model.
    pub cached_input: Option<&'static str>,
    /// Output price per 1M tokens, as a decimal string.
    pub output: &'static str,
    /// Public pricing source URL.
    pub source_url: &'static str,
    /// Effective-as-of date (ISO 8601) for the source data.
    pub effective_as_of: &'static str,
}

const fn price(
    provider_id: &'static str,
    model: &'static str,
    input: &'static str,
    cached_input: Option<&'static str>,
    output: &'static str,
    source_url: &'static str,
    effective_as_of: &'static str,
) -> ModelPrice {
    ModelPrice {
        provider_id,
        model,
        input,
        cached_input,
        output,
        source_url,
        effective_as_of,
    }
}

// -------------------------------------------------------------------------
// OpenAI
// -------------------------------------------------------------------------

pub const OPENAI_SOURCE_URL: &str = "https://developers.openai.com/api/docs/pricing";
pub const OPENAI_EFFECTIVE_AS_OF: &str = "2026-07-13";

const fn openai_price(
    model: &'static str,
    input: &'static str,
    cached_input: Option<&'static str>,
    output: &'static str,
) -> ModelPrice {
    price(
        "openai",
        model,
        input,
        cached_input,
        output,
        OPENAI_SOURCE_URL,
        OPENAI_EFFECTIVE_AS_OF,
    )
}

/// OpenAI standard-processing text-token prices, USD per 1M tokens.
/// Keep exact model entries before aliases; dated model snapshots fall back to
/// their undated family.
const OPENAI_PRICES: &[ModelPrice] = &[
    openai_price("gpt-5.6-sol", "5", Some("0.5"), "30"),
    openai_price("gpt-5.6-terra", "2.5", Some("0.25"), "15"),
    openai_price("gpt-5.6-luna", "1", Some("0.1"), "6"),
    openai_price("gpt-5.5-pro", "30", None, "180"),
    openai_price("gpt-5.5", "5", Some("0.5"), "30"),
    openai_price("gpt-5.4-pro", "30", None, "180"),
    openai_price("gpt-5.4-mini", "0.75", Some("0.075"), "4.5"),
    openai_price("gpt-5.4-nano", "0.2", Some("0.02"), "1.25"),
    openai_price("gpt-5.4", "2.5", Some("0.25"), "15"),
    openai_price("gpt-5.3-chat-latest", "1.75", Some("0.175"), "14"),
    openai_price("gpt-5.3-codex", "1.75", Some("0.175"), "14"),
    openai_price("gpt-5.2-chat-latest", "1.75", Some("0.175"), "14"),
    openai_price("gpt-5.2-codex", "1.75", Some("0.175"), "14"),
    openai_price("gpt-5.2-pro", "21", None, "168"),
    openai_price("gpt-5.2", "1.75", Some("0.175"), "14"),
    openai_price("gpt-5.1-codex-mini", "0.25", Some("0.025"), "2"),
    openai_price("gpt-5.1-codex-max", "1.25", Some("0.125"), "10"),
    openai_price("gpt-5.1-codex", "1.25", Some("0.125"), "10"),
    openai_price("gpt-5.1-chat-latest", "1.25", Some("0.125"), "10"),
    openai_price("gpt-5.1", "1.25", Some("0.125"), "10"),
    openai_price("gpt-5-codex", "1.25", Some("0.125"), "10"),
    openai_price("gpt-5-mini", "0.25", Some("0.025"), "2"),
    openai_price("gpt-5-nano", "0.05", Some("0.005"), "0.4"),
    openai_price("gpt-5-pro", "15", None, "120"),
    openai_price("gpt-5", "1.25", Some("0.125"), "10"),
    openai_price("codex-mini-latest", "1.5", Some("0.375"), "6"),
    openai_price("gpt-4.1-mini", "0.4", Some("0.1"), "1.6"),
    openai_price("gpt-4.1-nano", "0.1", Some("0.025"), "0.4"),
    openai_price("gpt-4.1", "2", Some("0.5"), "8"),
    openai_price("gpt-4o-mini", "0.15", Some("0.075"), "0.6"),
    openai_price("gpt-4o-2024-05-13", "5", None, "15"),
    openai_price("gpt-4o", "2.5", Some("1.25"), "10"),
    openai_price("o1-pro", "150", None, "600"),
    openai_price("o1-mini", "1.1", Some("0.55"), "4.4"),
    openai_price("o1", "15", Some("7.5"), "60"),
    openai_price("o3-pro", "20", None, "80"),
    openai_price("o3-mini", "1.1", Some("0.55"), "4.4"),
    openai_price("o3", "2", Some("0.5"), "8"),
    openai_price("o4-mini", "1.1", Some("0.275"), "4.4"),
    openai_price("gpt-4-turbo-2024-04-09", "10", None, "30"),
    openai_price("gpt-4-0125-preview", "10", None, "30"),
    openai_price("gpt-4-1106-preview", "10", None, "30"),
    openai_price("gpt-4-1106-vision-preview", "10", None, "30"),
    openai_price("gpt-4-0613", "30", None, "60"),
    openai_price("gpt-4-0314", "30", None, "60"),
    openai_price("gpt-4-32k", "60", None, "120"),
    openai_price("gpt-3.5-turbo-0125", "0.5", None, "1.5"),
    openai_price("gpt-3.5-turbo-1106", "1", None, "2"),
    openai_price("gpt-3.5-turbo-0613", "1.5", None, "2"),
    openai_price("gpt-3.5-turbo-16k-0613", "3", None, "4"),
    openai_price("gpt-3.5-turbo-instruct", "1.5", None, "2"),
    openai_price("gpt-3.5-turbo", "0.5", None, "1.5"),
    openai_price("davinci-002", "2", None, "2"),
    openai_price("babbage-002", "0.4", None, "0.4"),
];

// -------------------------------------------------------------------------
// Kimi (Moonshot AI)
// -------------------------------------------------------------------------

pub const KIMI_SOURCE_URL: &str = "https://platform.kimi.com/docs/pricing/overview";
pub const KIMI_EFFECTIVE_AS_OF: &str = "2026-07-14";

const fn kimi_price(
    model: &'static str,
    input: &'static str,
    cached_input: Option<&'static str>,
    output: &'static str,
) -> ModelPrice {
    price(
        "kimi",
        model,
        input,
        cached_input,
        output,
        KIMI_SOURCE_URL,
        KIMI_EFFECTIVE_AS_OF,
    )
}

/// Kimi text-token prices, USD per 1M tokens.
///
/// Original CNY prices are converted to USD using [`CNY_TO_USD`].
const KIMI_PRICES: &[ModelPrice] = &[
    kimi_price("kimi-k2.7-code", "0.9029", Some("0.1806"), "3.7503"),
    kimi_price(
        "kimi-k2.7-code-highspeed",
        "1.8057",
        Some("0.3611"),
        "7.5006",
    ),
    kimi_price("kimi-k2.6", "0.9029", Some("0.1528"), "3.7503"),
    kimi_price("moonshot-v1-8k", "0.2778", None, "1.3890"),
    kimi_price("moonshot-v1-32k", "0.6945", None, "2.7780"),
    kimi_price("moonshot-v1-128k", "1.3889", None, "4.1670"),
    kimi_price("moonshot-v1-8k-vision-preview", "0.2778", None, "1.3890"),
    kimi_price("moonshot-v1-32k-vision-preview", "0.6945", None, "2.7780"),
    kimi_price("moonshot-v1-128k-vision-preview", "1.3889", None, "4.1670"),
];

// -------------------------------------------------------------------------
// MiniMax
// -------------------------------------------------------------------------

pub const MINIMAX_SOURCE_URL: &str = "https://platform.minimaxi.com/docs/guides/pricing-paygo";
pub const MINIMAX_EFFECTIVE_AS_OF: &str = "2026-07-14";

const fn minimax_price(
    model: &'static str,
    input: &'static str,
    cached_input: Option<&'static str>,
    output: &'static str,
) -> ModelPrice {
    price(
        "minimax",
        model,
        input,
        cached_input,
        output,
        MINIMAX_SOURCE_URL,
        MINIMAX_EFFECTIVE_AS_OF,
    )
}

/// MiniMax pay-as-you-go text-token prices, USD per 1M tokens.
///
/// Original CNY prices are converted to USD using [`CNY_TO_USD`].
const MINIMAX_PRICES: &[ModelPrice] = &[
    minimax_price("minimax-m3", "0.2917", Some("0.0583"), "1.1668"),
    minimax_price("minimax-m2.7", "0.2917", Some("0.0583"), "1.1668"),
    minimax_price("minimax-m2.7-highspeed", "0.5834", Some("0.0583"), "2.3335"),
    minimax_price("minimax-m2.5", "0.2917", Some("0.0292"), "1.1668"),
    minimax_price("minimax-m2.5-highspeed", "0.5834", Some("0.0292"), "2.3335"),
    minimax_price("minimax-m2.1", "0.2917", Some("0.0292"), "1.1668"),
    minimax_price("minimax-m2.1-highspeed", "0.5834", Some("0.0292"), "2.3335"),
    minimax_price("minimax-m2", "0.2917", Some("0.0292"), "1.1668"),
];

// -------------------------------------------------------------------------
// Zhipu GLM
// -------------------------------------------------------------------------

pub const GLM_SOURCE_URL: &str = "https://open.bigmodel.cn/pricing";
pub const GLM_EFFECTIVE_AS_OF: &str = "2026-07-14";

const fn glm_price(
    model: &'static str,
    input: &'static str,
    cached_input: Option<&'static str>,
    output: &'static str,
) -> ModelPrice {
    price(
        "glm",
        model,
        input,
        cached_input,
        output,
        GLM_SOURCE_URL,
        GLM_EFFECTIVE_AS_OF,
    )
}

/// Zhipu GLM text-token prices, USD per 1M tokens.
///
/// Sourced from public comparison providers because the official pricing page
/// requires JavaScript rendering.
const GLM_PRICES: &[ModelPrice] = &[
    glm_price("glm-5", "0.60", None, "2.08"),
    glm_price("glm-4.7", "0.06", None, "0.40"),
];

// -------------------------------------------------------------------------
// Xiaomi MiMo
// -------------------------------------------------------------------------

pub const MIMO_SOURCE_URL: &str = "https://www.xiaomi-mimo.com/pricing";
pub const MIMO_EFFECTIVE_AS_OF: &str = "2026-07-14";

const fn mimo_price(
    model: &'static str,
    input: &'static str,
    cached_input: Option<&'static str>,
    output: &'static str,
) -> ModelPrice {
    price(
        "mimo",
        model,
        input,
        cached_input,
        output,
        MIMO_SOURCE_URL,
        MIMO_EFFECTIVE_AS_OF,
    )
}

/// Xiaomi MiMo text-token prices, USD per 1M tokens.
///
/// Sourced from public comparison providers because official pricing is not
/// widely available in machine-readable form.
const MIMO_PRICES: &[ModelPrice] = &[
    mimo_price("mimo-v2-flash", "0.10", Some("0.010"), "0.30"),
    mimo_price("mimo-v2.5", "0.14", None, "0.28"),
    mimo_price("mimo-v2.5-pro", "0.435", None, "0.87"),
];

// -------------------------------------------------------------------------
// Anthropic Claude
// -------------------------------------------------------------------------

pub const CLAUDE_SOURCE_URL: &str = "https://www.anthropic.com/pricing";
pub const CLAUDE_EFFECTIVE_AS_OF: &str = "2026-07-14";

const fn claude_price(
    model: &'static str,
    input: &'static str,
    cached_input: Option<&'static str>,
    output: &'static str,
) -> ModelPrice {
    price(
        "claude",
        model,
        input,
        cached_input,
        output,
        CLAUDE_SOURCE_URL,
        CLAUDE_EFFECTIVE_AS_OF,
    )
}

/// Anthropic Claude text-token prices, USD per 1M tokens.
///
/// Cache-hit values are the 5-minute context window prices.
const CLAUDE_PRICES: &[ModelPrice] = &[
    claude_price("claude-sonnet-5", "2.00", Some("0.20"), "10.00"),
    claude_price("claude-sonnet-4.6", "3.00", Some("0.30"), "15.00"),
    claude_price("claude-sonnet-4.5", "3.00", Some("0.30"), "15.00"),
    claude_price("claude-opus-4.8", "5.00", Some("0.50"), "25.00"),
    claude_price("claude-opus-4.7", "5.00", Some("0.50"), "25.00"),
    claude_price("claude-opus-4.6", "5.00", Some("0.50"), "25.00"),
    claude_price("claude-opus-4.5", "5.00", Some("0.50"), "25.00"),
    claude_price("claude-haiku-4.5", "1.00", Some("0.10"), "5.00"),
    claude_price("claude-haiku-3.5", "0.80", Some("0.08"), "4.00"),
    claude_price("claude-fable-5", "10.00", Some("1.00"), "50.00"),
];

// -------------------------------------------------------------------------
// DeepSeek
// -------------------------------------------------------------------------

pub const DEEPSEEK_SOURCE_URL: &str = "https://api-docs.deepseek.com/quick_start/pricing";
pub const DEEPSEEK_EFFECTIVE_AS_OF: &str = "2026-07-14";

const fn deepseek_price(
    model: &'static str,
    input: &'static str,
    cached_input: Option<&'static str>,
    output: &'static str,
) -> ModelPrice {
    price(
        "deepseek",
        model,
        input,
        cached_input,
        output,
        DEEPSEEK_SOURCE_URL,
        DEEPSEEK_EFFECTIVE_AS_OF,
    )
}

/// DeepSeek text-token prices, USD per 1M tokens.
const DEEPSEEK_PRICES: &[ModelPrice] = &[
    deepseek_price("deepseek-v4-flash", "0.14", Some("0.0028"), "0.28"),
    deepseek_price("deepseek-v4-pro", "0.435", Some("0.003625"), "0.87"),
];

// -------------------------------------------------------------------------
// Google Gemini
// -------------------------------------------------------------------------

pub const GEMINI_SOURCE_URL: &str = "https://ai.google.dev/gemini-api/docs/pricing";
pub const GEMINI_EFFECTIVE_AS_OF: &str = "2026-07-14";

const fn gemini_price(
    model: &'static str,
    input: &'static str,
    cached_input: Option<&'static str>,
    output: &'static str,
) -> ModelPrice {
    price(
        "gemini",
        model,
        input,
        cached_input,
        output,
        GEMINI_SOURCE_URL,
        GEMINI_EFFECTIVE_AS_OF,
    )
}

/// Google Gemini text-token prices, USD per 1M tokens.
///
/// Prices use the paid-tier Standard tier. Gemini also offers free, Batch, and
/// Flex tiers not reflected here.
const GEMINI_PRICES: &[ModelPrice] = &[
    gemini_price("gemini-3.5-flash", "1.50", Some("0.15"), "9.00"),
    gemini_price("gemini-3.1-flash-lite", "0.25", Some("0.025"), "1.50"),
    gemini_price("gemini-3.1-pro-preview", "2.00", Some("0.20"), "12.00"),
];

// -------------------------------------------------------------------------
// Catalog helpers
// -------------------------------------------------------------------------

/// All known provider identifiers, in a stable order.
pub const PROVIDER_IDS: &[&str] = &[
    "openai", "kimi", "minimax", "glm", "mimo", "claude", "deepseek", "gemini",
];

/// Return the price table for a given provider, or `None` if unknown.
pub fn provider_prices(provider_id: &str) -> Option<&'static [ModelPrice]> {
    match provider_id {
        "openai" => Some(OPENAI_PRICES),
        "kimi" => Some(KIMI_PRICES),
        "minimax" => Some(MINIMAX_PRICES),
        "glm" => Some(GLM_PRICES),
        "mimo" => Some(MIMO_PRICES),
        "claude" => Some(CLAUDE_PRICES),
        "deepseek" => Some(DEEPSEEK_PRICES),
        "gemini" => Some(GEMINI_PRICES),
        _ => None,
    }
}

/// Return the source URL for a provider's pricing data.
pub fn provider_source_url(provider_id: &str) -> Option<&'static str> {
    match provider_id {
        "openai" => Some(OPENAI_SOURCE_URL),
        "kimi" => Some(KIMI_SOURCE_URL),
        "minimax" => Some(MINIMAX_SOURCE_URL),
        "glm" => Some(GLM_SOURCE_URL),
        "mimo" => Some(MIMO_SOURCE_URL),
        "claude" => Some(CLAUDE_SOURCE_URL),
        "deepseek" => Some(DEEPSEEK_SOURCE_URL),
        "gemini" => Some(GEMINI_SOURCE_URL),
        _ => None,
    }
}

/// Return the effective-as-of date for a provider's pricing data.
pub fn provider_effective_as_of(provider_id: &str) -> Option<&'static str> {
    match provider_id {
        "openai" => Some(OPENAI_EFFECTIVE_AS_OF),
        "kimi" => Some(KIMI_EFFECTIVE_AS_OF),
        "minimax" => Some(MINIMAX_EFFECTIVE_AS_OF),
        "glm" => Some(GLM_EFFECTIVE_AS_OF),
        "mimo" => Some(MIMO_EFFECTIVE_AS_OF),
        "claude" => Some(CLAUDE_EFFECTIVE_AS_OF),
        "deepseek" => Some(DEEPSEEK_EFFECTIVE_AS_OF),
        "gemini" => Some(GEMINI_EFFECTIVE_AS_OF),
        _ => None,
    }
}

// -------------------------------------------------------------------------
// Estimation
// -------------------------------------------------------------------------

/// Estimate standard-processing text-token cost for a provider/model.
///
/// Returns `None` if the model is not in the catalog.
pub fn estimate_text_tokens(
    provider_id: &str,
    model: &str,
    input_tokens: i64,
    cached_input_tokens: i64,
    output_tokens: i64,
) -> Option<Decimal> {
    let prices = provider_prices(provider_id)?;
    let price = prices
        .iter()
        .find(|price| price.model == model)
        .or_else(|| {
            let family = strip_date_suffix(model);
            prices.iter().find(|price| price.model == family)
        })?;
    let input_tokens = input_tokens.max(0);
    let cached_tokens = cached_input_tokens.clamp(0, input_tokens);
    let uncached_tokens = input_tokens - cached_tokens;
    let output_tokens = output_tokens.max(0);
    let input_rate = decimal(price.input)?;
    let cached_rate = price.cached_input.and_then(decimal).unwrap_or(input_rate);
    let output_rate = decimal(price.output)?;
    let million = Decimal::from(1_000_000);
    Some(
        (Decimal::from(uncached_tokens) * input_rate
            + Decimal::from(cached_tokens) * cached_rate
            + Decimal::from(output_tokens) * output_rate)
            / million,
    )
}

fn decimal(value: &str) -> Option<Decimal> {
    Decimal::from_str(value).ok()
}

fn strip_date_suffix(model: &str) -> &str {
    if model.len() > 11 {
        let suffix = &model[model.len() - 11..];
        let bytes = suffix.as_bytes();
        if bytes[0] == b'-'
            && bytes[1..5].iter().all(u8::is_ascii_digit)
            && bytes[5] == b'-'
            && bytes[6..8].iter().all(u8::is_ascii_digit)
            && bytes[8] == b'-'
            && bytes[9..11].iter().all(u8::is_ascii_digit)
        {
            return &model[..model.len() - 11];
        }
    }
    model
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openai_estimates_uncached_cached_and_output_tokens() {
        let value = estimate_text_tokens("openai", "gpt-5", 2_000_000, 1_000_000, 500_000).unwrap();
        assert_eq!(value, Decimal::from_str("6.375").unwrap());
    }

    #[test]
    fn openai_dated_models_use_family_price() {
        assert_eq!(
            estimate_text_tokens("openai", "gpt-4o-mini-2024-07-18", 1_000_000, 0, 0),
            Some(Decimal::from_str("0.15").unwrap())
        );
    }

    #[test]
    fn kimi_price_conversion_and_lookup() {
        // 1M uncached input + 1M output for kimi-k2.6.
        let value = estimate_text_tokens("kimi", "kimi-k2.6", 1_000_000, 0, 1_000_000).unwrap();
        assert_eq!(value, Decimal::from_str("4.6532").unwrap());
    }

    #[test]
    fn minimax_price_conversion_and_lookup() {
        let value = estimate_text_tokens("minimax", "minimax-m3", 1_000_000, 0, 1_000_000).unwrap();
        assert_eq!(value, Decimal::from_str("1.4585").unwrap());
    }

    #[test]
    fn glm_lookup() {
        let value = estimate_text_tokens("glm", "glm-5", 1_000_000, 0, 1_000_000).unwrap();
        assert_eq!(value, Decimal::from_str("2.68").unwrap());
    }

    #[test]
    fn mimo_lookup_with_cached_input() {
        let value =
            estimate_text_tokens("mimo", "mimo-v2-flash", 1_000_000, 1_000_000, 1_000_000).unwrap();
        assert_eq!(value, Decimal::from_str("0.31").unwrap());
    }

    #[test]
    fn claude_lookup() {
        let value =
            estimate_text_tokens("claude", "claude-sonnet-5", 1_000_000, 0, 1_000_000).unwrap();
        assert_eq!(value, Decimal::from_str("12.00").unwrap());
    }

    #[test]
    fn deepseek_lookup() {
        let value =
            estimate_text_tokens("deepseek", "deepseek-v4-flash", 1_000_000, 0, 1_000_000).unwrap();
        assert_eq!(value, Decimal::from_str("0.42").unwrap());
    }

    #[test]
    fn gemini_lookup() {
        let value =
            estimate_text_tokens("gemini", "gemini-3.5-flash", 1_000_000, 0, 1_000_000).unwrap();
        assert_eq!(value, Decimal::from_str("10.50").unwrap());
    }

    #[test]
    fn unknown_provider_returns_none() {
        assert_eq!(estimate_text_tokens("unknown", "gpt-5", 100, 0, 100), None);
    }

    #[test]
    fn unknown_model_returns_none() {
        assert_eq!(
            estimate_text_tokens("openai", "future-model", 100, 0, 100),
            None
        );
    }
}
