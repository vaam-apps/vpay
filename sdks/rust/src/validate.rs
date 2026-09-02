//! Request-shape validation run before anything reaches the wire.
//!
//! A deliberate port of `sdks/nodejs/src/validate.ts` — same rule, same
//! bound, same "refuse rather than transmit approximately" stance. Kept in
//! its own module, mirroring the Node SDK's own file layout, so the two can
//! be diffed against each other by name.

use crate::error::Error;

/// The largest integer JavaScript can hold exactly: `Number.MAX_SAFE_INTEGER`,
/// `2^53 - 1`.
///
/// Rust has no such limit — an `i64` amount is exact all the way to
/// `i64::MAX` — so this bound exists **only** for cross-SDK parity: an amount
/// the Rust SDK sent and the Node SDK refused (or, worse, silently rounded)
/// would be a divergence in the money path, which `docs/flows/money.md`
/// forbids. The number is written out rather than computed so that a reader
/// can see it is JavaScript's constant and not an arbitrary ceiling.
pub(crate) const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;

/// Refuses an amount that is negative or beyond [`MAX_SAFE_INTEGER`].
///
/// `field` names the parameter on the wire, so the error points at the same
/// name the server's own `param` would.
///
/// # Errors
/// [`Error::InvalidParams`] — before any request is built, so a rejected
/// amount never spends an assertion `jti` or an idempotency key.
pub(crate) fn check_amount(amount: i64, field: &str) -> Result<(), Error> {
    if !(0..=MAX_SAFE_INTEGER).contains(&amount) {
        return Err(Error::InvalidParams {
            param: field.to_string(),
            message: format!(
                "must be a non-negative integer in minor units, at most {MAX_SAFE_INTEGER} \
                 (JavaScript's Number.MAX_SAFE_INTEGER, for parity with @vpay/sdk), got {amount}"
            ),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_zero_and_the_maximum_safe_integer() {
        assert!(check_amount(0, "amount").is_ok());
        assert!(check_amount(5000, "amount").is_ok());
        assert!(check_amount(MAX_SAFE_INTEGER, "amount").is_ok());
    }

    #[test]
    fn refuses_a_negative_amount() {
        let err = check_amount(-1, "amount").unwrap_err();
        assert!(matches!(err, Error::InvalidParams { ref param, .. } if param == "amount"));
    }

    #[test]
    fn refuses_an_amount_past_the_safe_integer_ceiling() {
        // `2^53` is the first integer JavaScript cannot represent exactly, so
        // it is the first amount the Node SDK would refuse.
        assert!(check_amount(MAX_SAFE_INTEGER + 1, "amount").is_err());
        assert!(check_amount(i64::MAX, "amount").is_err());
    }

    #[test]
    fn the_message_names_the_field_it_was_given() {
        let err = check_amount(-1, "refund amount").unwrap_err();
        assert!(err.to_string().contains("refund amount"), "{err}");
    }
}
