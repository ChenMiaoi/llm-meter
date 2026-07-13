//! Standard-processing text token prices published by OpenAI.
//!
//! Values are USD per one million tokens. Actual invoices can differ because
//! of service tier, regional processing, tools, fine-tuning, or future price
//! changes, so callers must expose results as estimates.
//!
//! The canonical catalog lives in [`llm_meter_core::pricing`]; this module
//! is a thin OpenAI-specific wrapper.

use rust_decimal::Decimal;

pub const PRICE_SOURCE_URL: &str = llm_meter_core::pricing::OPENAI_SOURCE_URL;
pub const PRICE_AS_OF: &str = llm_meter_core::pricing::OPENAI_EFFECTIVE_AS_OF;

/// Estimate standard-processing text-token cost in USD for an OpenAI model.
pub fn estimate_text_tokens(
    model: &str,
    input_tokens: i64,
    cached_input_tokens: i64,
    output_tokens: i64,
) -> Option<Decimal> {
    llm_meter_core::pricing::estimate_text_tokens(
        "openai",
        model,
        input_tokens,
        cached_input_tokens,
        output_tokens,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;
    use std::str::FromStr;

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
