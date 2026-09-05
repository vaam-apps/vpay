//! The audit `sqlx::AssertSqlSafe` demands, made a test instead of a promise.
//!
//! sqlx 0.9 (sqlx#3723) accepts a statement only as a `&'static str` or
//! wrapped in [`sqlx::AssertSqlSafe`], whose contract is that the caller has
//! checked the string for injection. This crate has 36 statements built by
//! `format!`, so it wraps 36 times — and a wrapper whose contract is
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

/// Every `{name}` in a format string, ignoring `{{`/`}}` escapes and the
/// positional/empty `{}` form.
///
/// `{{}}` appears for real in `charges.rs` (`COALESCE(provider_ref_extra,
/// '{{}}'::JSONB)`) — an escaped empty JSON object, not an interpolation. A
/// scanner that missed that would report a nonexistent capture called `` and
/// this test would fail on a statement that is fine.
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
        // `{:?}`-style formatting specs are not names; none of the SQL
        // statements uses one, and splitting on `:` keeps that true if one
        // ever appears.
        let name = name.split(':').next().unwrap_or_default().trim().to_owned();
        if !name.is_empty() {
            out.insert(name);
        }
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
    /// sqlx 0.8 → 0.9 bump.
    const EXPECTED_ASSERT_SITES: usize = 36;

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
