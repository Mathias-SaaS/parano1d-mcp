// SPDX-License-Identifier: Apache-2.0
//! Small JSON value helpers shared by the safety and tool layers.

use serde_json::Value;

/// Parse a JSON number or a decimal integer string into `u64`.
pub fn value_u64(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|text| text.parse::<u64>().ok()))
}

/// A required numeric field.
pub fn field_u64(value: &Value, field: &str) -> Result<u64, String> {
    value
        .get(field)
        .and_then(value_u64)
        .ok_or_else(|| format!("missing or invalid field {field}"))
}

/// An optional numeric field. Present-but-invalid is a hard error, so callers
/// never silently treat a malformed cap or amount as zero.
pub fn optional_field_u64(value: &Value, field: &str) -> Result<Option<u64>, String> {
    match value.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(raw) => value_u64(raw)
            .map(Some)
            .ok_or_else(|| format!("field {field} is present but not a u64")),
    }
}

/// Total per-call spend: the amount plus the fee. The cap must cover both,
/// otherwise a tiny amount with a huge fee would pass an amount-only cap.
pub fn total_spend(amount_micronoid: u64, fee_micronoid: u64) -> Result<u64, String> {
    amount_micronoid
        .checked_add(fee_micronoid)
        .ok_or_else(|| "amount + fee overflows u64".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn accepts_numbers_and_decimal_strings() {
        assert_eq!(value_u64(&json!(42)), Some(42));
        assert_eq!(value_u64(&json!("42")), Some(42));
        assert_eq!(value_u64(&json!(-1)), None);
        assert_eq!(value_u64(&json!("4.2")), None);
        assert_eq!(value_u64(&json!(null)), None);
    }

    #[test]
    fn optional_field_fails_closed_on_malformed_input() {
        assert_eq!(optional_field_u64(&json!({}), "fee").unwrap(), None);
        assert_eq!(
            optional_field_u64(&json!({"fee": null}), "fee").unwrap(),
            None
        );
        assert_eq!(
            optional_field_u64(&json!({"fee": "5000"}), "fee").unwrap(),
            Some(5000)
        );
        assert!(optional_field_u64(&json!({"fee": "abc"}), "fee").is_err());
        assert!(optional_field_u64(&json!({"fee": {}}), "fee").is_err());
    }

    #[test]
    fn total_spend_includes_fee_and_checks_overflow() {
        assert_eq!(total_spend(100, 5).unwrap(), 105);
        assert_eq!(total_spend(0, 5).unwrap(), 5);
        assert!(total_spend(u64::MAX, 1).is_err());
    }
}
