//! Standard-processing text token prices published by OpenAI.
//!
//! Values are USD per one million tokens. Actual invoices can differ because
//! of service tier, regional processing, tools, fine-tuning, or future price
//! changes, so callers must expose results as estimates.

use rust_decimal::Decimal;
use std::str::FromStr;

pub const PRICE_SOURCE_URL: &str = "https://developers.openai.com/api/docs/pricing";
pub const PRICE_AS_OF: &str = "2026-07-13";

#[derive(Debug, Clone, Copy)]
struct ModelPrice {
    model: &'static str,
    input: &'static str,
    cached_input: Option<&'static str>,
    output: &'static str,
}

// Standard processing, text tokens, USD / 1M tokens. Keep exact model entries
// before aliases; dated model snapshots fall back to their undated family.
const PRICES: &[ModelPrice] = &[
    price("gpt-5.6-sol", "5", Some("0.5"), "30"),
    price("gpt-5.6-terra", "2.5", Some("0.25"), "15"),
    price("gpt-5.6-luna", "1", Some("0.1"), "6"),
    price("gpt-5.5-pro", "30", None, "180"),
    price("gpt-5.5", "5", Some("0.5"), "30"),
    price("gpt-5.4-pro", "30", None, "180"),
    price("gpt-5.4-mini", "0.75", Some("0.075"), "4.5"),
    price("gpt-5.4-nano", "0.2", Some("0.02"), "1.25"),
    price("gpt-5.4", "2.5", Some("0.25"), "15"),
    price("gpt-5.3-chat-latest", "1.75", Some("0.175"), "14"),
    price("gpt-5.3-codex", "1.75", Some("0.175"), "14"),
    price("gpt-5.2-chat-latest", "1.75", Some("0.175"), "14"),
    price("gpt-5.2-codex", "1.75", Some("0.175"), "14"),
    price("gpt-5.2-pro", "21", None, "168"),
    price("gpt-5.2", "1.75", Some("0.175"), "14"),
    price("gpt-5.1-codex-mini", "0.25", Some("0.025"), "2"),
    price("gpt-5.1-codex-max", "1.25", Some("0.125"), "10"),
    price("gpt-5.1-codex", "1.25", Some("0.125"), "10"),
    price("gpt-5.1-chat-latest", "1.25", Some("0.125"), "10"),
    price("gpt-5.1", "1.25", Some("0.125"), "10"),
    price("gpt-5-codex", "1.25", Some("0.125"), "10"),
    price("gpt-5-mini", "0.25", Some("0.025"), "2"),
    price("gpt-5-nano", "0.05", Some("0.005"), "0.4"),
    price("gpt-5-pro", "15", None, "120"),
    price("gpt-5", "1.25", Some("0.125"), "10"),
    price("codex-mini-latest", "1.5", Some("0.375"), "6"),
    price("gpt-4.1-mini", "0.4", Some("0.1"), "1.6"),
    price("gpt-4.1-nano", "0.1", Some("0.025"), "0.4"),
    price("gpt-4.1", "2", Some("0.5"), "8"),
    price("gpt-4o-mini", "0.15", Some("0.075"), "0.6"),
    price("gpt-4o-2024-05-13", "5", None, "15"),
    price("gpt-4o", "2.5", Some("1.25"), "10"),
    price("o1-pro", "150", None, "600"),
    price("o1-mini", "1.1", Some("0.55"), "4.4"),
    price("o1", "15", Some("7.5"), "60"),
    price("o3-pro", "20", None, "80"),
    price("o3-mini", "1.1", Some("0.55"), "4.4"),
    price("o3", "2", Some("0.5"), "8"),
    price("o4-mini", "1.1", Some("0.275"), "4.4"),
    price("gpt-4-turbo-2024-04-09", "10", None, "30"),
    price("gpt-4-0125-preview", "10", None, "30"),
    price("gpt-4-1106-preview", "10", None, "30"),
    price("gpt-4-1106-vision-preview", "10", None, "30"),
    price("gpt-4-0613", "30", None, "60"),
    price("gpt-4-0314", "30", None, "60"),
    price("gpt-4-32k", "60", None, "120"),
    price("gpt-3.5-turbo-0125", "0.5", None, "1.5"),
    price("gpt-3.5-turbo-1106", "1", None, "2"),
    price("gpt-3.5-turbo-0613", "1.5", None, "2"),
    price("gpt-3.5-turbo-16k-0613", "3", None, "4"),
    price("gpt-3.5-turbo-instruct", "1.5", None, "2"),
    price("gpt-3.5-turbo", "0.5", None, "1.5"),
    price("davinci-002", "2", None, "2"),
    price("babbage-002", "0.4", None, "0.4"),
];

const fn price(
    model: &'static str,
    input: &'static str,
    cached_input: Option<&'static str>,
    output: &'static str,
) -> ModelPrice {
    ModelPrice {
        model,
        input,
        cached_input,
        output,
    }
}

/// Estimate standard-processing text-token cost in USD.
pub fn estimate_text_tokens(
    model: &str,
    input_tokens: i64,
    cached_input_tokens: i64,
    output_tokens: i64,
) -> Option<Decimal> {
    let price = PRICES
        .iter()
        .find(|price| price.model == model)
        .or_else(|| {
            let family = strip_date_suffix(model);
            PRICES.iter().find(|price| price.model == family)
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
    fn estimates_uncached_cached_and_output_tokens() {
        let value = estimate_text_tokens("gpt-5", 2_000_000, 1_000_000, 500_000).unwrap();
        assert_eq!(value, Decimal::from_str("6.375").unwrap());
    }

    #[test]
    fn dated_models_use_family_price() {
        assert_eq!(
            estimate_text_tokens("gpt-4o-mini-2024-07-18", 1_000_000, 0, 0),
            Some(Decimal::from_str("0.15").unwrap())
        );
    }

    #[test]
    fn unknown_models_are_not_guessed() {
        assert_eq!(estimate_text_tokens("future-model", 100, 0, 100), None);
    }
}
