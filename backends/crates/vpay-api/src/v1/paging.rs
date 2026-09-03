//! Stripe-shaped cursor paging: `limit`, `starting_after`, `ending_before`,
//! and the rules that decide what each one is allowed to be.
//!
//! # Why this is a module and not two copies
//!
//! `/v1/payment_intents` and `/v1/events` page identically — the same three
//! parameters, the same ceiling, the same "these two are opposite directions
//! and you may not send both" refusal — and they page over two repositories
//! whose SQL is deliberately the same shape (`vpay_db::payment_intents::list_page`,
//! `vpay_db::events::list_page`). Two hand-written copies of these rules is
//! how one endpoint's `limit` quietly becomes a validation rule while the
//! other's stays a ceiling, and how one of them starts accepting a cursor
//! from the *other* resource.
//!
//! The one thing that is **not** shared is which id prefix a cursor must
//! carry: that is per-resource and is passed in, because accepting a `pi_…`
//! where an `evt_…` was meant is precisely the mistake the shape check
//! exists to catch (see [`validated_cursor`]).

use vpay_db::ListPage;

use crate::error::ApiError;

/// The two cursor parameters, named once so the `param` a caller is told to
/// fix cannot drift from the field they sent.
pub(crate) const STARTING_AFTER: &str = "starting_after";
pub(crate) const ENDING_BEFORE: &str = "ending_before";

/// Page size when a caller names none, and the ceiling it is capped to.
pub(crate) const DEFAULT_LIMIT: i64 = 10;
pub(crate) const MAX_LIMIT: i64 = 100;

/// One resource's paging vocabulary: the id prefix its cursors carry, and
/// the noun a refusal names.
///
/// Carried as a pair rather than derived from the prefix because the
/// sentence a merchant reads ("A cursor must be an event id…") is copy, not
/// a mechanical transform of `evt_`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CursorKind {
    /// `vpay_core::ids`' prefix for this resource, e.g. `evt_`.
    pub(crate) prefix: &'static str,
    /// How the refusal names the object, e.g. `an event id`.
    pub(crate) noun: &'static str,
}

/// Builds the repository's [`ListPage`] from the three raw query values,
/// applying every rule in this module.
///
/// The "not both cursors" check lives here rather than in each handler for
/// the reason the module exists: it is one refusal with one sentence, and a
/// handler that forgot it would silently apply both predicates and return a
/// window neither cursor asked for.
///
/// # Errors
///
/// [`ApiError::InvalidParam`] naming `limit`, `starting_after` or
/// `ending_before` — never a bare "bad request", because Stripe's `param`
/// field is the whole reason a client can tell which of the three it got
/// wrong.
pub(crate) fn list_page(
    limit: Option<&str>,
    starting_after: Option<String>,
    ending_before: Option<String>,
    kind: CursorKind,
) -> Result<ListPage, ApiError> {
    let page = ListPage {
        limit: parse_limit(limit)?,
        starting_after: validated_cursor(STARTING_AFTER, starting_after, kind)?,
        ending_before: validated_cursor(ENDING_BEFORE, ending_before, kind)?,
    };
    if page.starting_after.is_some() && page.ending_before.is_some() {
        return Err(ApiError::invalid_param(
            STARTING_AFTER,
            "Use either `starting_after` or `ending_before`, not both: they name opposite \
             directions through the list.",
        ));
    }
    Ok(page)
}

/// Checks a cursor's *shape* — that it could be one of this merchant's ids
/// under `kind.prefix` — and nothing else.
///
/// The repository resolves a cursor id to a `seq` with a merchant-scoped
/// subquery, so an id that matches nothing (a typo, another merchant's, a
/// deleted one) yields `NULL`, every comparison against `NULL` is false, and
/// the page comes back **empty**. That silence is the right answer for a
/// *foreign* id — telling the caller apart from an empty page would make the
/// list an existence oracle across tenants — and the wrong one for a
/// mistyped id, where the merchant is left staring at an empty list with
/// nothing to fix.
///
/// A shape check separates the two without leaking anything: it depends only
/// on the bytes the caller sent, never on what is in the database. A
/// well-formed id that names no row of theirs still returns the empty page,
/// deliberately.
///
/// An empty value is treated as absent rather than as a malformed cursor:
/// `?starting_after=` is what a client templating an optional field emits
/// when it has none, and refusing it would break paging for a caller that is
/// not paging.
///
/// # Errors
///
/// [`ApiError::InvalidParam`] naming `param`.
pub(crate) fn validated_cursor(
    param: &'static str,
    raw: Option<String>,
    kind: CursorKind,
) -> Result<Option<String>, ApiError> {
    let Some(cursor) = raw.map(|cursor| cursor.trim().to_owned()) else {
        return Ok(None);
    };
    if cursor.is_empty() {
        return Ok(None);
    }
    if !vpay_core::ids::is_well_formed(kind.prefix, &cursor) {
        return Err(ApiError::invalid_param(
            param,
            format!(
                "A cursor must be {}, as returned in a previous page's `data`.",
                kind.noun
            ),
        ));
    }
    Ok(Some(cursor))
}

/// The page size: absent means [`DEFAULT_LIMIT`], and anything above
/// [`MAX_LIMIT`] is capped to it rather than refused — a ceiling, not a
/// validation rule. A caller who asks for more gets a full page and
/// `has_more: true`, which is a correct answer to their question; refusing
/// would only make them ask again.
///
/// # Errors
///
/// [`ApiError::InvalidParam`] naming `limit` for a value that is not a
/// whole number, or is below 1.
pub(crate) fn parse_limit(raw: Option<&str>) -> Result<i64, ApiError> {
    let Some(raw) = raw else {
        return Ok(DEFAULT_LIMIT);
    };
    let limit: i64 = raw
        .trim()
        .parse()
        .map_err(|_error| ApiError::invalid_param("limit", "`limit` must be a whole number."))?;
    if limit < 1 {
        return Err(ApiError::invalid_param(
            "limit",
            "`limit` must be at least 1.",
        ));
    }
    Ok(limit.min(MAX_LIMIT))
}

#[cfg(test)]
mod tests {
    use vpay_core::ids;

    use super::{
        CursorKind, DEFAULT_LIMIT, ENDING_BEFORE, MAX_LIMIT, STARTING_AFTER, list_page,
        parse_limit, validated_cursor,
    };
    use crate::error::ApiError;

    /// The `param` a refusal names, or `None` if it is not a parameter
    /// refusal at all.
    fn param_of(error: &ApiError) -> Option<&str> {
        match error {
            ApiError::InvalidParam { param, .. } => Some(param),
            _ => None,
        }
    }

    const INTENTS: CursorKind = super::super::payment_intents::CURSOR;
    const EVENTS: CursorKind = super::super::events::CURSOR;

    /// A `limit` above the ceiling is *capped*, not refused — see
    /// [`parse_limit`]'s doc for why that is the right answer to the
    /// caller's question.
    #[test]
    fn the_page_limit_is_a_ceiling_and_not_a_validation_rule() {
        assert_eq!(parse_limit(None).expect("the default"), DEFAULT_LIMIT);
        assert_eq!(parse_limit(Some("7")).expect("a plain limit"), 7);
        assert_eq!(
            parse_limit(Some("1000")).expect("capped, not refused"),
            MAX_LIMIT
        );
        for raw in ["0", "-3", "many"] {
            let error = parse_limit(Some(raw)).expect_err("refused: {raw}");
            assert_eq!(param_of(&error), Some("limit"), "for {raw:?}");
        }
    }

    /// A cursor is checked for *shape* only, and the shapes it refuses are
    /// the ones that would otherwise be answered with an empty page.
    ///
    /// The last assertion is the one that keeps this from becoming an
    /// oracle: a well-formed id this merchant does not own is accepted here
    /// and answered with an empty page by the query, exactly as a foreign
    /// `pi_…` in a `GET /v1/payment_intents/{id}` is answered `404`.
    #[test]
    fn a_cursor_is_checked_for_shape_and_not_for_existence() {
        let real = ids::payment_intent_id();
        assert_eq!(
            validated_cursor(STARTING_AFTER, Some(real.clone()), INTENTS)
                .expect("a real id is accepted"),
            Some(real.clone())
        );
        // Absent, and the empty string a client templating an optional
        // field sends when it has no cursor.
        assert_eq!(
            validated_cursor(STARTING_AFTER, None, INTENTS).expect("absent"),
            None
        );
        for blank in ["", "   "] {
            assert_eq!(
                validated_cursor(ENDING_BEFORE, Some(blank.to_owned()), INTENTS)
                    .expect("blank is absent, not malformed"),
                None
            );
        }
        // Surrounding whitespace survives a copy/paste out of a terminal.
        assert_eq!(
            validated_cursor(STARTING_AFTER, Some(format!("  {real} ")), INTENTS)
                .expect("trimmed, then accepted"),
            Some(real)
        );

        for malformed in [
            "pi_",
            "pi_tooshort",
            "ch_00000000000000000000000x",
            "00000000000000000000000x",
            "PI_00000000000000000000000X",
            "pi_0000000000000000000000 x",
            "1",
        ] {
            let error = validated_cursor(ENDING_BEFORE, Some(malformed.to_owned()), INTENTS)
                .expect_err("a malformed cursor must be named, not answered with an empty page");
            assert_eq!(param_of(&error), Some(ENDING_BEFORE), "for {malformed:?}");
        }

        // Well-formed, and no merchant has ever had it: accepted here on
        // purpose — the query answers an empty page, which is what stops
        // this endpoint from telling one merchant which ids another has.
        assert!(
            validated_cursor(
                STARTING_AFTER,
                Some("pi_00000000000000000000000x".to_owned()),
                INTENTS
            )
            .expect("shape is all that is checked")
            .is_some()
        );
    }

    /// The prefix is per-resource, and each list refuses the *other* list's
    /// cursor.
    ///
    /// This is the case the shared module exists for. Without it, a caller
    /// paging `/v1/events` with a leftover `pi_…` gets a permanently empty
    /// page — the cursor subquery resolves to `NULL` against `events`, every
    /// comparison is false, and nothing anywhere says why.
    #[test]
    fn each_list_refuses_the_other_lists_cursor() {
        let event = ids::event_id();
        let intent = ids::payment_intent_id();

        assert_eq!(
            validated_cursor(STARTING_AFTER, Some(event.clone()), EVENTS)
                .expect("an event id pages the event list"),
            Some(event.clone())
        );
        assert_eq!(
            param_of(
                &validated_cursor(STARTING_AFTER, Some(intent.clone()), EVENTS)
                    .expect_err("a pi_ cursor is not an event cursor")
            ),
            Some(STARTING_AFTER)
        );
        assert_eq!(
            param_of(
                &validated_cursor(STARTING_AFTER, Some(event), INTENTS)
                    .expect_err("an evt_ cursor is not a payment intent cursor")
            ),
            Some(STARTING_AFTER)
        );
        assert_eq!(
            validated_cursor(ENDING_BEFORE, Some(intent.clone()), INTENTS)
                .expect("a pi_ id pages the intent list"),
            Some(intent)
        );
    }

    /// The two cursors name opposite directions, so sending both is a
    /// refusal rather than a window neither of them asked for.
    #[test]
    fn the_two_cursors_cannot_be_combined() {
        let first = ids::event_id();
        let second = ids::event_id();

        let page = list_page(Some("5"), Some(first.clone()), None, EVENTS)
            .expect("one cursor and a limit is a page");
        assert_eq!(page.limit, 5);
        assert_eq!(page.starting_after, Some(first.clone()));
        assert_eq!(page.ending_before, None);

        let error = list_page(None, Some(first), Some(second), EVENTS)
            .expect_err("both cursors at once is refused");
        assert_eq!(param_of(&error), Some(STARTING_AFTER));
    }
}
