//! The audit `sqlx::AssertSqlSafe` demands, made a test instead of a promise.
//!
//! sqlx 0.9 (sqlx#3723) accepts a statement only as a `&'static str` or
//! wrapped in [`sqlx::AssertSqlSafe`], whose contract is that the caller has
//! checked the string for injection. This crate has 37 statements built by
//! `format!`, so it wraps 37 times — and a wrapper whose contract is
//! discharged by a comment is discharged by whoever last read the comment.
//!
//! The invariant, stated once for the whole crate: **every `format!` whose
//! result reaches `AssertSqlSafe` interpolates a `const … : &str` declared in
//! this crate, and nothing else.** Not a merchant id, not a cursor, not a
//! limit, not a status — every one of those is already a bind parameter, and
//! this test is what keeps it that way. The reasoning, and the two named
//! exceptions, are in `docs/reference/vpay-db.md` § dynamic SQL strings and
//! sqlx 0.9.
//!
//! "Interpolates a `const`" means **captured by name**: `{COLUMNS}`, not
//! `{}`. A positional capture takes its value from the argument list, which
//! this module deliberately does not resolve, so it is reported as a violation
//! on sight — see [`POSITIONAL_CAPTURE`], which is where this module's own
//! blind spot was, and what a review mutation walked straight through on
//! 2026-09-05.
//!
//! `#[cfg(test)]`: it reads this crate's own sources off disk through
//! `CARGO_MANIFEST_DIR`, which is a test-time fact, and it must not be
//! compiled into a shipping binary.
//!
//! Why source text and not something typed: the property is about what a
//! *future* `format!` may contain, and there is no type that expresses
//! "interpolates only constants". A reviewer is the alternative, and a
//! reviewer is what sqlx#3723 exists to stop relying on.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

/// The two interpolations that are not a `const` and are allowed anyway, each
/// with the reason it is safe and the exact text that makes it so.
///
/// A closed list on purpose: a third entry is a deliberate edit to this file,
/// which is the review this module exists to force.
const ALLOWED_NON_CONSTANTS: [(&str, &str); 2] = [
    // `let direction = if backwards { "ASC" } else { "DESC" };` — a `bool`
    // chooses between two literals written here. A sort direction cannot be a
    // bind parameter in Postgres, which is why it is interpolated at all.
    // `assert_direction_is_two_literals` checks the definition, so renaming
    // the variable onto a caller's value fails rather than passing.
    (
        "direction",
        "let direction = if backwards { \"ASC\" } else { \"DESC\" };",
    ),
    // `columns = crate::charges::COLUMNS` — `settlement.rs` names another
    // module's constant, so the named-argument form is the only spelling
    // available. Checked as a path below, not merely allowed.
    ("columns", "crate::charges::COLUMNS"),
];

/// `backends/crates/vpay-db/src`.
fn src_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

/// Every `.rs` file under `src/` except this one, sorted.
///
/// Sorted so a failure names files in a stable order, and non-empty is
/// asserted by the callers: a scan that found nothing would pass every
/// assertion below while checking nothing, which is the failure mode
/// `verify-status` and `check-schema` each guard against in their own way.
///
/// This file is excluded because it *quotes* the constructs it looks for —
/// `AssertSqlSafe(` appears here in a needle, in three failure messages and
/// in this sentence, and a scanner that counted its own prose would report
/// four bogus violations and an inflated site count. Excluding one named file
/// is narrow enough to state; a substring exemption would not be.
fn sources() -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut stack = vec![src_dir()];
    while let Some(dir) = stack.pop() {
        let entries = fs::read_dir(&dir).expect("vpay-db/src is readable during its own tests");
        for entry in entries {
            let path = entry.expect("a readable directory entry").path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs")
                && path.file_name().is_some_and(|n| n != "sql_audit.rs")
            {
                let name = path
                    .strip_prefix(src_dir())
                    .expect("every path came from src/")
                    .display()
                    .to_string();
                let text = fs::read_to_string(&path).expect("a readable source file");
                out.push((name, text));
            }
        }
    }
    out.sort();
    out
}

/// Every `const NAME: &str` declared anywhere in this crate.
///
/// The whole crate rather than per file, because `settlement.rs` interpolates
/// `payment_intents::LIVE_CHARGE_STATES` and `charges::COLUMNS`. Visibility is
/// not part of the test: a `pub(crate) const` is as immutable at runtime as a
/// private one, and it is immutability — not reach — that makes it safe here.
fn string_constants(sources: &[(String, String)]) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for (_, text) in sources {
        for line in text.lines() {
            let line = line.trim_start();
            let line = line
                .strip_prefix("pub(crate) ")
                .or_else(|| line.strip_prefix("pub "))
                .unwrap_or(line);
            let Some(rest) = line.strip_prefix("const ") else {
                continue;
            };
            let Some((name, tail)) = rest.split_once(':') else {
                continue;
            };
            if tail.trim_start().starts_with("&str") {
                out.insert(name.trim().to_owned());
            }
        }
    }
    out
}

/// The body of every `format!(…)` in `text` whose result is a SQL statement —
/// i.e. every one bound to a variable named `sql`.
///
/// Balanced-paren scan from the `format!(`, so a `format!` spanning ten lines
/// (most of them do) is one item rather than ten. Nested parentheses inside
/// the string literal — `count(*)`, `now()`, `($1::BIGINT * INTERVAL '1
/// second')` — are what makes the naive "read to the next `)`" wrong, and this
/// crate is full of them.
fn sql_format_bodies(text: &str) -> Vec<String> {
    let bytes: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut search = 0usize;
    while let Some(found) = text[search..].find("format!(") {
        let start_byte = search + found;
        search = start_byte + "format!(".len();

        // Only the ones building a statement. `let sql =` may be on this line
        // or the one above it (rustfmt wraps), so look back a little.
        let before = &text[start_byte.saturating_sub(40)..start_byte];
        if !before.contains("sql") {
            continue;
        }

        let open = text[..start_byte].chars().count() + "format!(".chars().count();
        let mut depth = 1usize;
        let mut i = open;
        while i < bytes.len() && depth > 0 {
            match bytes.get(i) {
                Some('(') => depth += 1,
                Some(')') => depth -= 1,
                _ => {}
            }
            i += 1;
        }
        out.push(
            bytes
                .get(open..i.saturating_sub(1))
                .map(|slice| slice.iter().collect::<String>())
                .unwrap_or_default(),
        );
    }
    out
}

/// What [`interpolations`] calls a capture that has no name of its own —
/// `{}`, `{0}` written as `{}`, or a spec-only `{:>8}`.
///
/// **This is the hole the first version of this module had, found by a review
/// mutation on 2026-09-05 and fixed here.** `interpolations` used to *discard*
/// an empty capture, so `format!("… WHERE payment_intent_id = '{}'",
/// payment_intent_id)` — the natural spelling of the exact injection this
/// module exists to stop, and the one a reviewer is least likely to notice
/// next to thirty-five identical-looking constant interpolations — passed all
/// five tests below. The named spelling `{payment_intent_id}` was caught; the
/// positional one was invisible.
///
/// Reported as a name rather than silently allowed, because a positional
/// capture is *never* checkable here: the value comes from the argument list,
/// which this scanner deliberately does not try to resolve. Every legitimate
/// statement in this crate implicitly captures a `const` by name, so "use
/// `{CONSTANT}`" is always an available answer.
const POSITIONAL_CAPTURE: &str = "<positional `{}`>";

/// Every capture in a format string, ignoring `{{`/`}}` escapes.
///
/// `{{}}` appears for real in `charges.rs` (`COALESCE(provider_ref_extra,
/// '{{}}'::JSONB)`) — an escaped empty JSON object, not an interpolation. A
/// scanner that missed that would report a nonexistent capture called `` and
/// this test would fail on a statement that is fine.
///
/// A capture with no name is reported as [`POSITIONAL_CAPTURE`] rather than
/// dropped; read that constant for why.
fn interpolations(body: &str) -> BTreeSet<String> {
    let chars: Vec<char> = body.chars().collect();
    let mut out = BTreeSet::new();
    let mut i = 0usize;
    while i < chars.len() {
        if chars.get(i) != Some(&'{') {
            i += 1;
            continue;
        }
        if chars.get(i + 1) == Some(&'{') {
            i += 2;
            continue;
        }
        let mut j = i + 1;
        let mut name = String::new();
        while let Some(c) = chars.get(j) {
            if *c == '}' {
                break;
            }
            name.push(*c);
            j += 1;
        }
        // `{:?}`-style formatting specs are not names; splitting on `:` leaves
        // `{value:>8}` as `value` and `{:>8}` as nothing, which the line below
        // then reports as a positional capture.
        let name = name.split(':').next().unwrap_or_default().trim().to_owned();
        out.insert(if name.is_empty() {
            POSITIONAL_CAPTURE.to_owned()
        } else {
            name
        });
        i = j + 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// How many `AssertSqlSafe` call sites this crate is expected to have.
    ///
    /// An exact figure, not a floor: this is the number of places the
    /// compiler's own check has been switched off, and it should not be
    /// possible to add one without saying so here. Measured 2026-09-05 on the
    /// sqlx 0.8 → 0.9 bump; 36 → 37 on 2026-09-05 with
    /// `refunds::Refunds::get_for_merchant`, the merchant read behind
    /// `GET /v1/refunds/{id}` (issue #45); **37 → 39 on 2026-09-06** with
    /// `refunds::Refunds::list_for_intent` and
    /// `events::Events::list_for_objects`, the two reads behind
    /// `GET /dash/v1/payment_intents/{id}` (exp23). Neither adds an
    /// interpolation: both are `SELECT {COLUMNS} …` with every caller value
    /// bound, and `list_page_filtered` — the third new read — reuses the
    /// existing `payment_intents::list_page` site rather than adding one.
    /// **39 → 45 on 2026-09-06** with the six `customers` statements S4a
    /// added (`create`, `get_for_merchant`, `update`, `list_page`,
    /// `idle_since`, `erase_idle`); `touch_last_used` and the hard delete go
    /// through CrateStack and build no string.
    /// **45 → 59 on 2026-09-07** with the fourteen `invoices` statements S4b
    /// added, which is the largest single jump this constant has taken and is
    /// worth accounting for rather than absorbing: `get_for_merchant`,
    /// `list_page`, `update_draft`, `get_item_for_merchant`, `add_item`,
    /// `update_item`, `delete_item`, `attach_intent`, `find_open_by_intent`,
    /// `resum_draft`, `insert_in_tx`, `finalize_in_tx`, `void_in_tx` and
    /// `mark_paid_for_intent_in_tx`. **Three of the module's statements are
    /// NOT here**, and each for a different reason worth knowing:
    /// `mark_uncollectible` and `items_for_invoice` go through CrateStack and
    /// build no string at all; `delete_draft` and `next_number_in_tx` are
    /// plain `&'static str` literals with nothing to interpolate, which sqlx
    /// 0.9 accepts directly.
    ///
    /// Every one of the fourteen interpolates crate constants only —
    /// `COLUMNS`, `QUALIFIED_COLUMNS`, `ITEM_COLUMNS`, `PARENT_IS_A_DRAFT`,
    /// `NO_LIVE_INTENT`, and `list_page`'s `direction`, which is the
    /// already-allowed `if backwards { "ASC" } else { "DESC" }`. No caller
    /// value reaches a `format!` in this crate, which is what the sibling
    /// test proves and what this count keeps reviewable.
    ///
    /// **45 → 41 on 2026-09-07** (S5), and it is the first time this number
    /// has gone DOWN. Four `checkout_sessions` reads —
    /// `get_for_merchant`, `get_by_id_unscoped`, `find_open_by_intent` and
    /// `find_latest_by_intent` — now run through CrateStack and build no
    /// string, so four places the compiler's `&'static str` check was
    /// switched off are gone. A falling count is the one direction this
    /// constant welcomes without argument; a rising one still needs the
    /// sentence above.
    /// **59 → 55 after the S5 rebase over S4b**: S4b's fourteen invoice sites
    /// stay, and S5's four `checkout_sessions` reads no longer build a string.
    ///
    /// **55 → 56 on 2026-09-10** (issues #57 and #66), and the net of +1
    /// hides four additions and three removals, which is the whole reason
    /// this number is reviewed rather than counted. Four terminal writes
    /// moved out of the pool and into the caller's transaction, because each
    /// now commits with the `events` row that reports it:
    /// `payment_intents::cancel` → `cancel_in_tx`, `customers::create` →
    /// `insert_in_tx`, `customers::update` → `update_in_tx`, and the new
    /// `customers::lock_for_update`, which is the `SELECT … FOR UPDATE` that
    /// makes the update's `metadata` merge definite. The three pooled
    /// originals were **deleted** rather than kept beside the transactional
    /// ones, so the count moved by one rather than by four — and "write the
    /// row without the event" stopped being expressible rather than merely
    /// discouraged.
    ///
    /// All four interpolate crate constants only (`COLUMNS`,
    /// `LIVE_CHARGE_STATES`), which is the audit the sibling test performs.
    ///
    /// **56 → 57 on 2026-09-10** (issue #91, D5), and this one really is a
    /// single addition rather than a net: `invoices::add_refund_for_intent_in_tx`,
    /// the statement that moves `amount_refunded` inside the refund
    /// settlement's transaction. It interpolates `COLUMNS` and nothing else;
    /// its three caller-supplied values — the intent id, the amount and the
    /// instant — are `$1`, `$2` and `$3`. The other statement that landed with
    /// it, `refunds::settle_in_tx`, is **not** here and that is the point of
    /// counting: it builds no string at all, so it is an ordinary
    /// `&'static str` and the compiler's own check is never switched off for
    /// it. A new site is worth exactly this much scrutiny — the audit in
    /// `docs/reference/vpay-db.md` § dynamic SQL strings and sqlx 0.9 was
    /// re-read against both statements before this number moved.
    /// **56 -> 60 on 2026-09-10** (issues #67, #68, #96 item 2), a net +4 over
    /// five additions and one removal — the same reason this number is
    /// reviewed rather than counted. `customers::delete_idle`'s
    /// `DELETE … RETURNING` is gone, and the erasure that replaced it builds
    /// five statements:
    ///
    ///   * `erase_in_tx`'s `SELECT NOT ({UNREFERENCED})`, which chooses
    ///     between hard-deleting a customer and anonymising it;
    ///   * `anonymize`'s `UPDATE customers`, which writes the redaction
    ///     marker into all nine identifier columns;
    ///   * two `jsonb_object_agg` rewrites in `redact_stored_copies`, over
    ///     `events.data` and `idempotency_keys.response_body`;
    ///   * `erase_idle`'s `SELECT … FOR UPDATE`, which re-evaluates the
    ///     sweep's guard inside the transaction.
    ///
    /// Each interpolates crate constants and nothing else — `UNREFERENCED`,
    /// `COLUMNS`, `REDACT_CUSTOMER_KEY` — and the values they write are
    /// **bind parameters**: `REDACTED` and a JSON object built from it, never
    /// anything a caller sent. The third redaction, over `charges.payer_ref`,
    /// is a plain `&'static str` and needs no waiver at all.
    /// **60 → 61 on 2026-09-11**, when this branch rebased over the invoice
    /// refund's own new site: four are the erasure's (`customers`, `charges`,
    /// `refunds`, `events`) and the sixty-first is `invoices`'.
    ///
    /// **Still 61 on 2026-09-12** (issue #111), and the number not moving is
    /// the point. The second of the two `jsonb_object_agg` rewrites above —
    /// the one over `idempotency_keys.response_body` — moved out of
    /// `redact_stored_copies` into `redact_stored_responses_in_tx`, so that
    /// `crate::idempotency`'s `store` can run **the same statement** when a
    /// response it is about to make replayable turns out to name a payer an
    /// erasure has just removed. One statement, two callers: a second
    /// spelling of the redaction would have been a second site here, and the
    /// two sides of that race would then be free to disagree about which keys
    /// are a payer's.
    const EXPECTED_ASSERT_SITES: usize = 61;

    /// **The gate.** No `format!` that becomes a statement interpolates
    /// anything but a crate constant.
    ///
    /// Decisive: change any statement below to interpolate a caller's value —
    /// `format!("… WHERE merchant_id = '{merchant_id}'")` — and this fails,
    /// naming the file and the capture. That is the mutation the whole module
    /// exists for, and it is the one `AssertSqlSafe` would otherwise let
    /// through silently.
    #[test]
    fn every_interpolation_into_a_statement_is_a_crate_constant() {
        let sources = sources();
        assert!(
            sources.len() > 10,
            "the source scan found {} file(s); vpay-db has far more, so this \
             test would have been checking nothing",
            sources.len()
        );
        let constants = string_constants(&sources);
        assert!(
            constants.contains("COLUMNS") && constants.contains("LIVE_CHARGE_STATES"),
            "the constant scan missed known constants, so it is not the scan \
             this test thinks it is: {constants:?}"
        );

        let allowed: BTreeSet<&str> = ALLOWED_NON_CONSTANTS.iter().map(|(n, _)| *n).collect();
        let mut checked = 0usize;
        let mut problems = Vec::new();
        for (file, text) in &sources {
            for body in sql_format_bodies(text) {
                checked += 1;
                for name in interpolations(&body) {
                    if constants.contains(&name) || allowed.contains(name.as_str()) {
                        continue;
                    }
                    if name == POSITIONAL_CAPTURE {
                        problems.push(format!(
                            "{file}: a statement uses a positional `{{}}` capture, whose value \
                             comes from the argument list and cannot be checked here. If it is a \
                             caller's value it must be a bind parameter; if it is a fixed \
                             fragment, capture the `const` by name (`{{COLUMNS}}`) so this test \
                             can see what it is — see docs/reference/vpay-db.md § dynamic SQL \
                             strings and sqlx 0.9"
                        ));
                        continue;
                    }
                    problems.push(format!(
                        "{file}: a statement interpolates `{{{name}}}`, which is neither a \
                         `const …: &str` in this crate nor one of the two audited exceptions \
                         ({allowed:?}). If it is a caller's value it must be a bind parameter; \
                         if it is genuinely a fixed fragment, make it a `const` — see \
                         docs/reference/vpay-db.md § dynamic SQL strings and sqlx 0.9"
                    ));
                }
            }
        }

        assert!(problems.is_empty(), "{}", problems.join("\n"));
        assert!(
            checked >= EXPECTED_ASSERT_SITES,
            "only {checked} statement-building `format!`(s) were found, fewer than the \
             {EXPECTED_ASSERT_SITES} `AssertSqlSafe` sites — the scanner stopped matching \
             them, so a green here would mean nothing"
        );
    }

    /// Every `AssertSqlSafe` in this crate wraps the audited variable, and
    /// there are exactly as many as expected.
    ///
    /// The first half is what stops the audit being bypassed by wrapping
    /// something the test above never looked at:
    /// `AssertSqlSafe(format!("… {merchant_id}"))` interpolates into no
    /// variable called `sql` and would have slipped past. The second half is
    /// what stops a site being added without a reviewer noticing.
    #[test]
    fn every_assert_sql_safe_wraps_the_variable_the_audit_covers() {
        let sources = sources();
        let mut sites = 0usize;
        let mut problems = Vec::new();
        for (file, text) in &sources {
            for (offset, _) in text.match_indices("AssertSqlSafe(") {
                let rest = &text[offset + "AssertSqlSafe(".len()..];
                sites += 1;
                if !rest.starts_with("sql)") {
                    let excerpt: String = rest.chars().take(40).collect();
                    problems.push(format!(
                        "{file}: `AssertSqlSafe({excerpt}…` does not wrap the `sql` variable \
                         that `every_interpolation_into_a_statement_is_a_crate_constant` audits"
                    ));
                }
            }
        }
        assert!(problems.is_empty(), "{}", problems.join("\n"));
        assert_eq!(
            sites, EXPECTED_ASSERT_SITES,
            "the number of places sqlx's injection check is asserted away changed; \
             re-do the audit in docs/reference/vpay-db.md and move this number in the \
             same commit"
        );
    }

    /// The `direction` exception really is two literals chosen by a `bool`.
    ///
    /// Without this, the allowlist entry would be the loophole: a later
    /// `let direction = page.order.clone();` would be interpolated straight
    /// into an `ORDER BY` and the gate above would wave it through by name.
    #[test]
    fn the_audited_non_constants_are_still_what_the_audit_says_they_are() {
        let sources = sources();
        for (name, required) in ALLOWED_NON_CONSTANTS {
            let uses: Vec<&(String, String)> = sources
                .iter()
                .filter(|(_, text)| {
                    sql_format_bodies(text)
                        .iter()
                        .any(|body| interpolations(body).contains(name))
                })
                .collect();
            assert!(
                !uses.is_empty(),
                "`{name}` is allowlisted and no statement interpolates it — a stale \
                 exception is an exception nobody re-read"
            );
            for (file, text) in uses {
                assert!(
                    text.contains(required),
                    "{file} interpolates `{{{name}}}` but no longer contains `{required}`, \
                     so the reason it is exempt from the constant rule may have gone with it"
                );
            }
        }
    }

    /// A control on the two scanners themselves.
    ///
    /// Both are string scanners over real Rust, and both have a plausible way
    /// to be wrong that would make the gate vacuous: `sql_format_bodies`
    /// stopping at the first `)` inside `count(*)`, and `interpolations`
    /// reading `{{}}` as a capture. Driven over text rather than the crate, so
    /// the control cannot be satisfied by the sources happening to be clean.
    #[test]
    fn the_scanners_survive_nested_parens_and_escaped_braces() {
        let text = r#"
            let sql = format!(
                "SELECT {COLUMNS}, count(*) FROM t \
                 WHERE x = COALESCE(y, '{{}}'::JSONB) AND z IN ({LIVE_CHARGE_STATES}) \
                 ORDER BY seq {direction} LIMIT $1"
            );
        "#;
        let bodies = sql_format_bodies(text);
        assert_eq!(bodies.len(), 1, "the balanced scan produced {bodies:?}");
        let body = bodies.first().expect("the assertion above found one body");
        assert!(
            body.contains("LIMIT $1"),
            "the scan stopped early, at the first inner `)`: {body}"
        );
        assert_eq!(
            interpolations(body),
            ["COLUMNS", "LIVE_CHARGE_STATES", "direction"]
                .into_iter()
                .map(str::to_owned)
                .collect::<BTreeSet<String>>(),
            "`{{}}` is an escaped empty JSON object, not a capture"
        );
    }

    /// A positional `{}` is seen, and is not a name any allowlist can hold.
    ///
    /// The regression test for this module's own blind spot, found by a review
    /// mutation on 2026-09-05: `interpolations` used to discard an unnamed
    /// capture, so `format!("… = '{}'", payment_intent_id)` — a live injection
    /// — passed every test here, while the named spelling
    /// `{payment_intent_id}` failed. Driven over text, so it stays a statement
    /// about the scanner rather than about whatever the crate happens to
    /// contain.
    ///
    /// Decisive: delete the `POSITIONAL_CAPTURE` branch of `interpolations`
    /// and this fails, and so does the gate above once a positional capture
    /// exists.
    #[test]
    fn a_positional_capture_is_reported_and_is_neither_a_constant_nor_allowed() {
        let text = r#"
            let sql = format!("SELECT {COLUMNS} FROM charges WHERE id = '{}'", caller_value);
        "#;
        let bodies = sql_format_bodies(text);
        let body = bodies.first().expect("one statement-building format!");
        assert_eq!(
            interpolations(body),
            ["COLUMNS", POSITIONAL_CAPTURE]
                .into_iter()
                .map(str::to_owned)
                .collect::<BTreeSet<String>>(),
            "an unnamed capture must be reported, not dropped"
        );

        // And it cannot be waved through the way a name can: it is in neither
        // set the gate consults.
        let constants = string_constants(&sources());
        assert!(!constants.contains(POSITIONAL_CAPTURE));
        assert!(
            !ALLOWED_NON_CONSTANTS
                .iter()
                .any(|(name, _)| *name == POSITIONAL_CAPTURE),
            "the allowlist must never name the positional capture — its value is \
             not visible to this module at all"
        );

        // A spec-only capture is the same case; a named one with a spec is not.
        assert!(interpolations("{:>8}").contains(POSITIONAL_CAPTURE));
        assert!(interpolations("{COLUMNS:>8}").contains("COLUMNS"));

        // `{{}}` is still an escape and still produces nothing — the property
        // `the_scanners_survive_nested_parens_and_escaped_braces` pins, re-
        // asserted here because it is what this change could plausibly break.
        assert!(interpolations("'{{}}'::JSONB").is_empty());
    }

    /// A `format!` that is not building a statement is not audited, and must
    /// not be — `format!("{row:?}")` in a `Debug` test would otherwise have to
    /// be declared a SQL constant.
    #[test]
    fn a_format_that_builds_no_statement_is_not_scanned() {
        assert!(
            sql_format_bodies(r#"let formatted = format!("{row:?}");"#).is_empty(),
            "only statements (the `sql` variable) are in scope for this audit"
        );
    }
}
