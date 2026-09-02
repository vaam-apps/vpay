//! Repository automation. Run via `cargo xtask <cmd>` or `just`.
//!
//! Three of these commands exist to enforce promises this repository makes
//! about itself, because a promise nothing checks is a promise that decays:
//!
//! * `verify-no-mocks`  — no test double is reachable from a shipping binary.
//! * `verify-status`    — every `NotImplemented` is declared in `docs/status.md`.
//! * `verify-errors`    — every error type classifies itself, and `anyhow`
//!   stays at the process edge (`docs/adr/0011-error-modelling.md`).

// This is a CLI; stdout is its output medium, not stray debugging.
#![allow(clippy::print_stdout)]

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn main() -> ExitCode {
    let cmd = std::env::args().nth(1).unwrap_or_else(|| "help".into());
    let root = repo_root();

    let result = match cmd.as_str() {
        "verify-no-mocks" => verify_no_mocks(&root),
        "verify-status" => verify_status(&root),
        "verify-errors" => verify_errors(&root),
        "verify-all" => verify_no_mocks(&root)
            .and_then(|()| verify_status(&root))
            .and_then(|()| verify_errors(&root)),
        "help" | "--help" | "-h" => {
            println!("usage: cargo xtask <verify-no-mocks|verify-status|verify-errors|verify-all>");
            Ok(())
        }
        other => Err(format!("unknown command: {other}")),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("xtask: {msg}");
            ExitCode::FAILURE
        }
    }
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap_or(Path::new("."))
        .to_path_buf()
}

/// Crates that may only ever appear under `[dev-dependencies]`.
const TEST_ONLY: [&str; 6] = [
    "vpay-testkit",
    "wiremock",
    "testcontainers",
    "testcontainers-modules",
    "mockall",
    "fake",
];

/// Manifests that produce a shipping binary.
const SHIPPING: [&str; 2] = [
    "backends/apps/vpay-server/Cargo.toml",
    "backends/apps/vpay-worker-bin/Cargo.toml",
];

/// Fail if a test double is reachable from a shipping binary's runtime deps.
///
/// A stub rail is a WireMock *host in configuration*, never a linked
/// implementation. See `docs/adr/0006-no-mocks-in-main-processes.md`.
fn verify_no_mocks(root: &Path) -> Result<(), String> {
    let mut problems = Vec::new();

    for rel in SHIPPING {
        let path = root.join(rel);
        let text = fs::read_to_string(&path).map_err(|e| format!("{rel}: {e}"))?;
        let runtime = runtime_dependency_section(&text);
        for banned in TEST_ONLY {
            if runtime.lines().any(|l| l.trim_start().starts_with(banned)) {
                problems.push(format!("{rel} has a runtime dependency on `{banned}`"));
            }
        }
    }

    // Nothing outside the testkit itself may reference a stub adapter type.
    for src in rust_sources(&root.join("backends/apps")) {
        let text = fs::read_to_string(&src).unwrap_or_default();
        for needle in ["MockAdapter", "FakeAdapter", "StubAdapter", "DummyAdapter"] {
            if text.contains(needle) {
                problems.push(format!("{} mentions `{needle}`", src.display()));
            }
        }
    }

    if problems.is_empty() {
        println!("verify-no-mocks: ok — no test double reachable from a shipping binary");
        Ok(())
    } else {
        Err(format!(
            "no-mocks violations:\n  - {}",
            problems.join("\n  - ")
        ))
    }
}

/// Everything before the first `[dev-dependencies]` / `[build-dependencies]`.
fn runtime_dependency_section(manifest: &str) -> String {
    let mut out = String::new();
    let mut in_dev = false;
    for line in manifest.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            in_dev = t.contains("dev-dependencies") || t.contains("build-dependencies");
        }
        if !in_dev {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// Fail if the code claims something is unbuilt that `docs/status.md` does not
/// declare — or vice versa. Keeps the status page honest by construction.
fn verify_status(root: &Path) -> Result<(), String> {
    let status_path = root.join("docs/status.md");
    let status = fs::read_to_string(&status_path)
        .map_err(|e| format!("docs/status.md: {e} (the status page is mandatory)"))?;

    let mut found = BTreeSet::new();
    for src in rust_sources(&root.join("backends")) {
        let text = fs::read_to_string(&src).unwrap_or_default();
        // Scan the whole file, not line by line: rustfmt wraps long calls, and
        // a line-based scan silently under-reports. That would make this check
        // pass while an unimplemented path went undeclared — the exact failure
        // it exists to prevent.
        found.extend(scan_not_implemented(&text));
    }

    let undeclared: Vec<_> = found.iter().filter(|t| !status.contains(*t)).collect();
    if !undeclared.is_empty() {
        return Err(format!(
            "these unimplemented items are missing from docs/status.md:\n  - {}",
            undeclared
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join("\n  - ")
        ));
    }

    println!(
        "verify-status: ok — {} unimplemented item(s), all declared in docs/status.md",
        found.len()
    );
    Ok(())
}

/// Extract every `NotImplemented("token")` argument from a source file.
///
/// Whitespace-tolerant, because rustfmt may put the string literal on its own
/// line. Ignores the enum declaration itself, which has no string literal.
fn scan_not_implemented(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (_, after) in text
        .match_indices("NotImplemented(")
        .map(|(i, m)| (i, &text[i + m.len()..]))
    {
        let trimmed = after.trim_start();
        let Some(rest) = trimmed.strip_prefix('"') else {
            continue; // e.g. `NotImplemented(_)` in a match arm, or the enum decl
        };
        if let Some((token, _)) = rest.split_once('"') {
            out.push(token.to_owned());
        }
    }
    out
}

/// Where every library crate lives. Binaries (`backends/apps`) are exempt from
/// both halves of `verify-errors`: they are the *edge*, which is exactly where
/// ADR-0011 puts `anyhow`, and they consume `Classify` rather than implement it.
const LIBRARY_CRATES_DIR: &str = "backends/crates";

/// A type whose name ends in one of these is an error by this workspace's
/// naming convention, and ADR-0011 requires it to classify itself.
/// `Rejection` is here because axum's extractor vocabulary names them that
/// (`FromRequestParts::Rejection`), and `AuthRejection` reaches a merchant
/// through the same envelope every other error does.
const ERROR_SUFFIXES: [&str; 2] = ["Error", "Rejection"];

/// Every spelling of the `Classify` impl header a crate may legitimately use.
///
/// `crate::error::Classify` is `vpay-core` classifying its own leaves; the
/// other three are the re-export (`vpay_core::Classify`), the full path, and a
/// `use`d bare name. Matching text rather than types is the price of having no
/// dependencies here — see the module docs. A wrong-but-plausible spelling
/// fails the check loudly rather than passing silently, which is the safe
/// direction.
const CLASSIFY_IMPL_HEADERS: [&str; 4] = [
    "impl Classify for ",
    "impl vpay_core::Classify for ",
    "impl vpay_core::error::Classify for ",
    "impl crate::error::Classify for ",
];

/// Fail if a `pub` error type in a library crate does not implement
/// `vpay_core::error::Classify`, or if a library crate lists `anyhow` as a
/// runtime dependency.
///
/// Both halves enforce [ADR-0011](../../docs/adr/0011-error-modelling.md): an
/// error that classifies itself gets its HTTP status, retry policy, log
/// severity and process exit code derived once, so two boundaries can never
/// answer the same failure differently. An unclassified error would force the
/// nearest handler to invent an answer, which is the drift the ADR exists to
/// stop. `anyhow` in a library is the same failure in another shape: it erases
/// the type a payment path needs to branch on.
fn verify_errors(root: &Path) -> Result<(), String> {
    let crates_dir = root.join(LIBRARY_CRATES_DIR);
    let mut crate_dirs: Vec<PathBuf> = fs::read_dir(&crates_dir)
        .map_err(|e| format!("{LIBRARY_CRATES_DIR}: {e}"))?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    crate_dirs.sort();

    // A moved or renamed tree would otherwise make this check pass by finding
    // nothing — the failure mode `verify-status` guards against with the same
    // instinct.
    if crate_dirs.is_empty() {
        return Err(format!(
            "{LIBRARY_CRATES_DIR} contains no crate directories"
        ));
    }

    let mut problems = Vec::new();
    let mut classified = 0usize;

    for crate_dir in &crate_dirs {
        let sources: Vec<(PathBuf, String)> = rust_sources(crate_dir)
            .into_iter()
            // Integration tests define no type that crosses a boundary, so an
            // error declared there classifies nothing. Their `impl`s are
            // skipped for the same reason: an impl that only exists under
            // `cargo test` would not satisfy a caller in production.
            .filter(|path| !path.components().any(|c| c.as_os_str() == "tests"))
            .map(|path| {
                let text = fs::read_to_string(&path).unwrap_or_default();
                (path, searchable(&text))
            })
            .collect();

        let mut seen = BTreeSet::new();
        for (path, text) in &sources {
            for name in scan_error_types(text) {
                if !seen.insert(name.clone()) {
                    continue;
                }
                if sources.iter().any(|(_, t)| has_classify_impl(t, &name)) {
                    classified += 1;
                } else {
                    problems.push(format!(
                        "{}: `{name}` has no `impl Classify` anywhere in `{}` (ADR-0011)",
                        relative(root, path),
                        crate_dir.file_name().unwrap_or_default().to_string_lossy(),
                    ));
                }
            }
        }

        let manifest_path = crate_dir.join("Cargo.toml");
        let Ok(manifest) = fs::read_to_string(&manifest_path) else {
            continue; // not a crate directory after all
        };
        if declares_dependency(&runtime_dependency_section(&manifest), "anyhow") {
            problems.push(format!(
                "{}: `anyhow` is a runtime dependency of a library crate; \
                 ADR-0011 confines it to `backends/apps/*` (and to [dev-dependencies])",
                relative(root, &manifest_path),
            ));
        }
    }

    if problems.is_empty() {
        println!(
            "verify-errors: ok — {classified} error type(s), all classified; \
             anyhow confined to binaries"
        );
        Ok(())
    } else {
        Err(format!(
            "error-modelling violations:\n  - {}",
            problems.join("\n  - ")
        ))
    }
}

/// A path as written in the repo, so a failure message can be pasted into an
/// editor.
fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

/// Drop comments and `#[cfg(test)]` items, then collapse whitespace runs to
/// one space.
///
/// Three reasons, all about not lying:
///
/// * a doc comment quoting `impl Classify for FooError` must not satisfy the
///   check — including a `/* */` one, which is why block comments go too;
/// * rustfmt is free to wrap a long `impl` header across lines, which a
///   line-based scan would miss;
/// * an `impl` that only exists under `cargo test` satisfies no caller in
///   production, and a type declared inside a test module reaches no
///   boundary. Both are removed here rather than tolerated, so the check
///   cannot be satisfied *or* tripped by test code.
///
/// Only *leading* `//` is stripped — a trailing comment is left alone so that
/// a string literal containing `//` (a URL) cannot swallow the rest of its
/// line and hide a declaration.
fn searchable(text: &str) -> String {
    let without_blocks = strip_block_comments(text);
    let without_tests = strip_cfg_test_items(&without_blocks);
    let mut out = String::with_capacity(without_tests.len());
    let mut in_whitespace = false;
    for line in without_tests
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
    {
        for ch in line.chars().chain(std::iter::once(' ')) {
            if ch.is_whitespace() {
                if !in_whitespace {
                    out.push(' ');
                    in_whitespace = true;
                }
            } else {
                out.push(ch);
                in_whitespace = false;
            }
        }
    }
    out
}

/// Replace `/* ... */` comments with a space, leaving line breaks intact so
/// the line-based half of [`searchable`] still sees the same lines.
///
/// Nesting is honoured (Rust's block comments nest), and `"/*"` inside a
/// string literal is not a comment — a scan that ignored either would delete
/// live code and make this check pass by finding nothing.
fn strip_block_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    while let Some(c) = chars.next() {
        if depth > 0 {
            if c == '\n' {
                out.push('\n');
            } else if c == '/' && chars.peek() == Some(&'*') {
                chars.next();
                depth += 1;
            } else if c == '*' && chars.peek() == Some(&'/') {
                chars.next();
                depth -= 1;
                out.push(' ');
            }
            continue;
        }
        if in_string {
            out.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match c {
            '"' => {
                in_string = true;
                out.push(c);
            }
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                depth = 1;
            }
            // A line comment is left for `searchable`'s line filter, but its
            // contents must not open a block comment.
            '/' if chars.peek() == Some(&'/') => {
                out.push(c);
                for rest in chars.by_ref() {
                    out.push(rest);
                    if rest == '\n' {
                        break;
                    }
                }
            }
            _ => out.push(c),
        }
    }
    out
}

/// Delete every item annotated `#[cfg(test)]`, body and all.
///
/// The scan that follows must not see test code at all: a `pub enum
/// FooError` declared in a test module reaches no boundary and needs no
/// classification, and — the direction that actually matters — an
/// `impl Classify for FooError` written inside `#[cfg(test)] mod tests`
/// would satisfy this check while satisfying no caller in production.
///
/// Deletes from the attribute to the end of the item: to the matching `}` of
/// the item's first block (`mod tests { .. }`, a function body), or to the
/// terminating `;` if one comes first (`#[cfg(test)] use ...;`). String
/// literals are skipped while counting, because a test containing
/// `from_str("{")` would otherwise unbalance the count and swallow the rest
/// of the file.
fn strip_cfg_test_items(text: &str) -> String {
    let bytes: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0usize;
    while i < bytes.len() {
        if let Some(after_attr) = match_cfg_test(&bytes, i) {
            i = end_of_cfg_test_item(&bytes, after_attr);
            continue;
        }
        if let Some(c) = bytes.get(i) {
            out.push(*c);
        }
        i += 1;
    }
    out
}

/// If `#[cfg(test)]` starts at `i`, the index just past it. Whitespace
/// between the tokens is allowed, because this runs before whitespace is
/// collapsed and nothing guarantees rustfmt's spelling forever.
fn match_cfg_test(chars: &[char], i: usize) -> Option<usize> {
    let mut pos = i;
    for token in ["#", "[", "cfg", "("] {
        while chars.get(pos).is_some_and(|c| c.is_whitespace()) {
            pos += 1;
        }
        for expected in token.chars() {
            if chars.get(pos) != Some(&expected) {
                return None;
            }
            pos += 1;
        }
    }
    // Capture the balanced predicate inside `cfg( ... )`.
    let start = pos;
    let mut depth = 1usize;
    while let Some(&c) = chars.get(pos) {
        pos += 1;
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            _ => {}
        }
    }
    if depth != 0 {
        return None;
    }
    let predicate: String = chars.get(start..pos.saturating_sub(1))?.iter().collect();
    while chars.get(pos).is_some_and(|c| c.is_whitespace()) {
        pos += 1;
    }
    if chars.get(pos) != Some(&']') {
        return None;
    }
    pos += 1;
    if cfg_predicate_is_test_gated(&predicate) {
        Some(pos)
    } else {
        None
    }
}

/// Whether a `cfg(...)` predicate compiles the item *only* under `cargo
/// test` for at least one of its branches: bare `test`, `any(test, ..)`,
/// `all(test, ..)`, at any nesting — but never `not(test)`, which is the
/// *production*-only spelling. An impl gated this way cannot satisfy a
/// production caller, so the scanner treats it as absent. Deliberately
/// textual: the alternative is a `cfg` expression parser, and this check
/// only needs to refuse the spellings a reviewer actually tried
/// (`any(test, feature = "unused")` slipped past the literal-`test` match).
fn cfg_predicate_is_test_gated(predicate: &str) -> bool {
    let compact: String = predicate.chars().filter(|c| !c.is_whitespace()).collect();
    if compact.contains("not(test)") {
        return false;
    }
    compact
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .any(|token| token == "test")
}

/// The index just past the item that `#[cfg(test)]` at `start` annotates.
fn end_of_cfg_test_item(chars: &[char], start: usize) -> usize {
    let mut depth = 0usize;
    let mut i = start;
    let mut in_string = false;
    let mut in_char = false;
    let mut escaped = false;
    while i < chars.len() {
        let Some(&c) = chars.get(i) else { break };
        i += 1;
        if escaped {
            escaped = false;
            continue;
        }
        if in_string || in_char {
            match c {
                '\\' => escaped = true,
                '"' if in_string => in_string = false,
                '\'' if in_char => in_char = false,
                _ => {}
            }
            continue;
        }
        match c {
            '"' => in_string = true,
            // A lifetime (`'a`) is not a char literal; only treat `'` as one
            // when the character after next closes it.
            '\'' if chars.get(i + 1) == Some(&'\'') || chars.get(i) == Some(&'\\') => {
                in_char = true;
            }
            '{' => depth += 1,
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return i;
                }
            }
            ';' if depth == 0 => return i,
            _ => {}
        }
    }
    chars.len()
}

/// Every `pub enum`/`pub struct` that is an error: one that derives
/// `thiserror::Error`, or one whose name says so.
///
/// The derive is the primary signal and the naming convention is the
/// backstop, not the other way round. Names miss: `UnknownCurrency` is a
/// `thiserror` enum that crosses the boundary and reaches a merchant through
/// the same envelope every other error does, and a suffix-only scan had
/// never seen it. Derives miss too — a hand-written `impl std::error::Error`
/// has no derive — so both are kept.
///
/// `pub` only: a private or `pub(crate)` type cannot reach a boundary, so
/// nothing outside its module has to classify it. Input must have been
/// through [`searchable`], which has already removed comments and
/// `#[cfg(test)]` items.
fn scan_error_types(searchable_text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for keyword in ["pub enum ", "pub struct "] {
        for (i, m) in searchable_text.match_indices(keyword) {
            // `pub(crate) enum` does not contain "pub enum", but guard the
            // left edge anyway so a longer identifier ending in "pub" cannot
            // produce a phantom match.
            if searchable_text[..i]
                .chars()
                .next_back()
                .is_some_and(is_ident_char)
            {
                continue;
            }
            let name: String = searchable_text[i + m.len()..]
                .chars()
                .take_while(|c| is_ident_char(*c))
                .collect();
            if name.is_empty() {
                continue;
            }
            if ERROR_SUFFIXES.iter().any(|s| name.ends_with(s)) || derives_error(searchable_text, i)
            {
                out.push(name);
            }
        }
    }
    out
}

/// Whether the attribute block immediately above the declaration at
/// `decl_start` contains a `derive` naming `Error`.
///
/// Walks backwards over consecutive `#[..]` attributes, because the derive is
/// rarely the last one (`#[derive(Debug, thiserror::Error)]` then
/// `#[error("unknown currency: {0}")]` then the declaration). Stops at the
/// first thing that is not an attribute, so a derive on an *unrelated*
/// earlier item cannot bleed onto this one.
fn derives_error(searchable_text: &str, decl_start: usize) -> bool {
    let mut before = &searchable_text[..decl_start];
    loop {
        before = before.trim_end();
        if !before.ends_with(']') {
            return false;
        }
        let Some(open) = matching_open_bracket(before) else {
            return false;
        };
        let Some(attribute) = before.get(open + 1..before.len() - 1) else {
            return false;
        };
        // `#[..]`, not `[..]` — an index expression or a slice type is not
        // an attribute.
        if !before[..open].trim_end().ends_with('#') {
            return false;
        }
        if attribute_derives_error(attribute) {
            return true;
        }
        before = &before[..before[..open].trim_end().len() - 1];
    }
}

/// The byte index of the `[` matching the `]` that ends `text`.
fn matching_open_bracket(text: &str) -> Option<usize> {
    let mut depth = 0usize;
    for (i, c) in text.char_indices().rev() {
        match c {
            ']' => depth += 1,
            '[' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Whether an attribute's body (`derive(Debug, thiserror::Error)`) derives
/// something whose final path segment is `Error`.
///
/// Both spellings count: `thiserror::Error` and a bare `Error` brought in by
/// a `use`. A trait merely *named* like one (`ErrorKind`) does not — the
/// segment must be exactly `Error`.
fn attribute_derives_error(attribute: &str) -> bool {
    let attribute = attribute.trim();
    let Some(list) = attribute
        .strip_prefix("derive")
        .map(str::trim_start)
        .and_then(|rest| rest.strip_prefix('('))
        .and_then(|rest| rest.strip_suffix(')'))
    else {
        return false;
    };
    list.split(',')
        .map(str::trim)
        .any(|path| path.rsplit("::").next().is_some_and(|last| last == "Error"))
}

/// Whether this file implements `Classify` for `name`, in any of the spellings
/// a crate here may use.
fn has_classify_impl(searchable_text: &str, name: &str) -> bool {
    CLASSIFY_IMPL_HEADERS.iter().any(|header| {
        let needle = format!("{header}{name}");
        searchable_text.match_indices(&needle).any(|(i, m)| {
            // `impl Classify for LedgerError` must not be satisfied by
            // `impl Classify for LedgerErrorKind`.
            !searchable_text[i + m.len()..]
                .chars()
                .next()
                .is_some_and(is_ident_char)
        })
    })
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Whether a manifest section declares a dependency on `name`.
///
/// Tighter than a bare `starts_with`: `anyhow` is a short name and a
/// hypothetical `anyhow-derive` is a different crate. Accepts both TOML
/// spellings (`name = "1"`, `name.workspace = true`).
fn declares_dependency(section: &str, name: &str) -> bool {
    section.lines().any(|line| {
        line.trim_start()
            .strip_prefix(name)
            .and_then(|rest| rest.chars().next())
            .is_some_and(|c| c == ' ' || c == '=' || c == '.')
    })
}

fn rust_sources(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "target") {
                continue;
            }
            out.extend(rust_sources(&path));
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dev_dependencies_are_excluded_from_the_runtime_section() {
        let manifest = "\
[dependencies]
axum = \"0.8\"

[dev-dependencies]
vpay-testkit = { path = \"x\" }
";
        let runtime = runtime_dependency_section(manifest);
        assert!(runtime.contains("axum"));
        assert!(!runtime.contains("vpay-testkit"));
    }

    #[test]
    fn scan_finds_tokens_across_wrapped_lines() {
        let text = "Err(ProviderError::NotImplemented(\n    \"orange_money::parse_callback\",\n))";
        assert_eq!(
            scan_not_implemented(text),
            vec!["orange_money::parse_callback"]
        );
    }

    #[test]
    fn scan_ignores_match_arms_and_the_enum_declaration() {
        let text = "NotImplemented(&'static str),\nErr(ProviderError::NotImplemented(_)) => {}";
        assert!(scan_not_implemented(text).is_empty());
    }

    #[test]
    fn scan_finds_several_tokens_in_one_file() {
        let text = r#"NotImplemented("a::b") NotImplemented(
            "c::d"
        )"#;
        assert_eq!(scan_not_implemented(text), vec!["a::b", "c::d"]);
    }

    #[test]
    fn a_runtime_testkit_dependency_is_visible_to_the_check() {
        let manifest = "[dependencies]\nvpay-testkit = { path = \"x\" }\n";
        let runtime = runtime_dependency_section(manifest);
        assert!(runtime.contains("vpay-testkit"));
    }

    // ------------------------------------------------------- verify-errors ---

    fn error_types(source: &str) -> Vec<String> {
        scan_error_types(&searchable(source))
    }

    fn classifies(source: &str, name: &str) -> bool {
        has_classify_impl(&searchable(source), name)
    }

    #[test]
    fn scan_finds_a_pub_error_enum_through_derives_and_doc_comments() {
        let source = "\
/// Everything the ledger can refuse to do.
///
/// Not `pub enum NotThisOne` — this line is a comment.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum LedgerError {
    #[error(\"unbalanced\")]
    Unbalanced,
}
";
        assert_eq!(error_types(source), vec!["LedgerError"]);
    }

    #[test]
    fn scan_finds_pub_error_structs_and_rejections() {
        let source =
            "pub struct UnknownCurrencyError(String);\npub enum AuthRejection { MissingHeader }";
        assert_eq!(
            error_types(source),
            vec!["AuthRejection", "UnknownCurrencyError"]
        );
    }

    #[test]
    fn scan_finds_a_thiserror_type_whose_name_says_nothing() {
        // The type that was invisible to a suffix-only scan: it derives
        // `thiserror::Error`, crosses the boundary, and reaches a merchant
        // through the same envelope as everything else.
        let source = "\
#[derive(Debug, thiserror::Error)]
#[error(\"unknown currency: {0}\")]
pub struct UnknownCurrency(pub String);
";
        assert_eq!(error_types(source), vec!["UnknownCurrency"]);
    }

    #[test]
    fn a_bare_error_derive_counts_as_well_as_the_qualified_one() {
        // `use thiserror::Error;` then `#[derive(Error)]` is the other
        // spelling, and it is just as much an error type.
        assert_eq!(
            error_types("#[derive(Debug, Error)]\npub enum Whatever { A }"),
            vec!["Whatever"]
        );
    }

    #[test]
    fn a_derive_that_is_not_error_does_not_make_a_type_an_error() {
        for source in [
            "#[derive(Debug, Clone, Serialize)]\npub struct Money(i64);",
            // Not `Error`: the final path segment has to match exactly, or
            // every `ErrorKind`-ish helper type would demand a `Classify`.
            "#[derive(Debug, thiserror::ErrorKind)]\npub enum Whatever { A }",
            // An attribute that is not a derive at all.
            "#[non_exhaustive]\npub struct Money(i64);",
        ] {
            assert!(error_types(source).is_empty(), "matched: {source}");
        }
    }

    #[test]
    fn a_derive_on_an_earlier_item_does_not_bleed_onto_a_later_one() {
        let source = "\
#[derive(Debug, thiserror::Error)]
pub enum RealError { A }

pub struct Innocent(i64);
";
        assert_eq!(error_types(source), vec!["RealError"]);
    }

    #[test]
    fn a_block_comment_neither_declares_nor_classifies() {
        // Same rule as line comments: the check must not be satisfiable, or
        // trippable, by writing about the code.
        let commented_impl = "/* impl Classify for LedgerError {} */\npub enum LedgerError { A }";
        assert!(!classifies(commented_impl, "LedgerError"));

        let commented_decl = "/*\n#[derive(thiserror::Error)]\npub enum GhostError {}\n*/";
        assert!(error_types(commented_decl).is_empty());

        // A `/*` inside a string literal opens no comment, so the
        // declaration after it is still found.
        let in_a_string = "const GLOB: &str = \"/*\";\npub enum RealError { A }";
        assert_eq!(error_types(in_a_string), vec!["RealError"]);
    }

    #[test]
    fn a_type_declared_inside_a_cfg_test_module_needs_no_classification() {
        // It reaches no boundary, so requiring an impl for it would be noise
        // — and `vpay-core`'s own test module declares exactly this shape.
        let source = "\
#[cfg(test)]
mod tests {
    #[derive(Debug, thiserror::Error)]
    #[error(\"leaf\")]
    pub struct Leaf(&'static str);

    pub enum WrapperError { A }
}
";
        assert!(error_types(source).is_empty());
    }

    #[test]
    fn an_impl_gated_on_any_test_does_not_classify_anything() {
        // The exact spelling a reviewer used to slip past the literal
        // `#[cfg(test)]` match: gated on `any(test, ..)`, compiled only
        // under `cargo test`, so absent from every production build.
        let text = "pub struct Probe;\n\
                    #[cfg(any(test, feature = \"unused\"))]\n\
                    impl vpay_core::Classify for Probe { fn category(&self) -> C { C::Internal } }\n";
        assert!(!has_classify_impl(&searchable(text), "Probe"));
        let text = "pub struct Probe;\n\
                    #[cfg(all(test, unix))]\n\
                    impl vpay_core::Classify for Probe { fn category(&self) -> C { C::Internal } }\n";
        assert!(!has_classify_impl(&searchable(text), "Probe"));
    }

    #[test]
    fn an_impl_gated_on_not_test_is_production_code_and_classifies() {
        let text = "pub struct Probe;\n\
                    #[cfg(not(test))]\n\
                    impl vpay_core::Classify for Probe { fn category(&self) -> C { C::Internal } }\n";
        assert!(has_classify_impl(&searchable(text), "Probe"));
    }

    #[test]
    fn an_impl_inside_a_cfg_test_module_does_not_classify_anything() {
        // The direction that matters: an impl that only exists under
        // `cargo test` satisfies no caller in production, so it must not
        // satisfy the check either.
        let source = "\
pub enum LedgerError { A }

#[cfg(test)]
mod tests {
    impl vpay_core::Classify for LedgerError {}
}
";
        assert_eq!(error_types(source), vec!["LedgerError"]);
        assert!(!classifies(source, "LedgerError"));
    }

    #[test]
    fn an_unbalanced_brace_in_a_test_string_does_not_swallow_the_rest_of_the_file() {
        // A real line from `vpay-api`'s tests. Counting braces without
        // skipping string literals would delete everything after it —
        // including live declarations — and the check would pass by finding
        // nothing.
        let source = "\
#[cfg(test)]
mod tests {
    fn t() {
        let _ = serde_json::from_str::<Payload>(\"{\");
    }
}

pub enum RealError { A }
";
        assert_eq!(error_types(source), vec!["RealError"]);
    }

    #[test]
    fn a_cfg_test_item_with_no_block_ends_at_its_semicolon() {
        let source = "\
#[cfg(test)]
use something::Else;

pub enum RealError { A }
";
        assert_eq!(error_types(source), vec!["RealError"]);
    }

    #[test]
    fn scan_ignores_types_that_are_not_named_as_errors() {
        let source = "pub enum Foo { A }\npub struct Money(i64);\npub enum PaymentIntentStatus {}";
        assert!(error_types(source).is_empty());
    }

    #[test]
    fn scan_ignores_non_pub_error_types() {
        // A `pub(crate)` or private error reaches no boundary, so nothing has
        // to classify it.
        let source = "pub(crate) enum InternalError { A }\nenum PrivateError { B }";
        assert!(error_types(source).is_empty());
    }

    #[test]
    fn scan_survives_a_rustfmt_wrapped_declaration() {
        let source = "pub\n    enum   WrappedError\n{\n}";
        assert_eq!(error_types(source), vec!["WrappedError"]);
    }

    #[test]
    fn all_three_impl_spellings_count_as_classification() {
        for source in [
            "impl Classify for LedgerError {\n    fn category(&self) -> Category { todo }\n}",
            "impl vpay_core::Classify for LedgerError {}",
            "impl vpay_core::error::Classify for LedgerError {}",
            "impl crate::error::Classify for LedgerError {}",
        ] {
            assert!(classifies(source, "LedgerError"), "not matched: {source}");
        }
    }

    #[test]
    fn an_impl_for_a_longer_name_does_not_classify_the_shorter_one() {
        let source = "impl vpay_core::Classify for LedgerErrorKind {}";
        assert!(!classifies(source, "LedgerError"));
        assert!(classifies(source, "LedgerErrorKind"));
    }

    #[test]
    fn a_doc_comment_mentioning_the_impl_does_not_classify_anything() {
        // The check must not be satisfiable by writing about the impl.
        let source = "/// See `impl Classify for LedgerError` in the ADR.\npub enum LedgerError {}";
        assert!(!classifies(source, "LedgerError"));
    }

    #[test]
    fn an_unrelated_trait_impl_does_not_classify() {
        let source = "impl IntoResponse for AuthRejection {}\nimpl Display for AuthRejection {}";
        assert!(!classifies(source, "AuthRejection"));
    }

    #[test]
    fn a_runtime_anyhow_dependency_is_visible_to_the_check() {
        let manifest = "[dependencies]\nthiserror.workspace = true\nanyhow = \"1\"\n";
        assert!(declares_dependency(
            &runtime_dependency_section(manifest),
            "anyhow"
        ));
    }

    #[test]
    fn a_dev_only_anyhow_dependency_is_allowed() {
        let manifest = "[dependencies]\nthiserror.workspace = true\n\n[dev-dependencies]\nanyhow.workspace = true\n";
        assert!(!declares_dependency(
            &runtime_dependency_section(manifest),
            "anyhow"
        ));
    }

    #[test]
    fn a_different_crate_whose_name_starts_with_anyhow_is_not_anyhow() {
        let manifest = "[dependencies]\nanyhow-derive = \"1\"\n";
        assert!(!declares_dependency(
            &runtime_dependency_section(manifest),
            "anyhow"
        ));
    }

    #[test]
    fn the_workspace_spelling_of_a_dependency_is_recognised() {
        assert!(declares_dependency("anyhow.workspace = true", "anyhow"));
        assert!(declares_dependency(
            "anyhow = { version = \"1\" }",
            "anyhow"
        ));
    }
}
