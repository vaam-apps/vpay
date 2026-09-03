//! Repository automation. Run via `cargo xtask <cmd>` or `just`.
//!
//! Three of these commands exist to enforce promises this repository makes
//! about itself, because a promise nothing checks is a promise that decays:
//!
//! * `verify-no-mocks`  — no test double is reachable from a shipping binary.
//! * `verify-status`    — every `NotImplemented` is declared in `docs/status.md`.
//! * `verify-errors`    — every error type classifies itself, and `anyhow`
//!   stays at the process edge (`docs/adr/0011-error-modelling.md`).
//!
//! One reports rather than enforcing:
//!
//! * `verify-docs`      — doc-comment volume, long functions, ```` ```ignore ````
//!   fences and `#[allow]`s. It **never fails a build**: Step 7's decision (4)
//!   is that a comment budget is a report, because the cheapest way to pass a
//!   ratio gate is to delete the `# Errors` sections ADR-0011 depends on.
//!
//! One does real work rather than checking:
//!
//! * `gen-signing-key`  — generates the RS256 key the OP signs with, offline,
//!   for an operator to load into a Kubernetes Secret.
//!
//! # Dependencies
//!
//! The three `verify-*` commands take no dependencies at all and match on
//! text rather than on types — see [`has_classify_impl`] for what that costs.
//! That is still true of them. `gen-signing-key` is what put four crates
//! (`rsa`, `rand`, `sha2`, `base64`) in this crate's manifest: generating an
//! RSA key and computing an RFC 7638 thumbprint cannot be done by string
//! matching. They are all already in the workspace lockfile, so nothing new
//! is *fetched*; the verify gates simply compile a little more before they
//! run. What was deliberately *not* done is taking a dependency on
//! `vpay-api` to share its thumbprint code — that would drag axum, sqlx and
//! the whole authkestra stack into every CI convention check. See
//! [`rfc7638_thumbprint`] for how the resulting duplication is kept honest.

// This is a CLI; stdout is its output medium, not stray debugging.
#![allow(clippy::print_stdout)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().cloned().unwrap_or_else(|| "help".into());
    let root = repo_root();

    let result = match cmd.as_str() {
        "verify-no-mocks" => verify_no_mocks(&root),
        "verify-status" => verify_status(&root),
        "verify-errors" => verify_errors(&root),
        "verify-all" => verify_no_mocks(&root)
            .and_then(|()| verify_status(&root))
            .and_then(|()| verify_errors(&root)),
        // Not `Result`-shaped like the three gates above, and that is the
        // point: there is nothing here for a caller to fail on. See
        // `verify_docs`.
        "verify-docs" => {
            verify_docs(&root);
            Ok(())
        }
        "gen-signing-key" => gen_signing_key(&args),
        "help" | "--help" | "-h" => {
            println!(
                "usage: cargo xtask <verify-no-mocks|verify-status|verify-errors|verify-all>\n\
                 \x20      cargo xtask verify-docs        (a report; never fails)\n\
                 \x20      cargo xtask gen-signing-key --out <dir>"
            );
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

/// The same two, by package name, for the `cargo metadata` walk.
const SHIPPING_PACKAGES: [&str; 2] = ["vpay-server", "vpay-worker-bin"];

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

    // Nothing outside the testkit itself may reference a stub adapter type,
    // and no shipping binary may open a pool that pretends to be connected.
    for src in rust_sources(&root.join("backends/apps")) {
        let text = fs::read_to_string(&src).unwrap_or_default();
        for violation in app_source_violations(&text) {
            problems.push(format!("{} {violation}", src.display()));
        }
    }

    // The whole graph, not just the two app manifests — see
    // `test_only_reachable_from` and `test_only_declared_by_a_workspace_member`
    // for what the manifest scan above could not see, and why the two rules
    // catch different halves of the same defect.
    let metadata = cargo_metadata(root)?;
    problems.extend(test_only_reachable_from(&metadata, &SHIPPING_PACKAGES));
    problems.extend(test_only_declared_by_a_workspace_member(&metadata));

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

/// Type names a shipping binary must not so much as spell.
///
/// Matched against the raw file, comments included: a binary that names a
/// stub adapter in a comment is describing a code path someone intended, and
/// the point of ADR-0006 is that there is nothing to describe.
const STUB_ADAPTER_NAMES: [&str; 4] = ["MockAdapter", "FakeAdapter", "StubAdapter", "DummyAdapter"];

/// What is wrong with one source file under `backends/apps`, phrased to
/// follow the file's path in a violation line.
///
/// Split out of [`verify_no_mocks`] so it can be driven over a synthetic
/// file: the check reads the two shipping binaries, so the only way to see
/// it fire without breaking them is to hand it text.
///
/// The `connect_lazy` half is the one that needs explaining.
/// `vpay_db::connect_lazy` is not a test double — the pool is the real
/// `sqlx` one and every query really reaches Postgres — which is exactly why
/// no linter and no `[dev-dependencies]` rule would ever object to it. What
/// it *does* is defeat the property `vpay_db::connect` exists to hold: a
/// process that cannot reach its database must fail at boot rather than at
/// the first payment. A binary that opened its pool lazily would report
/// itself started, pass its own readiness probe, and discover Postgres was
/// gone at the moment a merchant's confirm arrived. `connect_lazy` is public
/// solely so `vpay-api`'s unit tests can prove what an unreachable database
/// answers; this is the guard that keeps it there.
///
/// Comments and `#[cfg(test)]` items are stripped for that half (a binary's
/// own tests may legitimately want a pool that never connects, and a comment
/// naming the function is documentation) but deliberately *not* for
/// [`STUB_ADAPTER_NAMES`].
fn app_source_violations(text: &str) -> Vec<String> {
    let mut out: Vec<String> = STUB_ADAPTER_NAMES
        .iter()
        .filter(|needle| text.contains(**needle))
        .map(|needle| format!("mentions `{needle}`"))
        .collect();

    if searchable(text).contains("connect_lazy") {
        out.push(
            "calls `vpay_db::connect_lazy` outside `#[cfg(test)]`; a shipping binary opens \
             its pool with `vpay_db::connect`, which fails at boot when the database is \
             unreachable rather than at the first payment (ADR-0006)"
                .to_owned(),
        );
    }

    out
}

/// Runs `cargo metadata` and returns the parsed document.
///
/// The two `verify-*` checks that came before this one match on text and take
/// no dependencies, on purpose (see the module docs). This one cannot: "is a
/// test double reachable from a shipping binary" is a question about the
/// resolved dependency *graph*, including which edges are `dev` and which are
/// not, and a manifest grep cannot answer it — that is exactly the hole this
/// function exists to close. `serde_json` is already in the workspace
/// lockfile, so nothing new is fetched.
///
/// No `--all-features`: enabling optional dependencies would report edges the
/// shipping build does not have, and a check that cries wolf gets disabled.
/// No `--offline` either — a fresh clone with no registry cache should fail
/// loudly here rather than silently pass a supply-chain gate.
fn cargo_metadata(root: &Path) -> Result<serde_json::Value, String> {
    let output = std::process::Command::new(std::env::var("CARGO").unwrap_or("cargo".into()))
        .args(["metadata", "--format-version", "1"])
        .current_dir(root)
        .output()
        .map_err(|e| format!("running `cargo metadata`: {e}"))?;

    if !output.status.success() {
        return Err(format!(
            "`cargo metadata` failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    serde_json::from_slice(&output.stdout).map_err(|e| format!("parsing `cargo metadata`: {e}"))
}

/// Every [`TEST_ONLY`] crate reachable from `roots` through a non-dev edge.
///
/// # The hole this revealed, and the one it closes
///
/// [`verify_no_mocks`]'s original check read the two app manifests and
/// nothing else, so a test double one hop away was invisible: `vpay-testkit`
/// declared the Rust `wiremock` crate under `[dependencies]` — a *runtime*
/// dependency of a crate whose entire reason to exist is being test-only —
/// and the gate that exists to prevent exactly that was green for months.
/// Nothing in the manifests of `vpay-server` or `vpay-worker-bin` mentioned
/// wiremock, and nothing ever would; the defect was in the middle of the
/// graph. ADR-0006's rule is about *reachability*, so a check has to be
/// too — but note that this walk does **not** catch that historical case:
/// `vpay-testkit` is a dev-dependency everywhere, so it is unreachable from
/// either binary, and a graph walk alone would have called it clean. That
/// case is caught by [`test_only_declared_by_a_workspace_member`], which
/// refuses a test-only crate under any workspace member's `[dependencies]`.
/// What *this* walk closes is the other half: a crate on a non-dev path from
/// a binary that itself pulls a test double, which the manifest scan could
/// never see.
///
/// # What counts as an edge
///
/// Only `dep_kinds` entries whose `kind` is null — a normal dependency.
/// `dev` is excluded because a dev-dependency is not linked into the binary
/// (that is the whole permission ADR-0006 grants), and `build` is excluded
/// because a build script's dependency runs at compile time and ships in
/// nothing — the same line the manifest scan already drew by ignoring
/// `[build-dependencies]`.
///
/// Takes the parsed document rather than running cargo itself, so the walk
/// can be proven against a synthetic graph — including graphs this workspace
/// does not have and must never grow.
fn test_only_reachable_from(metadata: &serde_json::Value, roots: &[&str]) -> Vec<String> {
    let Some(nodes) = metadata
        .get("resolve")
        .and_then(|resolve| resolve.get("nodes"))
        .and_then(serde_json::Value::as_array)
    else {
        // A document with no resolve graph is not "no violations": it is a
        // check that did not run, and this one must never pass by finding
        // nothing.
        return vec!["`cargo metadata` returned no resolve graph to walk".to_owned()];
    };

    // id -> (name, non-dev dependency ids)
    let mut graph: BTreeMap<&str, (&str, Vec<&str>)> = BTreeMap::new();
    for node in nodes {
        let Some(id) = node.get("id").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let deps = node
            .get("deps")
            .and_then(serde_json::Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let linked = deps
            .iter()
            .filter(|dep| {
                dep.get("dep_kinds")
                    .and_then(serde_json::Value::as_array)
                    .is_some_and(|kinds| {
                        kinds
                            .iter()
                            .any(|kind| kind.get("kind").is_none_or(serde_json::Value::is_null))
                    })
            })
            .filter_map(|dep| dep.get("pkg").and_then(serde_json::Value::as_str))
            .collect();
        graph.insert(id, (package_name_of(metadata, id).unwrap_or(id), linked));
    }

    let mut problems = Vec::new();
    for root_name in roots {
        let Some(root_id) = graph
            .iter()
            .find(|(_, (name, _))| name == root_name)
            .map(|(id, _)| *id)
        else {
            // A renamed or removed binary must fail the check, not silently
            // shrink it — the same instinct `verify_errors` applies to an
            // empty crate directory.
            problems.push(format!(
                "`{root_name}` is not in the resolve graph; verify-no-mocks cannot check it"
            ));
            continue;
        };

        let mut seen: BTreeSet<&str> = BTreeSet::new();
        let mut queue = vec![root_id];
        while let Some(id) = queue.pop() {
            if !seen.insert(id) {
                continue;
            }
            let Some((name, deps)) = graph.get(id) else {
                continue;
            };
            if id != root_id && TEST_ONLY.contains(name) {
                problems.push(format!(
                    "`{name}` is reachable from `{root_name}` through non-dev dependencies"
                ));
            }
            queue.extend(deps.iter().copied());
        }
    }

    problems.sort_unstable();
    problems.dedup();
    problems
}

/// Pairs of (workspace member, [`TEST_ONLY`] crate) that may legitimately be a
/// runtime dependency.
///
/// `vpay-testkit` *wraps* testcontainers — starting a real
/// `postgres:16-alpine` or `wiremock/wiremock` container is the thing it
/// exists to do, and a container is not a test double (ADR-0006 says a stub
/// rail **is** a WireMock host reached over HTTP). So these two edges are the
/// intent, not a violation.
///
/// `wiremock` — the Rust crate, an *in-process* HTTP double — is deliberately
/// absent, for any member. Its only legitimate use is as a dev-dependency of
/// a test target that stubs something over loopback inside its own process;
/// as a runtime dependency of a library it is the beginning of an in-process
/// rail stub, which is the one thing ADR-0006 forbids outright.
const TEST_ONLY_RUNTIME_ALLOWED: [(&str, &str); 2] = [
    ("vpay-testkit", "testcontainers"),
    ("vpay-testkit", "testcontainers-modules"),
];

/// Every workspace member that lists a [`TEST_ONLY`] crate as a *runtime*
/// dependency, allowlist aside.
///
/// # Why this exists next to the reachability walk
///
/// They catch different halves. [`test_only_reachable_from`] answers
/// ADR-0006's literal question — is a double linked into a shipping binary —
/// and by construction it says nothing about a crate the binaries reach only
/// through dev edges. That is exactly where the real defect sat: `wiremock`
/// was a runtime dependency of `vpay-testkit`, and `vpay-testkit` is a
/// dev-dependency everywhere, so the graph walk alone would have called it
/// clean (verified, not assumed: re-adding that single line leaves the walk
/// green, and adding one non-dev edge onto the testkit makes it red).
///
/// It was still a defect worth failing on. AGENTS.md's rule is the stronger
/// one — these crates "may appear **only** under `[dev-dependencies]`" — and
/// a test-only crate carrying an in-process double as a runtime dependency is
/// a loaded gun in the middle of the graph: the day anything takes a non-dev
/// edge to it, the double ships.
fn test_only_declared_by_a_workspace_member(metadata: &serde_json::Value) -> Vec<String> {
    let members: BTreeSet<&str> = metadata
        .get("workspace_members")
        .and_then(serde_json::Value::as_array)
        .map(|ids| {
            ids.iter()
                .filter_map(serde_json::Value::as_str)
                .collect::<BTreeSet<&str>>()
        })
        .unwrap_or_default();

    let Some(packages) = metadata
        .get("packages")
        .and_then(serde_json::Value::as_array)
    else {
        return vec!["`cargo metadata` returned no packages to check".to_owned()];
    };

    let mut problems = Vec::new();
    for package in packages {
        let Some(id) = package.get("id").and_then(serde_json::Value::as_str) else {
            continue;
        };
        if !members.contains(id) {
            continue;
        }
        let Some(name) = package.get("name").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let dependencies = package
            .get("dependencies")
            .and_then(serde_json::Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();

        for dependency in dependencies {
            // `kind` is absent or null for a normal dependency; "dev" and
            // "build" are the two that never ship.
            let is_runtime = dependency
                .get("kind")
                .is_none_or(serde_json::Value::is_null);
            let Some(dep_name) = dependency.get("name").and_then(serde_json::Value::as_str) else {
                continue;
            };
            if is_runtime
                && TEST_ONLY.contains(&dep_name)
                && !TEST_ONLY_RUNTIME_ALLOWED.contains(&(name, dep_name))
            {
                problems.push(format!(
                    "workspace member `{name}` lists test-only crate `{dep_name}` under \
                     [dependencies]; it belongs under [dev-dependencies]"
                ));
            }
        }
    }

    problems.sort_unstable();
    problems.dedup();
    problems
}

/// The `name` of the package with this id, from `metadata.packages`.
///
/// Package ids are opaque (their spelling changed in Cargo 1.77 and may again),
/// so the name is looked up rather than parsed out of the id.
fn package_name_of<'a>(metadata: &'a serde_json::Value, id: &str) -> Option<&'a str> {
    metadata
        .get("packages")?
        .as_array()?
        .iter()
        .find(|package| package.get("id").and_then(serde_json::Value::as_str) == Some(id))?
        .get("name")?
        .as_str()
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

/// The heading in `docs/status.md` whose bullet list is the declaration this
/// check compares the code against.
///
/// A named section rather than "anywhere in the file", because the check now
/// runs in both directions and the two directions must read the *same* list.
/// Matching a token anywhere in the prose would let a token that is only
/// mentioned in a sentence satisfy the code→docs direction while the
/// docs→code direction (which has to enumerate) never sees it — two rules
/// disagreeing about what "declared" means is how a status page starts
/// lying.
const STATUS_TOKEN_HEADING: &str = "### Unimplemented items tracked by `verify-status`";

/// Fail if the code claims something is unbuilt that `docs/status.md` does not
/// declare — or vice versa. Keeps the status page honest by construction.
///
/// # Both directions, and why the second one is the one that rots
///
/// `AGENTS.md` promises this check "fails in both directions". Only the first
/// half was implemented: a `NotImplemented("…")` token with no bullet failed
/// the build, but a bullet naming a token no code carries passed silently.
/// That is the direction that actually rots, because it is what happens
/// *every time something gets built*: the implementer deletes the token,
/// forgets the bullet, and the status page goes on advertising an
/// unimplemented item that has shipped. A status page that under-claims is
/// still a lie, and it is the lie that makes people stop reading it.
///
/// # Only shipping code counts
///
/// The scan skips `tests/` directories and `#[cfg(test)]` items, exactly as
/// `verify-errors` does and for the same reason: a token inside a test is a
/// *fixture* — `vpay-worker`'s error tests build a
/// `ProviderError::NotImplemented("mtn_momo::submit")` to assert how it
/// classifies — and it names nothing a merchant can reach. Counting fixtures
/// would force `docs/status.md` to declare items that are implemented, which
/// is precisely the false claim the docs→code direction exists to catch.
fn verify_status(root: &Path) -> Result<(), String> {
    let status_path = root.join("docs/status.md");
    let status = fs::read_to_string(&status_path)
        .map_err(|e| format!("docs/status.md: {e} (the status page is mandatory)"))?;
    let declared = declared_tokens(&status)?;

    let mut found = BTreeSet::new();
    for src in rust_sources(&root.join("backends")) {
        // A token in an integration test is a fixture, not a shipping claim.
        if src.components().any(|c| c.as_os_str() == "tests") {
            continue;
        }
        let text = fs::read_to_string(&src).unwrap_or_default();
        // Scan the whole file, not line by line: rustfmt wraps long calls, and
        // a line-based scan silently under-reports. That would make this check
        // pass while an unimplemented path went undeclared — the exact failure
        // it exists to prevent. `searchable` drops comments and `#[cfg(test)]`
        // items first, so neither a doc comment quoting a token nor a unit
        // test constructing one counts as shipping code.
        found.extend(scan_not_implemented(&searchable(&text)));
    }

    let mut problems = Vec::new();
    let undeclared: Vec<&str> = found
        .iter()
        .filter(|token| !declared.contains(*token))
        .map(String::as_str)
        .collect();
    if !undeclared.is_empty() {
        problems.push(format!(
            "these unimplemented items are missing from docs/status.md under\n  \
             `{STATUS_TOKEN_HEADING}`:\n  - {}",
            undeclared.join("\n  - ")
        ));
    }
    let unbuilt: Vec<&str> = declared
        .iter()
        .filter(|token| !found.contains(*token))
        .map(String::as_str)
        .collect();
    if !unbuilt.is_empty() {
        problems.push(format!(
            "docs/status.md declares these unimplemented items and no shipping code carries \
             them\n  (they were built, or renamed, and the status page still advertises them \
             as gaps):\n  - {}",
            unbuilt.join("\n  - ")
        ));
    }

    if !problems.is_empty() {
        return Err(problems.join("\n"));
    }

    println!(
        "verify-status: ok — {} unimplemented item(s), all declared in docs/status.md and all \
         still in shipping code",
        found.len()
    );
    Ok(())
}

/// The tokens listed under [`STATUS_TOKEN_HEADING`], one per `- \`token\`` bullet.
///
/// Stops at the next heading of any level, so an item added to a *later*
/// section is not silently adopted into this list.
///
/// A missing heading is a hard error rather than an empty list: an empty
/// list would make the docs→code direction vacuous and the code→docs
/// direction fail on every token at once, and the real cause — someone
/// renamed the section — would appear nowhere in the message.
fn declared_tokens(status: &str) -> Result<BTreeSet<String>, String> {
    let after = status
        .split_once(STATUS_TOKEN_HEADING)
        .map(|(_, rest)| rest);
    let Some(after) = after else {
        return Err(format!(
            "docs/status.md has no `{STATUS_TOKEN_HEADING}` section; it is where every \
             ProviderError::NotImplemented token is declared (AGENTS.md rule 2)"
        ));
    };

    let mut out = BTreeSet::new();
    for line in after.lines() {
        let line = line.trim();
        if line.starts_with('#') {
            break;
        }
        let Some(bullet) = line.strip_prefix("- ") else {
            continue;
        };
        let Some(quoted) = bullet.trim().strip_prefix('`') else {
            continue;
        };
        if let Some((token, _)) = quoted.split_once('`') {
            out.insert(token.to_owned());
        }
    }
    Ok(out)
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
    let mut delegated = 0usize;

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
                    for (impl_path, impl_text) in &sources {
                        // Only the file that carries both the declaration and
                        // the impl can be checked — `undelegated_from_variants`
                        // needs the variants and the method bodies together —
                        // so it is also the only file that may contribute to
                        // the count. Counting a declaration whose impl lives
                        // elsewhere would print variants nothing verified.
                        if !has_classify_impl(impl_text, &name) {
                            continue;
                        }
                        let undelegated = undelegated_from_variants(impl_text, &name);
                        let swallowed: BTreeSet<&str> =
                            undelegated.iter().map(|(_, v)| v.as_str()).collect();
                        delegated += from_variants(impl_text, &name)
                            .iter()
                            .filter(|variant| !swallowed.contains(variant.as_str()))
                            .count();
                        for (method, variant) in undelegated {
                            problems.push(format!(
                                "{}: `{name}::{variant}` is `#[from]` but `Classify::{method}` \
                                 has no `Self::{variant}` arm — the wildcard would answer for \
                                 the leaf instead of delegating to it (ADR-0011)",
                                relative(root, impl_path),
                            ));
                        }
                    }
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
             {delegated} `#[from]` variant(s) delegate every `Classify` method they \
             match on; anyhow confined to binaries"
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

/// The five methods of `Classify`, in the order the trait declares them.
///
/// A composite that `#[from]`s a leaf must delegate *each* of them (ADR-0011,
/// "Composites do not re-classify"): `category` alone is not enough, because
/// the other four have category-derived defaults that would quietly overrule
/// the leaf's own overrides — `ProviderError::Rejected`'s `public_message`,
/// `Unsupported`'s severity.
const CLASSIFY_METHODS: [&str; 5] = ["category", "code", "retry", "severity", "public_message"];

/// The `{ … }` that starts at or after `from`, balanced, without the braces.
///
/// String literals are honoured: a `#[error("a { b")]` inside the block must
/// not unbalance it.
fn balanced_block(text: &str, from: usize) -> Option<&str> {
    let open = from + text.get(from..)?.find('{')?;
    let body_start = open + 1;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (offset, c) in text.get(open..)?.char_indices() {
        if in_string {
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
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return text.get(body_start..open + offset);
                }
            }
            _ => {}
        }
    }
    None
}

/// The body of the first item whose declaration starts with `header`.
fn item_body<'a>(searchable_text: &'a str, header: &str) -> Option<&'a str> {
    let at = searchable_text.find(header)?;
    balanced_block(searchable_text, at)
}

/// Every variant of `enum name` that carries a `#[from]` field.
///
/// Text-scanned like the rest of this file, tracking bracket depth so a
/// `#[from]` on a field is attributed to the variant that encloses it and a
/// nested type never reads as a variant name.
fn from_variants(searchable_text: &str, name: &str) -> Vec<String> {
    let Some(body) = item_body(searchable_text, &format!("pub enum {name} ")) else {
        return Vec::new();
    };
    let mut out: Vec<String> = Vec::new();
    let mut current: Option<String> = None;
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escaped = false;
    let mut ident_start: Option<usize> = None;
    for (offset, c) in body.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        if let Some(start) = ident_start
            && !is_ident_char(c)
        {
            if let Some(ident) = body.get(start..offset)
                && ident.starts_with(char::is_uppercase)
            {
                current = Some(ident.to_owned());
            }
            ident_start = None;
        }
        match c {
            '"' => in_string = true,
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            '#' if depth > 0 => {
                if body
                    .get(offset..)
                    .is_some_and(|rest| rest.starts_with("#[from]"))
                    && let Some(variant) = &current
                    && !out.contains(variant)
                {
                    out.push(variant.clone());
                }
            }
            _ => {
                if depth == 0 && ident_start.is_none() && (c.is_alphabetic() || c == '_') {
                    ident_start = Some(offset);
                }
            }
        }
    }
    out
}

/// Every spelling of "this method's answer depends on which variant `self`
/// is", so a method that discriminates cannot escape the check below by
/// discriminating in a form the scan does not know.
///
/// `match self` was the only form recognised when the check landed, which
/// made the whole rule opt-out: writing the same ladder as `if let
/// Self::A(e) = self { … } else { … }` or `matches!(self, Self::A(_))`
/// silently exempted the method, and an `else` branch answers for an
/// unnamed leaf exactly as a `_ =>` arm does. `match *self` is here for the
/// same reason — it is the ordinary spelling when the arms bind by value.
const SELF_DISCRIMINATING_FORMS: [&str; 5] = [
    "match self",
    "match *self",
    "match &self",
    "if let Self::",
    "matches!(self",
];

/// Every `#[from]` variant that a `Classify` method which discriminates on
/// `self` does not name.
///
/// The failure this catches: a new `#[from] SomeLeaf` variant added to
/// `ApiError` or `JobError` compiles the moment the existing `_ =>` arm
/// swallows it, and the leaf's own `code`/`retry`/`severity` are silently
/// replaced by the category default — the drift ADR-0011's "composites do not
/// re-classify" exists to stop, and the one kind of ADR-0011 violation that
/// produces no compiler error.
///
/// Only methods that discriminate on `self` (any of
/// [`SELF_DISCRIMINATING_FORMS`]) are checked: a `Classify` impl
/// that answers the same thing for every variant (`RailFailure::category`)
/// has no wildcard to hide in, and a method the impl does not define at all
/// inherits the category-derived default, which is decided by `category` —
/// itself checked here.
fn undelegated_from_variants(searchable_text: &str, name: &str) -> Vec<(String, String)> {
    let variants = from_variants(searchable_text, name);
    if variants.is_empty() {
        return Vec::new();
    }
    let Some(impl_body) = CLASSIFY_IMPL_HEADERS
        .iter()
        .find_map(|header| item_body(searchable_text, &format!("{header}{name} ")))
    else {
        return Vec::new();
    };

    let mut problems = Vec::new();
    for method in CLASSIFY_METHODS {
        let Some(body) = item_body(impl_body, &format!("fn {method}(")) else {
            continue;
        };
        if !SELF_DISCRIMINATING_FORMS
            .iter()
            .any(|form| body.contains(form))
        {
            continue;
        }
        for variant in &variants {
            if !body.contains(&format!("Self::{variant}")) {
                problems.push((method.to_owned(), variant.clone()));
            }
        }
    }
    problems
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

// ---------------------------------------------------------------------------
// verify-docs
// ---------------------------------------------------------------------------

/// The trees the report covers.
///
/// `sdks/rust`, `.xtask` and everything under `frontends/` are deliberately
/// outside it. These numbers exist to be compared against the baseline table
/// in `docs/plans/2026-09-03-step7-cleanup-rework.md` §"(a) Re-measured
/// baselines", which was measured over exactly these two trees; widening the
/// scope would make the before/after the step is judged on incomparable.
const DOC_REPORT_DIRS: [&str; 2] = ["backends/crates", "backends/apps"];

/// A production function at least this long is named in the report.
const LONG_FUNCTION_LINES: usize = 80;

/// How many worst-ratio files the report names.
const WORST_FILES: usize = 10;

/// Doc-comment lines against code lines, for one file or one crate.
#[derive(Debug, Default, Clone, Copy)]
struct DocCounts {
    /// Lines whose first non-space characters are `///` or `//!`.
    doc: usize,
    /// The part of `doc` inside a fenced block, delimiters included — a
    /// compiled example rather than prose.
    example: usize,
    /// Lines that are neither a doc comment, a `//` comment, nor blank.
    code: usize,
    /// Every non-doc line, blanks and `//` comments included — the design's
    /// own denominator, kept so the totals stay comparable with its table.
    other: usize,
}

impl DocCounts {
    /// The half of `doc` that is prose: the number a comment budget is
    /// actually about.
    ///
    /// Splitting the two is not a refinement, it is the difference between a
    /// report that helps and one that lies. Step 7's whole doctest half turns
    /// prose into compiled examples — `vpay-core` went from 1019 prose lines
    /// to 803 while its total doc lines rose from 1043 to 1412, because 585
    /// of them are now examples `just test-doc` runs. Measured on `doc`
    /// alone, doing exactly what the step asked for reads as a 31-point
    /// regression.
    fn prose(&self) -> usize {
        self.doc.saturating_sub(self.example)
    }
}

impl DocCounts {
    fn add(&mut self, rhs: DocCounts) {
        self.doc += rhs.doc;
        self.example += rhs.example;
        self.code += rhs.code;
        self.other += rhs.other;
    }
}

/// A production function of at least [`LONG_FUNCTION_LINES`] lines.
#[derive(Debug)]
struct LongFunction {
    line: usize,
    name: String,
    length: usize,
}

/// Everything the report knows about one file.
#[derive(Debug)]
struct FileReport {
    path: String,
    counts: DocCounts,
    long_functions: Vec<LongFunction>,
    ignore_fences: Vec<usize>,
    allows: Vec<(usize, String)>,
}

/// Print the documentation report. **Never fails**, by construction.
///
/// Step 7's decision (4): the comment budget is a report, not a gate. A hard
/// ratio would put pressure on exactly the `# Errors` and `# Panics` sections
/// [ADR-0011](../../docs/adr/0011-error-modelling.md) and rustdoc depend on,
/// and the cheapest way to pass it would be to delete them. So this returns
/// nothing to fail on — `just verify` runs it after the three gates and the
/// build cannot go red on a number printed here.
///
/// What it measures, per crate under [`DOC_REPORT_DIRS`], over `src/` only
/// (each crate's own `tests/` is a different kind of code and is skipped):
///
/// * doc-comment lines against code lines;
/// * functions of [`LONG_FUNCTION_LINES`] lines or more;
/// * ```` ```ignore ```` doctest fences — a doctest that is compiled by
///   nobody is a claim nothing checks, which is what `just test-doc` exists
///   to stop;
/// * `#[allow]` / `#[expect]` in production code.
///
/// All four are measured on the part of a file *before* its first
/// `#[cfg(test)]` — see [`production_region`].
fn verify_docs(root: &Path) {
    println!(
        "verify-docs: a report, not a gate — it never fails a build (Step 7, decision 4).\n\
         \x20 scope: {} — `src/` only, each crate's own `tests/` excluded, and\n\
         \x20        everything from a file's first `#[cfg(test)]` onward excluded.\n\
         \x20 `code` counts lines that are neither a doc comment, a `//` comment, nor blank.",
        DOC_REPORT_DIRS.join(", ")
    );

    let mut crates: Vec<(String, Vec<FileReport>)> = Vec::new();
    for dir in DOC_REPORT_DIRS {
        let Ok(entries) = fs::read_dir(root.join(dir)) else {
            println!("verify-docs: WARNING — `{dir}` is not readable; it contributed nothing");
            continue;
        };
        let mut crate_dirs: Vec<PathBuf> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .collect();
        crate_dirs.sort();
        for crate_dir in crate_dirs {
            let name = crate_dir
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            let mut files: Vec<FileReport> = rust_sources(&crate_dir)
                .into_iter()
                .filter(|path| path.components().any(|c| c.as_os_str() == "src"))
                .map(|path| {
                    let text = fs::read_to_string(&path).unwrap_or_default();
                    let region = production_region(&text);
                    FileReport {
                        path: relative(root, &path),
                        counts: count_doc_and_code(&region.raw),
                        long_functions: long_functions(&region.cleaned),
                        ignore_fences: ignore_fences(&region.raw),
                        allows: allow_sites(&region),
                    }
                })
                .collect();
            files.sort_by(|a, b| a.path.cmp(&b.path));
            if !files.is_empty() {
                crates.push((name, files));
            }
        }
    }

    if crates.is_empty() {
        println!(
            "verify-docs: WARNING — no crate sources were found under {}. This report \
             measured NOTHING; do not read its silence as a clean result.",
            DOC_REPORT_DIRS.join(" or ")
        );
        return;
    }

    print_ratio_table(&crates);
    print_long_functions(&crates);
    print_ignore_fences(&crates);
    print_allow_sites(&crates);
}

/// `doc / code` as a percentage with one decimal, without floating point —
/// [ADR-0007](../../docs/adr/0007-lint-policy.md) denies float arithmetic
/// workspace-wide and a report is not an exception worth carving.
fn ratio_tenths(doc: usize, code: usize) -> usize {
    if code == 0 {
        return 0;
    }
    doc.saturating_mul(1000) / code
}

fn percent(tenths: usize) -> String {
    format!("{}.{}%", tenths / 10, tenths % 10)
}

fn print_ratio_table(crates: &[(String, Vec<FileReport>)]) {
    let mut total = DocCounts::default();
    let width = crates
        .iter()
        .map(|(name, _)| name.chars().count())
        .max()
        .unwrap_or(4)
        .max(5);
    println!("\ndoc-comment lines against code lines");
    println!(
        "  `prose` is what a comment budget is about; `ex` is doc lines inside a ``` fence,\n  \
         which `just test-doc` compiles. `ratio` is prose/code — an example is not a comment."
    );
    println!(
        "  {:width$}  {:>7}  {:>7}  {:>7}  {:>8}",
        "crate", "prose", "ex", "code", "ratio"
    );
    for (name, files) in crates {
        let mut counts = DocCounts::default();
        for file in files {
            counts.add(file.counts);
        }
        total.add(counts);
        println!(
            "  {:width$}  {:>7}  {:>7}  {:>7}  {:>8}",
            name,
            counts.prose(),
            counts.example,
            counts.code,
            percent(ratio_tenths(counts.prose(), counts.code)),
        );
    }
    println!(
        "  {:width$}  {:>7}  {:>7}  {:>7}  {:>8}",
        "TOTAL",
        total.prose(),
        total.example,
        total.code,
        percent(ratio_tenths(total.prose(), total.code)),
    );
    println!(
        "  the same totals in the design's own convention, which predates the doctests and\n  \
         counts every doc line as prose (denominator = every non-doc line, blanks and `//`\n  \
         comments included): doc {} / code {} = {}",
        total.doc,
        total.other,
        percent(ratio_tenths(total.doc, total.other)),
    );

    let mut worst: Vec<&FileReport> = crates
        .iter()
        .flat_map(|(_, files)| files.iter())
        .filter(|file| file.counts.code > 0)
        .collect();
    worst.sort_by(|a, b| {
        ratio_tenths(b.counts.prose(), b.counts.code)
            .cmp(&ratio_tenths(a.counts.prose(), a.counts.code))
            .then_with(|| b.counts.prose().cmp(&a.counts.prose()))
    });
    println!("\n  the {WORST_FILES} files with the highest prose ratio");
    for file in worst.iter().take(WORST_FILES) {
        println!(
            "    {:>8}  {} (prose {} / ex {} / code {})",
            percent(ratio_tenths(file.counts.prose(), file.counts.code)),
            file.path,
            file.counts.prose(),
            file.counts.example,
            file.counts.code,
        );
    }
}

fn print_long_functions(crates: &[(String, Vec<FileReport>)]) {
    let mut all: Vec<(&FileReport, &LongFunction)> = crates
        .iter()
        .flat_map(|(_, files)| files.iter())
        .flat_map(|file| file.long_functions.iter().map(move |f| (file, f)))
        .collect();
    all.sort_by_key(|(_, func)| std::cmp::Reverse(func.length));
    println!(
        "\nproduction functions of {LONG_FUNCTION_LINES} lines or more ({})",
        all.len()
    );
    for (file, func) in &all {
        println!(
            "  {:>4}  {}:{}  fn {}",
            func.length, file.path, func.line, func.name
        );
    }
}

fn print_ignore_fences(crates: &[(String, Vec<FileReport>)]) {
    let sites: Vec<String> = crates
        .iter()
        .flat_map(|(_, files)| files.iter())
        .flat_map(|file| {
            file.ignore_fences
                .iter()
                .map(move |line| format!("{}:{line}", file.path))
        })
        .collect();
    println!(
        "\n```ignore doctest fences ({}) — an example nothing compiles",
        sites.len()
    );
    for site in &sites {
        println!("  {site}");
    }
}

fn print_allow_sites(crates: &[(String, Vec<FileReport>)]) {
    let sites: Vec<String> = crates
        .iter()
        .flat_map(|(_, files)| files.iter())
        .flat_map(|file| {
            file.allows
                .iter()
                .map(move |(line, text)| format!("{}:{line}  {text}", file.path))
        })
        .collect();
    println!(
        "\n#[allow] / #[expect] in production code ({})",
        sites.len()
    );
    for site in &sites {
        println!("  {site}");
    }
}

/// The part of a file that ships, in two spellings of the same characters.
#[derive(Debug)]
struct Region {
    /// As written — the only form in which a doc comment is still visible.
    raw: String,
    /// Comments and literals blanked, one space per character removed, so a
    /// position in `cleaned` is the same position in `raw`.
    cleaned: String,
}

/// The part of a file that ships: everything before its first `#[cfg(test)]`.
///
/// Deliberately cruder than [`strip_cfg_test_items`], which this file's three
/// *gates* use. This report prints `path:line` for everything it finds, and a
/// scanner that deleted test items out of the middle of a file would renumber
/// every line after them — so each printed line would point somewhere else.
/// Truncating at the first one keeps the numbers real, and costs nothing in
/// this workspace, where tests are written at the end of the file.
///
/// The search runs over [`strip_code_noise`]'s output, not the source: three
/// files in `vpay-worker` and `vpay-worker-bin` *discuss* `#[cfg(test)]` in
/// their module headers, and a scan over raw text stopped at the sentence —
/// dropping `poll_charge`, `run_loop` and both `#[expect]`s out of the first
/// version of this report. That is the failure mode the report exists to
/// avoid, arriving through the report itself.
///
/// The predicate rules are [`match_cfg_test`]'s: `#[cfg(any(test, …))]`
/// truncates, `#[cfg(not(test))]` (the production-only spelling) does not.
fn production_region(text: &str) -> Region {
    let raw: Vec<char> = text.chars().collect();
    let cleaned: Vec<char> = strip_code_noise(text).chars().collect();
    let mut cut = raw.len();
    let mut i = 0usize;
    while i < cleaned.len() {
        if match_cfg_test(&cleaned, i).is_some() {
            cut = i;
            break;
        }
        i += 1;
    }
    Region {
        raw: raw.get(..cut).unwrap_or_default().iter().collect(),
        cleaned: cleaned.get(..cut).unwrap_or_default().iter().collect(),
    }
}

/// Count doc lines, the fenced-example part of them, and code lines in a
/// production region.
///
/// Block comments are removed first (line breaks intact), so a `/* … */`
/// paragraph counts as neither doc nor code — it is prose either way, and
/// counting it as code would make a file look *better* the more of it there
/// is.
///
/// A doc line inside a ```` ``` ```` fence counts as an example, delimiters
/// included, and the fence state resets at the first non-doc line so an
/// unclosed fence in one item cannot swallow the next one's prose.
fn count_doc_and_code(region: &str) -> DocCounts {
    let visible = strip_block_comments(region);
    let mut counts = DocCounts::default();
    let mut in_fence = false;
    for line in visible.lines() {
        let trimmed = line.trim_start();
        let Some(body) = trimmed
            .strip_prefix("///")
            .or_else(|| trimmed.strip_prefix("//!"))
        else {
            in_fence = false;
            counts.other += 1;
            if !trimmed.is_empty() && !trimmed.starts_with("//") {
                counts.code += 1;
            }
            continue;
        };
        counts.doc += 1;
        let fence = body.trim_start().starts_with("```");
        if in_fence || fence {
            counts.example += 1;
        }
        if fence {
            in_fence = !in_fence;
        }
    }
    counts
}

/// Every function in a production region that is [`LONG_FUNCTION_LINES`] or
/// longer, measured from its `fn` line through its closing brace inclusive.
///
/// Comments and string literals are blanked first ([`strip_code_noise`]),
/// because a `format!` body containing `{` is what makes a naive brace-depth
/// scan report a function that runs to the end of the file.
fn long_functions(cleaned: &str) -> Vec<LongFunction> {
    let chars: Vec<char> = cleaned.chars().collect();
    let mut out = Vec::new();
    let mut line = 1usize;
    let mut i = 0usize;
    while let Some(&c) = chars.get(i) {
        if c == '\n' {
            line += 1;
            i += 1;
            continue;
        }
        if is_fn_keyword(&chars, i)
            && let Some(found) = function_at(&chars, i, line)
            && found.length >= LONG_FUNCTION_LINES
        {
            out.push(found);
        }
        i += 1;
    }
    out
}

/// Whether the `fn` keyword — not an identifier containing it — starts at `i`.
fn is_fn_keyword(chars: &[char], i: usize) -> bool {
    if chars.get(i) != Some(&'f') || chars.get(i + 1) != Some(&'n') {
        return false;
    }
    if i > 0 && chars.get(i - 1).is_some_and(|c| is_ident_char(*c)) {
        return false;
    }
    !chars.get(i + 2).is_some_and(|c| is_ident_char(*c))
}

/// The function whose `fn` keyword starts at `start`, if it has a body.
///
/// `None` for a declaration without one (a trait method's signature, an
/// `extern` block): those end at a `;` reached outside every bracket, and a
/// scan that took the *next* `{` in the file would attribute the following
/// item's body to them. The bracket depth is tracked for the same reason a
/// `;` cannot be trusted naively — `fn f(buf: [u8; 4])` carries one inside
/// its own signature.
fn function_at(chars: &[char], start: usize, start_line: usize) -> Option<LongFunction> {
    let mut i = start + 2;
    while chars.get(i).is_some_and(|c| c.is_whitespace()) {
        i += 1;
    }
    let name: String = chars
        .get(i..)?
        .iter()
        .take_while(|c| is_ident_char(**c))
        .collect();
    if name.is_empty() {
        return None;
    }

    let mut braces = 0usize;
    let mut parens = 0usize;
    let mut brackets = 0usize;
    let mut opened = false;
    let mut lines_spanned = 0usize;
    let mut j = start;
    while let Some(&c) = chars.get(j) {
        match c {
            '\n' => lines_spanned += 1,
            '(' => parens += 1,
            ')' => parens = parens.saturating_sub(1),
            '[' => brackets += 1,
            ']' => brackets = brackets.saturating_sub(1),
            '{' => {
                braces += 1;
                opened = true;
            }
            '}' => {
                braces = braces.saturating_sub(1);
                if opened && braces == 0 {
                    return Some(LongFunction {
                        line: start_line,
                        name,
                        length: lines_spanned + 1,
                    });
                }
            }
            ';' if !opened && parens == 0 && brackets == 0 => return None,
            _ => {}
        }
        j += 1;
    }
    None
}

/// The 1-based lines carrying a ```` ```ignore ```` doctest fence.
///
/// Only doc-comment lines are read: a fence quoted in an ordinary `//`
/// comment compiles nothing and promises nothing. The info string is split on
/// commas and whitespace, so ```` ```rust,ignore ```` counts too.
fn ignore_fences(region: &str) -> Vec<usize> {
    let mut out = Vec::new();
    for (index, line) in region.lines().enumerate() {
        let trimmed = line.trim_start();
        if !trimmed.starts_with("///") && !trimmed.starts_with("//!") {
            continue;
        }
        let Some((_, info)) = trimmed.split_once("```") else {
            continue;
        };
        if info
            .split(|c: char| c == ',' || c.is_whitespace())
            .any(|token| token == "ignore")
        {
            out.push(index + 1);
        }
    }
    out
}

/// Every `#[allow]` / `#[expect]` in a production region, with its line.
///
/// Read off the comment- and literal-stripped text so an attribute *quoted*
/// in a doc comment (this file does it twice) or built inside a macro's
/// string is not counted, then reported from the original line so the message
/// shows what was actually written.
fn allow_sites(region: &Region) -> Vec<(usize, String)> {
    let cleaned = &region.cleaned;
    let raw: Vec<&str> = region.raw.lines().collect();
    let mut out = Vec::new();
    for (index, line) in cleaned.lines().enumerate() {
        let hit = ["#[allow(", "#[expect(", "#![allow(", "#![expect("]
            .iter()
            .any(|needle| line.contains(needle));
        if hit {
            out.push((index + 1, attribute_text(&raw, index)));
        }
    }
    out
}

/// The attribute at `raw[index]`, joined across the lines rustfmt wrapped it
/// over.
///
/// Every `#[expect(...)]` in `vpay-worker` carries a `reason = "..."` long
/// enough that rustfmt breaks the attribute over four lines, and reporting
/// the first of them prints `#[expect(` — the one part that says nothing
/// about which lint was silenced or why.
fn attribute_text(raw: &[&str], index: usize) -> String {
    let mut text = String::new();
    for offset in 0..6usize {
        let Some(line) = raw.get(index + offset) else {
            break;
        };
        if offset > 0 {
            text.push(' ');
        }
        text.push_str(line.trim());
        let opens = text.matches('(').count() + text.matches('[').count();
        let closes = text.matches(')').count() + text.matches(']').count();
        if opens <= closes {
            break;
        }
    }
    text.chars().take(120).collect()
}

/// Blank out comments and every literal, keeping each line break in place.
///
/// Line count and line numbers are preserved exactly — the report prints
/// them — so everything removed is replaced by spaces rather than deleted.
/// Raw strings (`r#"…"#`) are handled because `vpay-db` writes SQL in them,
/// and lifetimes are told apart from `char` literals because `'a` is not an
/// unterminated `'`.
fn strip_code_noise(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0usize;
    while let Some(&c) = chars.get(i) {
        // A line comment: blanked to the end of its line.
        if c == '/' && chars.get(i + 1) == Some(&'/') {
            while chars.get(i).is_some_and(|c| *c != '\n') {
                out.push(' ');
                i += 1;
            }
            continue;
        }
        // A block comment, nesting as Rust's do.
        if c == '/' && chars.get(i + 1) == Some(&'*') {
            let mut depth = 1usize;
            out.push_str("  ");
            i += 2;
            while let Some(&inner) = chars.get(i) {
                if inner == '/' && chars.get(i + 1) == Some(&'*') {
                    depth += 1;
                    out.push_str("  ");
                    i += 2;
                    continue;
                }
                if inner == '*' && chars.get(i + 1) == Some(&'/') {
                    depth -= 1;
                    out.push_str("  ");
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                    continue;
                }
                out.push(if inner == '\n' { '\n' } else { ' ' });
                i += 1;
            }
            continue;
        }
        // A raw string: `r`, any number of `#`, then the quote.
        if c == 'r' && !(i > 0 && chars.get(i - 1).is_some_and(|p| is_ident_char(*p))) {
            let mut hashes = 0usize;
            let mut j = i + 1;
            while chars.get(j) == Some(&'#') {
                hashes += 1;
                j += 1;
            }
            if chars.get(j) == Some(&'"') {
                out.push_str(&" ".repeat(j - i + 1));
                i = j + 1;
                while let Some(&inner) = chars.get(i) {
                    if inner == '"' && (0..hashes).all(|k| chars.get(i + 1 + k) == Some(&'#')) {
                        out.push_str(&" ".repeat(hashes + 1));
                        i += hashes + 1;
                        break;
                    }
                    out.push(if inner == '\n' { '\n' } else { ' ' });
                    i += 1;
                }
                continue;
            }
        }
        // An ordinary string, escapes and all.
        if c == '"' {
            out.push(' ');
            i += 1;
            let mut escaped = false;
            while let Some(&inner) = chars.get(i) {
                out.push(if inner == '\n' { '\n' } else { ' ' });
                i += 1;
                if escaped {
                    escaped = false;
                } else if inner == '\\' {
                    escaped = true;
                } else if inner == '"' {
                    break;
                }
            }
            continue;
        }
        // A `char` literal — but `'a` is a lifetime and stays.
        if c == '\'' {
            let escaped = chars.get(i + 1) == Some(&'\\');
            let plain = chars.get(i + 2) == Some(&'\'');
            if escaped || plain {
                out.push(' ');
                i += 1;
                let mut skip = false;
                while let Some(&inner) = chars.get(i) {
                    out.push(if inner == '\n' { '\n' } else { ' ' });
                    i += 1;
                    if skip {
                        skip = false;
                    } else if inner == '\\' {
                        skip = true;
                    } else if inner == '\'' {
                        break;
                    }
                }
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    out
}

#[cfg(test)]
mod doc_report_tests {
    use super::*;

    /// Everything this report prints is a `path:line`, and every line number
    /// comes from counting characters in [`strip_code_noise`]'s output
    /// against the source. If that function ever removed a character instead
    /// of blanking it, every number after the removal would be wrong and
    /// nothing else here would notice.
    #[test]
    fn blanking_noise_changes_no_character_count_and_no_line_count() {
        let source = r####"
//! A header /* with a block comment */ and a "quoted" thing.
/// ```ignore
fn f() -> &'static str {
    let sql = r#"SELECT '{' FROM "t" -- not a comment"#;
    let brace = '{';
    let escaped = '\'';
    "a \" string with // and /* inside"
}
"####;
        let cleaned = strip_code_noise(source);
        assert_eq!(
            cleaned.chars().count(),
            source.chars().count(),
            "a blanked character must still be one character"
        );
        assert_eq!(cleaned.lines().count(), source.lines().count());
        assert!(
            !cleaned.contains("SELECT"),
            "a raw string's body must not survive: {cleaned}"
        );
        assert!(!cleaned.contains("not a comment"));
        assert!(cleaned.contains("fn f"), "code itself must survive");
    }

    #[test]
    fn the_production_region_stops_at_the_first_cfg_test() {
        let region = production_region(
            "fn shipped() {}\n#[cfg(test)]\nmod tests {\n    fn only_in_tests() {}\n}\n",
        );
        assert!(region.raw.contains("fn shipped"));
        assert!(!region.raw.contains("only_in_tests"));
    }

    /// The defect this report shipped with for one run: three files in
    /// `vpay-worker` and `vpay-worker-bin` *discuss* `#[cfg(test)]` in their
    /// module headers, and a scan over raw text stopped at the sentence —
    /// silently dropping two 200-line functions and two `#[expect]`s.
    #[test]
    fn a_cfg_test_written_in_a_comment_or_a_string_does_not_end_the_region() {
        let region = production_region(concat!(
            "//! This module has no `#[cfg(test)]` variant, deliberately.\n",
            "const NOTE: &str = \"#[cfg(test)]\";\n",
            "fn shipped() {}\n",
            "#[cfg(test)]\nmod tests {}\n",
        ));
        assert!(
            region.raw.contains("fn shipped"),
            "the region ended early: {}",
            region.raw
        );
        assert!(!region.raw.contains("mod tests"));
    }

    #[test]
    fn the_production_only_spelling_does_not_end_the_region() {
        let region = production_region("#[cfg(not(test))]\nfn shipped() {}\n");
        assert!(region.raw.contains("fn shipped"));
    }

    #[test]
    fn doc_lines_code_lines_and_everything_else_are_counted_apart() {
        let counts = count_doc_and_code(concat!(
            "//! module doc\n",
            "\n",
            "/// item doc\n",
            "// an ordinary comment\n",
            "/* a block\n",
            "   comment */\n",
            "pub fn f() {}\n",
        ));
        assert_eq!(counts.doc, 2, "`//!` and `///`, and nothing else");
        assert_eq!(counts.code, 1, "only `pub fn f() {{}}`");
        assert_eq!(
            counts.other, 5,
            "the design's denominator: every non-doc line, blanks and comments included"
        );
        assert_eq!(counts.example, 0);
        assert_eq!(counts.prose(), 2);
    }

    /// Step 7 turns prose into compiled examples, and a report that counted
    /// them as comment volume would score doing exactly what the step asked
    /// for as a regression: `vpay-core`'s prose fell 1019 -> 803 while its
    /// total doc lines rose 1043 -> 1412.
    #[test]
    fn a_fenced_example_is_counted_apart_from_prose() {
        let counts = count_doc_and_code(concat!(
            "/// One line of prose.\n",
            "///\n",
            "/// ```\n",
            "/// let x = 1;\n",
            "/// assert_eq!(x, 1);\n",
            "/// ```\n",
            "/// A closing line of prose.\n",
            "pub fn f() {}\n",
        ));
        assert_eq!(counts.doc, 7, "every doc line still counts once");
        assert_eq!(
            counts.example, 4,
            "the two fence delimiters and the two lines between them"
        );
        assert_eq!(counts.prose(), 3);
        assert_eq!(counts.code, 1);
    }

    #[test]
    fn a_fence_does_not_bleed_past_the_item_it_is_written_in() {
        let counts = count_doc_and_code(concat!(
            "/// ```\n",
            "/// let x = 1;\n",
            "pub fn unterminated() {}\n",
            "\n",
            "/// Prose belonging to the next item.\n",
            "pub fn g() {}\n",
        ));
        assert_eq!(
            counts.example, 2,
            "the unclosed fence ends with its own doc block"
        );
        assert_eq!(counts.prose(), 1);
    }

    #[test]
    fn a_fence_with_an_info_string_is_still_a_fence() {
        let counts = count_doc_and_code(concat!(
            "//! ```rust,no_run\n",
            "//! let x = 1;\n",
            "//! ```\n",
            "//! Prose.\n",
        ));
        assert_eq!(counts.example, 3);
        assert_eq!(counts.prose(), 1);
    }

    #[test]
    fn a_brace_inside_a_string_does_not_swallow_the_rest_of_the_file() {
        let mut source = String::from("fn short() {\n    let _ = \"{ { {\";\n}\n");
        source.push_str("fn long() {\n");
        for i in 0..90 {
            source.push_str(&format!("    let _x{i} = \"}}\";\n"));
        }
        source.push_str("}\n");
        let found = long_functions(&production_region(&source).cleaned);
        assert_eq!(found.len(), 1, "only the long one: {found:?}");
        let Some(func) = found.first() else {
            panic!("no function found")
        };
        assert_eq!(func.name, "long");
        assert_eq!(func.line, 4, "the `fn` line, not the first attribute");
        assert_eq!(func.length, 92, "signature through closing brace inclusive");
    }

    #[test]
    fn a_declaration_with_no_body_is_not_a_function_of_any_length() {
        let mut source = String::from("trait T {\n    fn declared(&self) -> u8;\n");
        // A body long enough to be reported, so that mis-attributing the
        // declaration to it would be visible.
        source.push_str("    fn implemented(&self) -> u8 {\n");
        for _ in 0..85 {
            source.push_str("        let _ = 1;\n");
        }
        source.push_str("        1\n    }\n}\n");
        let found = long_functions(&production_region(&source).cleaned);
        assert_eq!(found.len(), 1);
        assert!(found.iter().all(|f| f.name == "implemented"));
    }

    #[test]
    fn a_semicolon_inside_a_signature_does_not_hide_the_body() {
        let mut source = String::from("fn f(buf: [u8; 4]) -> [u8; 4] {\n");
        for _ in 0..85 {
            source.push_str("    let _ = 1;\n");
        }
        source.push_str("    buf\n}\n");
        let found = long_functions(&production_region(&source).cleaned);
        assert_eq!(found.len(), 1, "the `; 4]` is not the end of a declaration");
    }

    #[test]
    fn a_function_one_line_short_of_the_threshold_is_not_reported() {
        // Signature line + body + closing brace, so a function of exactly
        // `n` lines has `n - 2` body lines.
        let of_length = |n: usize| {
            let body = "    let _ = 1;\n".repeat(n - 2);
            let source = format!("fn f() {{\n{body}}}\n");
            let found = long_functions(&production_region(&source).cleaned);
            assert!(
                found.iter().all(|f| f.length == n),
                "a {n}-line function measured as {found:?}"
            );
            found.len()
        };
        assert_eq!(
            of_length(LONG_FUNCTION_LINES - 1),
            0,
            "{LONG_FUNCTION_LINES} is the floor"
        );
        assert_eq!(of_length(LONG_FUNCTION_LINES), 1);
    }

    #[test]
    fn only_an_ignore_fence_in_a_doc_comment_counts() {
        let lines = ignore_fences(concat!(
            "/// ```ignore\n",
            "/// ```rust,ignore\n",
            "//! ```text\n",
            "/// ```no_run\n",
            "/// ```\n",
            "// ```ignore\n",
        ));
        assert_eq!(
            lines,
            vec![1, 2],
            "`text`, `no_run`, a bare fence and an ordinary comment are not it"
        );
    }

    #[test]
    fn an_allow_is_counted_where_it_is_written_and_nowhere_it_is_quoted() {
        let region = production_region(concat!(
            "/// Prefer this to `#[allow(dead_code)]`.\n",
            "const HINT: &str = \"#[expect(clippy::all)]\";\n",
            "#[allow(deprecated)]\n",
            "fn f() {}\n",
            "#![expect(clippy::print_stdout)]\n",
        ));
        let sites = allow_sites(&region);
        assert_eq!(
            sites.len(),
            2,
            "the doc comment and the string literal are not attributes: {sites:?}"
        );
        assert_eq!(
            sites.first().map(|(line, _)| *line),
            Some(3),
            "reported at the line it is written on"
        );
        assert!(
            sites
                .first()
                .is_some_and(|(_, text)| text == "#[allow(deprecated)]"),
            "reported as written: {sites:?}"
        );
        assert_eq!(sites.get(1).map(|(line, _)| *line), Some(5));
    }

    /// Every `#[expect]` in `vpay-worker` carries a `reason =` long enough
    /// that rustfmt breaks the attribute over four lines, and the first of
    /// them is the one part that names neither the lint nor the reason.
    #[test]
    fn an_attribute_rustfmt_wrapped_is_reported_whole() {
        let region = production_region(concat!(
            "#[expect(\n",
            "    clippy::too_many_arguments,\n",
            "    reason = \"every argument is a distinct fact\"\n",
            ")]\n",
            "fn f() {}\n",
        ));
        let sites = allow_sites(&region);
        assert_eq!(sites.len(), 1, "one attribute, not one per line: {sites:?}");
        assert_eq!(
            sites.first().map(|(_, text)| text.as_str()),
            Some(
                "#[expect( clippy::too_many_arguments, \
                 reason = \"every argument is a distinct fact\" )]"
            )
        );
    }

    /// [ADR-0007](../../docs/adr/0007-lint-policy.md) denies float arithmetic
    /// workspace-wide, so the ratio is integer division. A crate with no code
    /// lines must not divide by zero.
    #[test]
    fn the_ratio_is_integer_arithmetic_and_survives_an_empty_crate() {
        assert_eq!(percent(ratio_tenths(3413, 2370)), "144.0%");
        assert_eq!(percent(ratio_tenths(10, 86)), "11.6%");
        assert_eq!(percent(ratio_tenths(0, 0)), "0.0%");
        assert_eq!(percent(ratio_tenths(7, 0)), "0.0%");
    }
}

// ---------------------------------------------------------------------------
// gen-signing-key
// ---------------------------------------------------------------------------

/// The file name written under `--out`. Fixed rather than a flag: this is
/// the name the Kubernetes Secret key and the `--oauth-signing-key-file`
/// path in every deployment manifest are expected to agree on, and one
/// spelling in one place is what keeps them agreeing.
const SIGNING_KEY_FILE: &str = "oauth-signing-key.pem";

/// Modulus size of a generated key.
///
/// 3072, not the 2048 `vpay_api::op::keys` accepts as its floor. The floor is
/// what vpay refuses to go below for a key it is *handed*; this is what vpay
/// generates when the choice is its own, and there is no reason to generate
/// at the minimum. 3072 is NIST SP 800-57's 128-bit-security RSA size, still
/// universally supported by JWT verifiers, and the extra signing cost is
/// irrelevant at token-issuance rates.
const GENERATED_KEY_BITS: usize = 3072;

/// A freshly generated signing key: where it was written, and the public
/// half an operator has to be able to see.
/// `Debug` is safe to derive: every field is public information — a path, a
/// thumbprint and a public key. The private key is deliberately not a field
/// here at all, so it cannot reach a log through this type even by accident.
#[derive(Debug)]
struct GeneratedSigningKey {
    path: PathBuf,
    kid: String,
    public_jwk: String,
}

/// `cargo xtask gen-signing-key --out <dir>` — writes a new RS256 signing
/// key and prints its `kid` and public JWK.
///
/// The private key never leaves the file: nothing here writes it to stdout,
/// to a log, or to the database. `docs/flows/dashboard-auth.md` describes the
/// intended handling — the file's contents become a Kubernetes Secret, the
/// pod mounts it, and `--oauth-signing-key-file` points at the mount.
fn gen_signing_key(args: &[String]) -> Result<(), String> {
    let out =
        flag_value(args, "--out").ok_or_else(|| "gen-signing-key needs --out <dir>".to_string())?;

    let generated = generate_signing_key(Path::new(&out))?;

    println!("wrote {}", generated.path.display());
    println!("  {GENERATED_KEY_BITS}-bit RSA, PKCS#8 PEM, mode 0600");
    println!("kid: {}", generated.kid);
    println!("public JWK:");
    println!("{}", generated.public_jwk);
    println!();
    println!(
        "The private key is in that file and nowhere else. Put it in a Secret, mount it, and \
         point --oauth-signing-key-file / VPAY_OAUTH_SIGNING_KEY_FILE at the mount. vpay derives \
         the kid above from the key itself (RFC 7638), so it needs no separate configuration and \
         every replica computes the same one."
    );

    Ok(())
}

/// The half of [`gen_signing_key`] with no printing in it, so a test can
/// assert on the result rather than on stdout.
///
/// Refuses to overwrite: the file is created with `create_new`, which fails
/// if anything is already there. That is a single atomic syscall rather than
/// an "exists?" check followed by a write — the check-then-write version has
/// a window in which a second invocation can clobber the first one's key,
/// and a clobbered signing key is unrecoverable (every token it signed stops
/// verifying, and the PEM is not stored anywhere else by design).
fn generate_signing_key(out_dir: &Path) -> Result<GeneratedSigningKey, String> {
    use base64::Engine as _;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use rsa::pkcs8::{EncodePrivateKey as _, LineEnding};
    use rsa::traits::PublicKeyParts as _;

    fs::create_dir_all(out_dir).map_err(|e| format!("cannot create {}: {e}", out_dir.display()))?;
    let path = out_dir.join(SIGNING_KEY_FILE);

    let mut rng = rand::rngs::OsRng;
    let key = rsa::RsaPrivateKey::new(&mut rng, GENERATED_KEY_BITS)
        .map_err(|e| format!("RSA key generation failed: {e}"))?;
    let pem = key
        .to_pkcs8_pem(LineEnding::LF)
        .map_err(|e| format!("PKCS#8 encoding failed: {e}"))?;

    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        // Set at creation, not with a later `set_permissions`: between the
        // two there would be a moment in which a private key is readable by
        // everyone with access to the directory.
        options.mode(0o600);
    }
    let mut file = options.open(&path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::AlreadyExists {
            format!(
                "{} already exists; refusing to overwrite a signing key (every token it signed \
                 would stop verifying). Move it aside first if you really mean to replace it.",
                path.display()
            )
        } else {
            format!("cannot write {}: {e}", path.display())
        }
    })?;
    io::Write::write_all(&mut file, pem.as_bytes())
        .map_err(|e| format!("cannot write {}: {e}", path.display()))?;

    #[cfg(not(unix))]
    println!(
        "warning: file modes are only set on unix; check the permissions on {} yourself",
        path.display()
    );

    let n = URL_SAFE_NO_PAD.encode(key.n().to_bytes_be());
    let e = URL_SAFE_NO_PAD.encode(key.e().to_bytes_be());
    let kid = rfc7638_thumbprint(&n, &e);

    Ok(GeneratedSigningKey {
        public_jwk: format!(
            r#"{{"kty":"RSA","n":"{n}","e":"{e}","alg":"RS256","use":"sig","kid":"{kid}"}}"#
        ),
        kid,
        path,
    })
}

/// The RFC 7638 §3 JWK thumbprint of an RSA public key: SHA-256 over
/// `{"e":..,"kty":"RSA","n":..}` — those three members only, no whitespace,
/// lexicographic order — base64url encoded without padding.
///
/// **This is a second implementation of
/// `vpay_api::op::keys::rfc7638_thumbprint`, deliberately.** xtask has no
/// dependency on any workspace crate and gains nothing from acquiring one:
/// `verify-errors`/`verify-status`/`verify-no-mocks` are CI gates that run on
/// every change, and making them link `vpay-api` would mean compiling axum,
/// sqlx and the whole authkestra stack before the workspace can check its own
/// conventions. The cost of the duplication is the risk that the two drift,
/// so both are pinned to RFC 7638's own worked example by a test with the
/// same name in both crates: a drift in either one fails there, not silently
/// at the first token a merchant cannot verify.
fn rfc7638_thumbprint(n: &str, e: &str) -> String {
    use base64::Engine as _;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use sha2::Digest as _;

    let canonical = format!(r#"{{"e":"{e}","kty":"RSA","n":"{n}"}}"#);
    URL_SAFE_NO_PAD.encode(sha2::Sha256::digest(canonical.as_bytes()))
}

/// Reads `--flag value` out of an argument list. Deliberately tiny: xtask
/// takes no dependency on an argument parser, and one flag on one command
/// does not justify the first.
fn flag_value(args: &[String], flag: &str) -> Option<String> {
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == flag {
            return iter.next().cloned();
        }
        if let Some(inline) = arg.strip_prefix(&format!("{flag}=")) {
            return Some(inline.to_string());
        }
    }
    None
}

#[cfg(test)]
mod signing_key_tests {
    use super::*;

    /// A directory under the system temp dir, unique per test, removed on
    /// drop. xtask has no `tempfile` dependency and one small guard is
    /// cheaper than acquiring one.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock is after the unix epoch")
                .as_nanos();
            let path = std::env::temp_dir()
                .join(format!("vpay-xtask-{label}-{}-{nanos}", std::process::id()));
            fs::create_dir_all(&path).expect("temp dir is creatable");
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// The cross-crate drift guard described on [`rfc7638_thumbprint`]: the
    /// identical test, with the identical vector from RFC 7638 §3.1, exists
    /// in `vpay-api`'s `op::keys`. If these two implementations ever
    /// disagree, at least one of them fails here.
    #[test]
    fn the_thumbprint_matches_rfc_7638s_own_worked_example() {
        let n = "0vx7agoebGcQSuuPiLJXZptN9nndrQmbXEps2aiAFbWhM78LhWx4\
                 cbbfAAtVT86zwu1RK7aPFFxuhDR1L6tSoc_BJECPebWKRXjBZCiF\
                 V4n3oknjhMstn64tZ_2W-5JsGY4Hc5n9yBXArwl93lqt7_RN5w6C\
                 f0h4QyQ5v-65YGjQR0_FDW2QvzqY368QQMicAtaSqzs8KJZgnYb9\
                 c7d0zgdAZHzu6qMQvRL5hajrn1n91CbOpbISD08qNLyrdkt-bFTW\
                 hAI4vMQFh6WeZu0fM4lFd2NcRwr3XPksINHaQ-G_xBniIqbw0Ls1\
                 jF44-csFCur-kEgU8awapJzKnqDKgw"
            .replace([' ', '\n'], "");

        assert_eq!(
            rfc7638_thumbprint(&n, "AQAB"),
            "NzbLsXh8uDCcd-6MNwXF4W_7noWXFZAfHkxZsRGC9Xs"
        );
    }

    /// The round trip that matters: what is written to disk must parse back
    /// as the same key, and re-deriving the thumbprint from the *file*
    /// (rather than from the in-memory key) must give the `kid` that was
    /// printed — otherwise an operator would deploy a Secret under a `kid`
    /// vpay will never compute for it.
    #[test]
    fn a_generated_key_parses_back_off_disk_with_the_same_kid() {
        use base64::Engine as _;
        use base64::engine::general_purpose::URL_SAFE_NO_PAD;
        use rsa::pkcs8::DecodePrivateKey as _;
        use rsa::traits::PublicKeyParts as _;

        let dir = TempDir::new("roundtrip");
        let generated = generate_signing_key(&dir.0).expect("key generation succeeds");

        let pem = fs::read_to_string(&generated.path).expect("the key file is readable");
        assert!(pem.starts_with("-----BEGIN PRIVATE KEY-----"), "PKCS#8 PEM");

        let reparsed = rsa::RsaPrivateKey::from_pkcs8_pem(&pem).expect("the file is a PKCS#8 key");
        assert_eq!(
            reparsed.n().bits(),
            GENERATED_KEY_BITS,
            "the generated key must be the advertised size"
        );

        let kid = rfc7638_thumbprint(
            &URL_SAFE_NO_PAD.encode(reparsed.n().to_bytes_be()),
            &URL_SAFE_NO_PAD.encode(reparsed.e().to_bytes_be()),
        );
        assert_eq!(
            kid, generated.kid,
            "the kid printed for an operator must be the kid the key itself yields"
        );
        assert!(
            generated.public_jwk.contains(&format!(r#""kid":"{kid}""#)),
            "the printed JWK carries the same kid: {}",
            generated.public_jwk
        );
        for private_member in [r#""d":"#, r#""p":"#, r#""q":"#, r#""dp":"#, r#""dq":"#] {
            assert!(
                !generated.public_jwk.contains(private_member),
                "the printed JWK must carry no private member ({private_member}): {}",
                generated.public_jwk
            );
        }
    }

    /// A signing key is unrecoverable once overwritten, so a second run must
    /// fail rather than clobber — and must leave the original file exactly
    /// as it was.
    #[test]
    fn it_refuses_to_overwrite_an_existing_key_file() {
        let dir = TempDir::new("no-clobber");
        let first = generate_signing_key(&dir.0).expect("the first generation succeeds");
        let original = fs::read_to_string(&first.path).expect("the key file is readable");

        let error = generate_signing_key(&dir.0).expect_err("a second generation must refuse");
        assert!(error.contains("refusing to overwrite"), "{error}");

        assert_eq!(
            fs::read_to_string(&first.path).expect("the key file is still readable"),
            original,
            "the existing key must be untouched by a refused generation"
        );
    }

    /// A private key must not be world- or group-readable. Unix-only because
    /// that is the only place a mode means anything; `generate_signing_key`
    /// prints a warning instead on other platforms.
    #[cfg(unix)]
    #[test]
    fn the_key_file_is_only_readable_by_its_owner() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = TempDir::new("mode");
        let generated = generate_signing_key(&dir.0).expect("key generation succeeds");

        let mode = fs::metadata(&generated.path)
            .expect("the key file exists")
            .permissions()
            .mode();
        assert_eq!(
            mode & 0o777,
            0o600,
            "expected mode 0600, found {:o}",
            mode & 0o777
        );
    }

    #[test]
    fn the_out_flag_is_read_in_both_spellings_and_required() {
        let spaced = [
            "gen-signing-key".to_string(),
            "--out".to_string(),
            "/tmp/x".to_string(),
        ];
        assert_eq!(flag_value(&spaced, "--out"), Some("/tmp/x".to_string()));

        let inline = ["gen-signing-key".to_string(), "--out=/tmp/y".to_string()];
        assert_eq!(flag_value(&inline, "--out"), Some("/tmp/y".to_string()));

        let missing = ["gen-signing-key".to_string()];
        assert_eq!(flag_value(&missing, "--out"), None);
        assert!(gen_signing_key(&missing).is_err(), "--out is required");
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

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

    /// The declaration list is read from one named section, and stops at the
    /// next heading — a token listed under a *later* section is not
    /// declared, because that section is not the one `AGENTS.md` points at.
    #[test]
    fn the_declared_list_is_the_named_section_and_nothing_after_it() {
        let status = format!(
            "# Status\n\nSome prose naming `ghost::token` in passing.\n\n\
             {STATUS_TOKEN_HEADING}\n\n\
             Every token below appears verbatim in the source.\n\n\
             - `mtn_momo::refund`\n- `orange_money::submit`\n\n\
             ### Adapters\n\n- `later::section`\n"
        );
        let declared = declared_tokens(&status).expect("the section is present");
        assert_eq!(
            declared,
            BTreeSet::from([
                "mtn_momo::refund".to_owned(),
                "orange_money::submit".to_owned()
            ]),
            "prose mentions and later sections declare nothing"
        );
    }

    /// A renamed or deleted section is a hard error, not an empty list: an
    /// empty list would fail every token at once with a message that never
    /// mentions the heading.
    #[test]
    fn a_status_page_without_the_section_is_itself_the_failure() {
        let error = declared_tokens("# Status\n\nNothing here.\n")
            .expect_err("a missing section must be reported");
        assert!(error.contains(STATUS_TOKEN_HEADING), "{error}");
    }

    /// The delegation check finds the `#[from]` variants and the methods that
    /// swallow them, and finds nothing when every arm is present.
    ///
    /// Both directions, for the reason [`the_status_check_fails_in_both_directions`]
    /// gives: a check that never fires is indistinguishable from one that
    /// cannot.
    #[test]
    fn a_from_variant_missing_a_classify_arm_is_reported() {
        let delegating = searchable(
            r#"
            pub enum ApiError {
                #[error("db")]
                Db(#[from] DbError),
                #[error("internal: {0}")]
                Internal(String),
            }
            impl Classify for ApiError {
                fn category(&self) -> Category {
                    match self {
                        Self::Db(e) => e.category(),
                        Self::Internal(_) => Category::Internal,
                    }
                }
                fn code(&self) -> &'static str {
                    match self {
                        Self::Db(e) => e.code(),
                        _ => "internal",
                    }
                }
            }
            "#,
        );
        assert_eq!(
            from_variants(&delegating, "ApiError"),
            vec!["Db".to_owned()]
        );
        assert!(undelegated_from_variants(&delegating, "ApiError").is_empty());

        let swallowing = delegating.replace("Self::Db(e) => e.code(), ", "");
        assert_eq!(
            undelegated_from_variants(&swallowing, "ApiError"),
            vec![("code".to_owned(), "Db".to_owned())],
            "a `_ =>` arm answering for a `#[from]` leaf must be reported"
        );
    }

    /// Discriminating on `self` without writing `match self` does not exempt
    /// a `Classify` method from naming every `#[from]` leaf.
    ///
    /// This is the evasion the first version of the check allowed: an `if
    /// let` ladder, a `matches!` and a `match *self` all answer differently
    /// per variant, and the trailing `else`/`false` answers for an unnamed
    /// leaf exactly as a `_ =>` arm would — but none of them contain the
    /// literal `match self`, so the method was skipped and the leaf's own
    /// `code`/`retry`/`severity` were silently replaced by the category
    /// default. Narrowing [`SELF_DISCRIMINATING_FORMS`] back to `match self`
    /// makes every assertion below fail.
    #[test]
    fn a_from_variant_swallowed_by_an_if_let_ladder_is_reported() {
        // `Db` is named; `Ledger` — also `#[from]` — is answered for by the
        // trailing `else`, by the `false` arm and by the `_ =>` respectively.
        let evading = searchable(
            r#"
            pub enum JobError {
                #[error("db")]
                Db(#[from] DbError),
                #[error("ledger")]
                Ledger(#[from] LedgerError),
                #[error("payload")]
                Payload(String),
            }
            impl Classify for JobError {
                fn category(&self) -> Category {
                    if let Self::Db(e) = self {
                        e.category()
                    } else {
                        Category::Internal
                    }
                }
                fn retry(&self) -> Retry {
                    if matches!(self, Self::Db(_)) {
                        Retry::AfterBackoff
                    } else {
                        Retry::Never
                    }
                }
                fn severity(&self) -> Severity {
                    match *self {
                        Self::Db(ref e) => e.severity(),
                        _ => Severity::Warn,
                    }
                }
            }
            "#,
        );
        assert_eq!(
            from_variants(&evading, "JobError"),
            vec!["Db".to_owned(), "Ledger".to_owned()],
            "both `#[from]` variants must be found before the delegation check runs"
        );
        assert_eq!(
            undelegated_from_variants(&evading, "JobError"),
            vec![
                ("category".to_owned(), "Ledger".to_owned()),
                ("retry".to_owned(), "Ledger".to_owned()),
                ("severity".to_owned(), "Ledger".to_owned()),
            ],
            "an `if let` / `matches!` / `match *self` ladder that omits a \
             `#[from]` leaf must be reported, not skipped"
        );
    }

    /// A `Classify` method that answers the same thing for every variant has
    /// no wildcard to hide in, so it is not required to enumerate them.
    ///
    /// `vpay_provider::RailFailure` is the real instance: both of its
    /// `#[from]` variants are `Category::Rail` and its `category` is one
    /// line with no `match`.
    #[test]
    fn a_classify_method_without_a_match_is_not_required_to_enumerate_variants() {
        let text = searchable(
            r#"
            pub enum RailFailure {
                #[error("sending the request")]
                Http(#[from] reqwest::Error),
                #[error("reading the response")]
                Body(#[from] http::HttpBodyError),
            }
            impl vpay_core::Classify for RailFailure {
                fn category(&self) -> vpay_core::Category {
                    vpay_core::Category::Rail
                }
            }
            "#,
        );
        assert_eq!(from_variants(&text, "RailFailure"), vec!["Http", "Body"]);
        assert!(undelegated_from_variants(&text, "RailFailure").is_empty());
    }

    /// Both directions of the check, over synthetic sources — the code→docs
    /// half that always worked, and the docs→code half that did not.
    ///
    /// Driven through the same two functions `verify_status` composes,
    /// because a test that reimplemented the comparison would pass whatever
    /// the check does.
    #[test]
    fn the_status_check_fails_in_both_directions() {
        let status = format!("{STATUS_TOKEN_HEADING}\n\n- `built::already`\n");
        let declared = declared_tokens(&status).expect("the section is present");

        // Direction 1: the code carries a token the page does not declare.
        let found = scan_not_implemented(&searchable(
            "fn f() { Err(ProviderError::NotImplemented(\"new::gap\")) }",
        ));
        assert!(
            found.iter().any(|token| !declared.contains(token)),
            "an undeclared token in shipping code must be visible to the check"
        );

        // Direction 2: the page declares a token no code carries — what
        // happens every time something is built and the bullet is forgotten.
        let found: BTreeSet<String> = found.into_iter().collect();
        assert!(
            declared.iter().any(|token| !found.contains(token)),
            "a declared token that no shipping code carries must be visible to the check"
        );
    }

    /// A token inside a `#[cfg(test)]` module is a fixture and declares
    /// nothing — `vpay-worker`'s error tests build one to assert how it
    /// classifies. Counting it would force `docs/status.md` to advertise a
    /// gap that does not exist.
    #[test]
    fn a_token_in_test_code_is_not_a_shipping_claim() {
        let text = "#[cfg(test)]\nmod tests {\n    \
                    const E: E = ProviderError::NotImplemented(\"mtn_momo::submit\");\n}\n";
        assert!(
            scan_not_implemented(&searchable(text)).is_empty(),
            "a token declared under #[cfg(test)] is a fixture"
        );
        // And the same token outside the module still counts.
        let shipping = format!("fn f() {{ Err(ProviderError::NotImplemented(\"a::b\")) }}\n{text}");
        assert_eq!(
            scan_not_implemented(&searchable(&shipping)),
            vec!["a::b"],
            "only the shipping occurrence is a claim"
        );
    }

    /// A doc comment quoting a token declares nothing either — the same
    /// reason `verify-errors` strips comments before looking for `impl
    /// Classify`.
    #[test]
    fn a_token_quoted_in_a_comment_is_not_a_shipping_claim() {
        let text = "/// Returns `ProviderError::NotImplemented(\"doc::only\")` one day.\n\
                    /* NotImplemented(\"block::comment\") */\nfn f() {}\n";
        assert!(scan_not_implemented(&searchable(text)).is_empty());
    }

    /// A synthetic `cargo metadata` document: two shipping binaries, a
    /// library each links, and a test-only crate reachable through whichever
    /// edge kind the caller asks for.
    ///
    /// Synthetic rather than this workspace's own metadata on purpose — the
    /// graphs that matter are the ones this repository must never grow, and a
    /// test that could only observe the current graph would pass forever
    /// without proving the walk works.
    fn synthetic_metadata(lib_to_wiremock_kind: &str, app_to_testkit_kind: &str) -> Value {
        let kind = |k: &str| {
            if k == "normal" { json!(null) } else { json!(k) }
        };
        json!({
            "workspace_members": ["app 0.1.0 (path+file:///app)", "lib 0.1.0 (path+file:///lib)"],
            "packages": [
                { "id": "app 0.1.0 (path+file:///app)", "name": "vpay-server", "dependencies": [
                    { "name": "lib", "kind": null },
                    { "name": "vpay-testkit", "kind": kind(app_to_testkit_kind) }
                ]},
                { "id": "lib 0.1.0 (path+file:///lib)", "name": "lib", "dependencies": [
                    { "name": "wiremock", "kind": kind(lib_to_wiremock_kind) }
                ]},
                { "id": "tk 0.1.0 (path+file:///tk)", "name": "vpay-testkit", "dependencies": [] },
                { "id": "wm 0.1.0 (registry+wiremock)", "name": "wiremock", "dependencies": [] }
            ],
            "resolve": { "nodes": [
                { "id": "app 0.1.0 (path+file:///app)", "deps": [
                    { "pkg": "lib 0.1.0 (path+file:///lib)", "dep_kinds": [{ "kind": null }] },
                    { "pkg": "tk 0.1.0 (path+file:///tk)",
                      "dep_kinds": [{ "kind": kind(app_to_testkit_kind) }] }
                ]},
                { "id": "lib 0.1.0 (path+file:///lib)", "deps": [
                    { "pkg": "wm 0.1.0 (registry+wiremock)",
                      "dep_kinds": [{ "kind": kind(lib_to_wiremock_kind) }] }
                ]},
                { "id": "tk 0.1.0 (path+file:///tk)", "deps": [] },
                { "id": "wm 0.1.0 (registry+wiremock)", "deps": [] }
            ]}
        })
    }

    /// The case the manifest scan alone could never see: the double is two
    /// hops away, and neither app manifest mentions it.
    #[test]
    fn a_double_reachable_through_a_library_is_a_violation() {
        let metadata = synthetic_metadata("normal", "dev");
        let problems = test_only_reachable_from(&metadata, &["vpay-server"]);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(
            problems
                .first()
                .is_some_and(|p| p.contains("`wiremock` is reachable from `vpay-server`")),
            "{problems:?}"
        );
    }

    /// And the permission ADR-0006 actually grants: the same crates, through
    /// dev edges, are fine — that is how every test in this workspace uses
    /// them.
    #[test]
    fn the_same_crates_behind_dev_edges_are_not_a_violation() {
        let metadata = synthetic_metadata("dev", "dev");
        assert!(
            test_only_reachable_from(&metadata, &["vpay-server"]).is_empty(),
            "a dev-dependency is not linked into the binary"
        );
    }

    /// A build-dependency runs at compile time and ships in nothing — the
    /// same line the manifest scan draws by ignoring `[build-dependencies]`.
    #[test]
    fn a_build_dependency_is_not_a_shipping_edge() {
        let metadata = synthetic_metadata("build", "dev");
        assert!(test_only_reachable_from(&metadata, &["vpay-server"]).is_empty());
    }

    /// A direct non-dev edge onto the testkit is caught too, and the whole
    /// subtree behind it with it.
    #[test]
    fn a_direct_runtime_edge_onto_the_testkit_is_a_violation() {
        let metadata = synthetic_metadata("dev", "normal");
        let problems = test_only_reachable_from(&metadata, &["vpay-server"]);
        assert!(
            problems
                .iter()
                .any(|p| p.contains("`vpay-testkit` is reachable")),
            "{problems:?}"
        );
    }

    /// A renamed or deleted binary must fail loudly. A check that silently
    /// stops checking is worse than no check, because the green tick is still
    /// there.
    #[test]
    fn a_shipping_binary_missing_from_the_graph_is_itself_a_failure() {
        let metadata = synthetic_metadata("normal", "dev");
        let problems = test_only_reachable_from(&metadata, &["vpay-renamed-server"]);
        assert!(
            problems
                .iter()
                .any(|p| p.contains("not in the resolve graph")),
            "{problems:?}"
        );
    }

    /// The half the reachability walk cannot see, and the exact shape of the
    /// defect that motivated this: `wiremock` as a *runtime* dependency of a
    /// crate the binaries only reach through dev edges.
    #[test]
    fn a_test_only_crate_under_dependencies_is_a_violation_even_when_unreachable() {
        let metadata = json!({
            "workspace_members": ["tk 0.1.0 (path+file:///tk)"],
            "packages": [
                { "id": "tk 0.1.0 (path+file:///tk)", "name": "vpay-testkit", "dependencies": [
                    { "name": "wiremock", "kind": null },
                    { "name": "testcontainers", "kind": null }
                ]}
            ],
            "resolve": { "nodes": [] }
        });

        let problems = test_only_declared_by_a_workspace_member(&metadata);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(
            problems
                .first()
                .is_some_and(|p| p.contains("`vpay-testkit` lists test-only crate `wiremock`")),
            "{problems:?}"
        );
        // `testcontainers` is allowlisted for this member: starting a real
        // container is what the testkit is *for*, and a container is not a
        // double (ADR-0006).
    }

    /// The same crate under `[dev-dependencies]` is the permitted shape, in
    /// every member.
    #[test]
    fn a_test_only_crate_under_dev_dependencies_is_permitted() {
        let metadata = json!({
            "workspace_members": ["x 0.1.0 (path+file:///x)"],
            "packages": [
                { "id": "x 0.1.0 (path+file:///x)", "name": "vpay-api", "dependencies": [
                    { "name": "wiremock", "kind": "dev" },
                    { "name": "testcontainers", "kind": "dev" }
                ]}
            ],
            "resolve": { "nodes": [] }
        });
        assert!(test_only_declared_by_a_workspace_member(&metadata).is_empty());
    }

    /// The allowlist is a pair, not a crate: `testcontainers` is permitted at
    /// runtime *in the testkit*, and nowhere else.
    #[test]
    fn the_runtime_allowlist_is_scoped_to_the_member_it_names() {
        let metadata = json!({
            "workspace_members": ["x 0.1.0 (path+file:///x)"],
            "packages": [
                { "id": "x 0.1.0 (path+file:///x)", "name": "vpay-db", "dependencies": [
                    { "name": "testcontainers", "kind": null }
                ]}
            ],
            "resolve": { "nodes": [] }
        });
        let problems = test_only_declared_by_a_workspace_member(&metadata);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(
            problems
                .first()
                .is_some_and(|p| p.contains("`vpay-db` lists test-only crate `testcontainers`")),
            "{problems:?}"
        );
    }

    /// A metadata document with no resolve graph means the walk did not run.
    /// It must not read as "nothing found".
    #[test]
    fn a_document_with_no_resolve_graph_is_a_failure_not_a_pass() {
        let problems = test_only_reachable_from(&json!({ "packages": [] }), &["vpay-server"]);
        assert!(
            !problems.is_empty(),
            "a check that cannot run must not pass"
        );
    }

    #[test]
    fn a_runtime_testkit_dependency_is_visible_to_the_check() {
        let manifest = "[dependencies]\nvpay-testkit = { path = \"x\" }\n";
        let runtime = runtime_dependency_section(manifest);
        assert!(runtime.contains("vpay-testkit"));
    }

    /// A shipping binary that opens its pool with `connect_lazy` is a
    /// violation; the same call under `#[cfg(test)]`, or merely named in a
    /// comment, is not.
    ///
    /// Driven over a synthetic file because the real scan reads
    /// `backends/apps`, and the only honest way to watch this fire is to
    /// hand it a file that breaks the rule rather than to break a binary.
    /// The `#[cfg(test)]` and comment halves are what keep the guard from
    /// being reverted the first time it cries wolf at `vpay-server`'s own
    /// CLI tests.
    #[test]
    fn a_binary_that_opens_its_pool_lazily_is_a_violation() {
        let shipping = "\
async fn run() -> anyhow::Result<()> {
    let repositories = vpay_db::connect_lazy(&args.database_url, TIMEOUT)?;
    Ok(())
}
";
        let problems = app_source_violations(shipping);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(
            problems.first().is_some_and(|p| p.contains("connect_lazy")),
            "{problems:?}"
        );

        let tested_and_documented = "\
/// The pool is opened eagerly, never with `connect_lazy`.
async fn run() -> anyhow::Result<()> {
    let repositories = vpay_db::connect(&args.database_url).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn an_unreachable_database_is_a_503() {
        let repositories = vpay_db::connect_lazy(\"postgres://x:1/y\", TIMEOUT)?;
    }
}
";
        assert!(
            app_source_violations(tested_and_documented).is_empty(),
            "a comment naming the function, and a call under `#[cfg(test)]`, are not calls \
             from a shipping path"
        );
    }

    /// The stub-adapter half of the same scan still matches raw text —
    /// including a comment, because there is no such code path to describe.
    #[test]
    fn a_stub_adapter_named_anywhere_in_a_binary_is_a_violation() {
        let problems = app_source_violations("// we could wire a MockAdapter here for local dev\n");
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(
            problems.first().is_some_and(|p| p.contains("MockAdapter")),
            "{problems:?}"
        );
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
