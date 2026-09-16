//! Generic, adapter-driven validation of `payment_method_data[<type>][…]` —
//! the confirm path's answer to "what did the payer type, and is it usable".
//!
//! Before this module existed, `payment_intents::payer_instrument` accepted
//! any non-blank string as a push rail's `msisdn` and handed it to the rail
//! verbatim: no format check, no normalisation, and an unrecognised key
//! silently vanished rather than being refused. This is the fix, and it is
//! deliberately generic — every rule here reads
//! [`vpay_provider::ProviderAdapter::payer_fields`] and walks whatever it
//! declares. **No provider code and no country literal appears in this
//! file**, on ADR-0002's terms applied to a payer field rather than to a
//! rail: the day a second rail declares a field, this module needs no edit.
//!
//! # The declared field set is a closed schema
//!
//! A key the caller sent that no [`PayerField`] names is refused, not
//! dropped. Silently ignoring it is how `payment_method_data[mtn_momo]
//! [msidn]` (a typo) used to reach `submit` with no `msisdn` at all, and how
//! `payment_method_data[orange_money][msisdn]` — sent to a rail that
//! collects nothing — used to look like it had been used when it was never
//! read. Both are now [`ApiError::InvalidParam`], naming the offending key
//! and never its value.
use std::collections::BTreeMap;

use phonenumber::Mode;
use serde_json::{Map, Value};
use vpay_provider::{PayerField, PayerFieldKind, PhonePayerType};

use crate::error::ApiError;

/// A validated, normalised payer field, keyed by [`PayerField::name`] — e.g.
/// `"msisdn"` -> `"237670000000"` (E.164, digits only, the shape
/// `vpay_provider::RefundTarget::msisdn` and
/// `crate::v1::account_holders::canonical_msisdn` already render, so a
/// number is spelled one way everywhere in this workspace).
pub(crate) type ResolvedFields = BTreeMap<&'static str, String>;

/// Walks `declared` against `instrument` (the object at
/// `payment_method_data[<code>]`, if the caller sent one) and returns every
/// field's normalised value, or the first refusal.
///
/// Two passes over `declared`, not one: every key in `instrument` is checked
/// against the closed schema *before* any per-field rule runs, so a caller
/// who both misspells a field and omits the real one is told about the
/// misspelling — the more specific mistake — rather than "msisdn is
/// required", which is true but points at the wrong line of their code.
///
/// # Errors
///
/// [`ApiError::InvalidParam`], naming the offending key, for: a key in
/// `instrument` that `declared` does not name; a `declared` field marked
/// `required` that is missing or blank; a `phone` field whose value does not
/// parse as, or is not a valid number of, the field's own region and
/// `phone_type`. [`ApiError::Internal`] only if an adapter declared a region
/// this crate's `phonenumber` port does not recognise — an adapter bug, not
/// a caller's, since `region` is never caller input (see
/// [`PayerFieldKind::Phone`]).
pub(crate) fn resolve_payer_fields(
    code: &str,
    declared: &[PayerField],
    instrument: Option<&Map<String, Value>>,
) -> Result<ResolvedFields, ApiError> {
    if let Some(instrument) = instrument {
        for key in instrument.keys() {
            if !declared.iter().any(|field| field.name == key) {
                return Err(unknown_field(code, key, declared));
            }
        }
    }

    let mut resolved = ResolvedFields::new();
    for field in declared {
        let raw = instrument
            .and_then(|instrument| instrument.get(field.name))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());

        let Some(raw) = raw else {
            if field.required {
                return Err(missing_field(code, field));
            }
            continue;
        };

        let normalized = match field.kind {
            PayerFieldKind::Phone { region, phone_type } => {
                validate_phone(code, field, region, phone_type, raw)?
            }
        };
        resolved.insert(field.name, normalized);
    }
    Ok(resolved)
}

/// [`ApiError::InvalidParam`] for a key `instrument` carries that `declared`
/// does not name. Lists the accepted names when there are any — a caller
/// who typo'd `msidn` is helped by seeing `msisdn` in the message — and says
/// plainly that the rail takes none when `declared` is empty, which is the
/// [`crate::model::RailSpec`] contract for a redirect rail spelled out in
/// prose.
fn unknown_field(code: &str, key: &str, declared: &[PayerField]) -> ApiError {
    let message = if declared.is_empty() {
        format!("`{code}` does not accept a `payment_method_data[{code}]` field at all.")
    } else {
        let accepted = declared
            .iter()
            .map(|field| field.name)
            .collect::<Vec<_>>()
            .join(", ");
        format!("`{code}` does not accept `{key}`; it accepts: {accepted}.")
    };
    ApiError::invalid_param(format!("payment_method_data[{code}][{key}]"), message)
}

/// [`ApiError::InvalidParam`] for a `required` field with no usable value.
fn missing_field(code: &str, field: &PayerField) -> ApiError {
    ApiError::invalid_param(
        format!("payment_method_data[{code}][{}]", field.name),
        format!(
            "`{code}` needs this field, sent as `payment_method_data[{code}][{}]`.",
            field.name
        ),
    )
}

/// Parses `raw` as a phone number of `region` (ISO 3166-1 alpha-2) and
/// `phone_type`, and normalises it to the digits-only E.164 form every
/// adapter in this workspace expects a `msisdn` in (`RefundTarget::msisdn`'s
/// doc has the worked examples).
///
/// # Two refusals with one message each, both silent about the value
///
/// A `phonenumber::ParseError` (not viable as any phone number) and a parsed
/// number that is not [`phonenumber::PhoneNumber::is_valid`], not resolved
/// to `region` itself, or not of `phone_type` are collapsed into the same
/// public message — [`Self`]-free, on `vpay_provider::InvalidMsisdn`'s
/// precedent (`RefundTarget::mobile_money`'s doc): telling a caller *which*
/// rule a number broke would mean echoing enough of it back to explain, and
/// the rule this module exists to enforce is "never log or echo the value".
///
/// The region check (`n.country().id() == Some(region)`) is what stops a
/// syntactically valid E.164 number for a *different* country from passing
/// a field declared for this one — `phonenumber::parse` happily resolves
/// `+14155552671` under a `Some(CM)` hint once the string names its own
/// country code, and without this check it would validate as a (wrong)
/// number for a rail that only ever meant to prompt a Cameroonian handset.
fn validate_phone(
    code: &str,
    field: &PayerField,
    region: &'static str,
    phone_type: PhonePayerType,
    raw: &str,
) -> Result<String, ApiError> {
    let country = region.parse::<phonenumber::country::Id>().map_err(|_| {
        ApiError::Internal(format!(
            "provider adapter for `{code}` declared payer field `{}` with region `{region}`, \
             which vpay's phonenumber port does not recognise as an ISO 3166-1 alpha-2 code",
            field.name
        ))
    })?;

    let invalid = || invalid_phone(code, field);

    let number = phonenumber::parse(Some(country), raw).map_err(|_| invalid())?;
    if !number.is_valid() || number.country().id() != Some(country) {
        return Err(invalid());
    }
    if number.number_type(&phonenumber::metadata::DATABASE) != expected_type(phone_type) {
        return Err(invalid());
    }

    // E.164 (`+237670000000`) minus the leading `+`: the shape
    // `vpay_provider::RefundTarget::msisdn` and
    // `crate::v1::account_holders::canonical_msisdn` already render, and the
    // shape every adapter's wire type expects a `partyId`/`msisdn` in.
    let e164 = number.format().mode(Mode::E164).to_string();
    Ok(e164.trim_start_matches('+').to_owned())
}

/// The one [`phonenumber::Type`] that satisfies each [`PhonePayerType`].
///
/// An exact match rather than also accepting
/// [`phonenumber::Type::FixedLineOrMobile`] (the answer libphonenumber gives
/// for a region, like the US, whose numbering plan cannot tell the two
/// apart from the digits alone): every region an adapter declares today
/// (`"CM"`) resolves an MTN mobile number to a clean
/// [`phonenumber::Type::Mobile`] — proven by this module's own tests — so
/// accepting the ambiguous answer too would be widening the rule to cover a
/// case no declared region has ever produced, on `docs/status.md`'s rule
/// against building ahead of a real caller.
const fn expected_type(phone_type: PhonePayerType) -> phonenumber::Type {
    match phone_type {
        PhonePayerType::Mobile => phonenumber::Type::Mobile,
    }
}

/// [`ApiError::InvalidParam`] for a phone field that failed to parse or
/// validate — see [`validate_phone`]'s own doc for why every failure mode
/// shares this one message.
fn invalid_phone(code: &str, field: &PayerField) -> ApiError {
    ApiError::invalid_param(
        format!("payment_method_data[{code}][{}]", field.name),
        format!(
            "`payment_method_data[{code}][{}]` must be a valid phone number.",
            field.name
        ),
    )
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const MSISDN_FIELD: PayerField = PayerField {
        name: "msisdn",
        kind: PayerFieldKind::Phone {
            region: "CM",
            phone_type: PhonePayerType::Mobile,
        },
        required: true,
        label_key: "msisdn.label",
    };

    fn instrument(value: &str) -> Map<String, Value> {
        let mut map = Map::new();
        map.insert("msisdn".to_owned(), json!(value));
        map
    }

    fn param_of(error: &ApiError) -> Option<String> {
        match error {
            ApiError::InvalidParam { param, .. } => Some(param.clone()),
            _ => None,
        }
    }

    /// The three spellings the checkout page and `RefundTarget::mobile_money`
    /// both accept all normalise to the same digits-only E.164 vpay hands a
    /// rail — and so does a bare national number, on `region`'s strength.
    #[test]
    fn a_valid_cm_mobile_number_normalises_to_digits_only_e164() {
        for spelling in [
            "237670000000",
            "+237670000000",
            "670000000",
            "+237 670 00 00 00",
        ] {
            let resolved =
                resolve_payer_fields("mtn_momo", &[MSISDN_FIELD], Some(&instrument(spelling)))
                    .unwrap_or_else(|error| panic!("{spelling:?} must validate: {error:?}"));
            assert_eq!(
                resolved.get("msisdn").map(String::as_str),
                Some("237670000000"),
                "{spelling:?}"
            );
        }
    }

    /// A missing or blank required field is refused, naming the field's own
    /// `payment_method_data` path.
    #[test]
    fn a_missing_required_field_is_refused_by_name() {
        for instrument in [None, Some(Map::new())] {
            let error = resolve_payer_fields("mtn_momo", &[MSISDN_FIELD], instrument.as_ref())
                .expect_err("a required field with no value must be refused");
            assert_eq!(
                param_of(&error).as_deref(),
                Some("payment_method_data[mtn_momo][msisdn]")
            );
        }
    }

    /// Neither garbage nor a well-formed number from the wrong country nor a
    /// non-mobile number validates — and the refusal never contains the
    /// value that was refused, matching `RefundTarget::mobile_money`'s own
    /// promise for the same reason (payer data must never reach a log via an
    /// error message).
    #[test]
    fn malformed_wrong_region_and_non_mobile_numbers_are_all_refused_without_an_echo() {
        for bad in [
            "not a phone number",
            "12345",
            // A real, valid, syntactically-complete US number — refused
            // because it does not resolve to CM, not because it is not a
            // phone number.
            "+14155552671",
        ] {
            let error = resolve_payer_fields("mtn_momo", &[MSISDN_FIELD], Some(&instrument(bad)))
                .expect_err(&format!("{bad:?} must be refused"));
            assert_eq!(
                param_of(&error).as_deref(),
                Some("payment_method_data[mtn_momo][msisdn]")
            );
            let ApiError::InvalidParam { message, .. } = &error else {
                panic!("expected InvalidParam")
            };
            assert!(
                !message.contains(bad),
                "the refusal for {bad:?} must not echo the value: {message}"
            );
        }
    }

    /// A field the adapter did not declare is refused, not dropped — the
    /// closed-schema rule this module exists to add. Both a misspelling on a
    /// rail that *does* collect a field, and any field at all on a rail that
    /// collects none, are the same defect.
    #[test]
    fn an_undeclared_field_is_refused_not_silently_ignored() {
        let mut typo = Map::new();
        typo.insert("msidn".to_owned(), json!("237670000000"));
        let error = resolve_payer_fields("mtn_momo", &[MSISDN_FIELD], Some(&typo))
            .expect_err("a misspelled field name must be refused");
        assert_eq!(
            param_of(&error).as_deref(),
            Some("payment_method_data[mtn_momo][msidn]"),
            "the refusal must name the field the caller actually sent"
        );

        let mut extra = Map::new();
        extra.insert("msisdn".to_owned(), json!("237670000000"));
        let error = resolve_payer_fields("orange_money", &[], Some(&extra))
            .expect_err("a redirect rail that declares no fields must refuse any it is sent");
        assert_eq!(
            param_of(&error).as_deref(),
            Some("payment_method_data[orange_money][msisdn]")
        );
    }

    /// A redirect rail with an empty instrument (the common case: the
    /// caller sends no `payment_method_data[orange_money]` object at all)
    /// resolves to nothing and is not an error.
    #[test]
    fn a_redirect_rail_with_no_declared_fields_and_no_instrument_resolves_to_nothing() {
        let resolved = resolve_payer_fields("orange_money", &[], None)
            .expect("no declared fields and no instrument is the ordinary case");
        assert!(resolved.is_empty());
    }
}
