//! Repository automation. Run via `cargo xtask <cmd>` or `just`.
//!
//! Five of these commands are the gates `just verify` runs, because a promise
//! nothing checks is a promise that decays:
//!
//! * `verify-no-mocks`  — no test double is reachable from a shipping binary.
//! * `verify-status`    — every `NotImplemented` is declared in `docs/status.md`.
//! * `verify-errors`    — every error type classifies itself, and `anyhow`
//!   stays at the process edge (`docs/adr/0011-error-modelling.md`).
//! * `verify-sdk-parity` — every ✅ in `docs/sdks/parity.md` names a test that
//!   exists, and every ⛔ carries a date (`docs/adr/0015-sdk-parity.md`).
//! * `verify-links`     — every relative link in a tracked `*.md` resolves to
//!   a tracked path. New 2026-09-05; before it, `just docs-check` printed
//!   "link checking is not implemented yet" and exited 0.
//!
//! A sixth is a gate that needs the network, so it is opt-in
//! (`just docs-check-citations`) and is **not** part of `just ci`:
//!
//! * `verify-citations` — every workflow-run id, pull request and issue a
//!   document cites as evidence exists. It fails rather than skips when `gh`
//!   is missing or unauthenticated: a check that downgrades itself reports
//!   success for a run in which nothing was checked.
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
//! The `verify-*` commands other than `verify-no-mocks` take no crate
//! dependencies at all and match on text rather than on types — see
//! [`has_classify_impl`] for what that costs. That is still true of them, and
//! of the two added on 2026-09-05, which reach for a *subprocess* rather than
//! a crate: `verify-links` asks `git ls-files` what the repository actually
//! tracks, because a directory walk would let an untracked scratch file
//! satisfy a link, and `verify-citations` asks `gh` — the one command here
//! allowed to need the network, and the reason it is not in `just ci`.
//!
//! `gen-signing-key` is what put four crates
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
        "verify-sdk-parity" => verify_sdk_parity(&root),
        "verify-links" => verify_links(&root),
        "verify-citations" => verify_citations(&root),
        // `verify-citations` is deliberately absent from `verify-all`: it
        // needs the network, and `verify-all` is what an offline gate list
        // runs.
        "verify-all" => verify_no_mocks(&root)
            .and_then(|()| verify_status(&root))
            .and_then(|()| verify_errors(&root))
            .and_then(|()| verify_sdk_parity(&root))
            .and_then(|()| verify_links(&root)),
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
                "usage: cargo xtask \
                 <verify-no-mocks|verify-status|verify-errors|verify-sdk-parity|verify-links\
                 |verify-all>\n\
                 \x20      cargo xtask verify-citations   (a gate; needs `gh` and the network)\n\
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

/// The call this gate counts. Written once so the scanner and its tests
/// cannot drift apart on the spelling.
const NOT_IMPLEMENTED_CALL: &str = "NotImplemented(";

/// Extract every `NotImplemented("token")` argument that shipping *code*
/// makes — never one a comment, a doc attribute or a string literal merely
/// spells out.
///
/// Whitespace-tolerant, because rustfmt may put the string literal on its own
/// line. Ignores the enum declaration itself, which has no string literal.
///
/// # Why this lexes instead of matching text
///
/// This used to be `text.match_indices("NotImplemented(")` over a caller that
/// had stripped comments line-first. Two shapes got through, both of them
/// prose: a *trailing* `// … NotImplemented("x") …` comment (the stripper
/// only dropped lines that *began* with `//`) and any raw string literal
/// carrying the token. Either one forced a phantom bullet into
/// `docs/status.md` — and because the check runs in both directions, that
/// bullet then had to stay, so the docs→code half could be satisfied by
/// nothing but prose. The louder cost is the one AGENTS.md cares about: the
/// cheapest way to make a false positive go away is to delete the honest
/// sentence from the adapter's doc comment that explained the gap.
///
/// The lexer skips over whole literals rather than blanking them, because the
/// token this extracts *is* a string literal — the one immediately after the
/// call's open paren. A raw-string argument is accepted too; nothing in the
/// tree writes one, but reading it costs a line and refusing it would be a
/// silent miss of exactly the kind this function exists to prevent.
fn scan_not_implemented(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let needle: Vec<char> = NOT_IMPLEMENTED_CALL.chars().collect();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < chars.len() {
        if let Some(end) = end_of_literal(&chars, i) {
            i = end;
            continue;
        }
        if chars
            .get(i..)
            .is_none_or(|rest| !rest.starts_with(needle.as_slice()))
        {
            i += 1;
            continue;
        }
        let mut pos = i + needle.len();
        while chars.get(pos).is_some_and(|c| c.is_whitespace()) {
            pos += 1;
        }
        // `NotImplemented(_)` in a match arm and the enum declaration itself
        // reach this point with no literal to read, and name nothing.
        if let Some(token) = literal_content(&chars, pos) {
            out.push(token);
        }
        i += needle.len();
    }
    out
}

/// The text between the delimiters of the literal at `i`, if one starts
/// there. Escapes are returned as written: a token is an identifier path, so
/// one containing an escape is a bug worth seeing rather than decoding.
fn literal_content(chars: &[char], i: usize) -> Option<String> {
    let end = end_of_literal(chars, i)?;
    let open = chars.get(i..end)?;
    let quote = open.iter().position(|c| *c == '"')?;
    let hashes = open
        .get(..quote)
        .unwrap_or_default()
        .iter()
        .filter(|c| **c == '#')
        .count();
    let inner = open.get(quote + 1..end.checked_sub(i + 1 + hashes)?)?;
    Some(inner.iter().collect())
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
/// Until 2026-09-05 only a *leading* `//` was stripped: comments were removed
/// by dropping whole lines, so a trailing one survived. That was a deliberate
/// trade — a line-based stripper cannot tell `// a comment` from the `//` in
/// `"https://…"`, and swallowing the rest of that line would have deleted
/// live code and made the check pass by finding nothing. [`strip_comments`]
/// removes the trade rather than the compromise: it lexes, so a `//` inside a
/// string literal is not a comment and every comment goes, wherever it sits.
fn searchable(text: &str) -> String {
    let without_comments = strip_comments(text);
    let without_tests = strip_cfg_test_items(&without_comments);
    let mut out = String::with_capacity(without_tests.len());
    let mut in_whitespace = false;
    for ch in without_tests.chars() {
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
    out
}

/// Replace every comment — `//`, `///`, `//!`, `/* */`, `/** */`, nested or
/// not — with a space, leaving line breaks and every literal intact.
///
/// This is the lexer the three text-matching gates share, and the reason it
/// is a lexer rather than a pair of `contains` calls is that all four of the
/// non-code places a token can be written are indistinguishable from code by
/// text alone. A `//` inside a string literal opens no comment; a `"` inside
/// a comment opens no string; `*/` inside `r#"…"#` closes nothing; `'"'` is a
/// character, not a quote. Getting any one of those wrong fails in one of two
/// directions, and only one of them is loud: a stripper that deletes too much
/// makes a gate pass by finding nothing.
///
/// Literals are left *verbatim* rather than blanked, because the token
/// `verify-status` extracts lives inside one: emptying string literals would
/// turn `NotImplemented("mtn_momo::refund")` into `NotImplemented("")`. It is
/// [`scan_not_implemented`] that refuses to look *inside* a literal, by
/// lexing over the same [`end_of_literal`] this does.
fn strip_comments(text: &str) -> String {
    strip_comment_kinds(text, CommentKinds::All)
}

/// Replace only `/* … */` comments with a space, leaving `//` lines — and so
/// `///` and `//!` — exactly where they were.
///
/// `verify-docs` is the one caller that wants this: it *counts* doc lines, so
/// a stripper that removed them would report every file as having none.
fn strip_block_comments(text: &str) -> String {
    strip_comment_kinds(text, CommentKinds::BlocksOnly)
}

/// Which comments [`strip_comment_kinds`] removes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum CommentKinds {
    /// Every comment. What the three gates want: prose satisfies nothing.
    All,
    /// Block comments only, `//` lines left verbatim. What `verify-docs`
    /// wants, because a `///` line is the thing it is counting.
    BlocksOnly,
}

/// The shared lexer. See [`strip_comments`] for why this is a lexer.
fn strip_comment_kinds(text: &str, kinds: CommentKinds) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0usize;
    while let Some(&c) = chars.get(i) {
        if c == '/' && chars.get(i + 1) == Some(&'*') {
            let mut depth = 1usize;
            i += 2;
            while depth > 0 {
                let Some(&c) = chars.get(i) else { break };
                if c == '/' && chars.get(i + 1) == Some(&'*') {
                    depth += 1;
                    i += 2;
                } else if c == '*' && chars.get(i + 1) == Some(&'/') {
                    depth -= 1;
                    i += 2;
                } else {
                    // Line breaks survive so the caller still sees the same
                    // lines; `strip_cfg_test_items` counts braces per item,
                    // not per line, but a human reading a failure does.
                    if c == '\n' {
                        out.push('\n');
                    }
                    i += 1;
                }
            }
            out.push(' ');
            continue;
        }
        if c == '/' && chars.get(i + 1) == Some(&'/') {
            let start = i;
            while chars.get(i).is_some_and(|c| *c != '\n') {
                i += 1;
            }
            match kinds {
                // A line comment kept verbatim must still not be re-lexed:
                // `// /*` opens nothing.
                CommentKinds::BlocksOnly => out.extend(chars.get(start..i).unwrap_or_default()),
                CommentKinds::All => out.push(' '),
            }
            continue; // the newline itself is pushed by the next iteration
        }
        if let Some(end) = end_of_literal(&chars, i) {
            out.extend(chars.get(i..end).unwrap_or_default());
            i = end;
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

/// If a string, raw-string, byte-string, C-string or character literal starts
/// at `i`, the index just past its closing delimiter.
///
/// Returns `None` for anything else, including a *lifetime* — `&'a str` and
/// `'static` open no literal, and treating them as one would swallow every
/// character up to the next `'` in the file. That is the same
/// delete-too-much failure the module docs warn about, which is why the
/// character-literal arm demands a closing quote two or three positions on
/// rather than assuming one.
///
/// Prefixes (`b`, `c`, `r`, `br`, `cr`) only count when the character before
/// them is not an identifier character, so the `r` at the end of `four` opens
/// nothing.
fn end_of_literal(chars: &[char], i: usize) -> Option<usize> {
    let c = *chars.get(i)?;
    if c == '"' {
        return Some(end_of_quoted(chars, i + 1));
    }
    if c == '\'' {
        return end_of_char_literal(chars, i);
    }
    if !matches!(c, 'b' | 'c' | 'r') {
        return None;
    }
    if i.checked_sub(1)
        .and_then(|before| chars.get(before))
        .is_some_and(|c| c.is_alphanumeric() || *c == '_')
    {
        return None;
    }
    let mut pos = i;
    let mut raw = false;
    if matches!(chars.get(pos), Some('b' | 'c')) {
        pos += 1;
    }
    if chars.get(pos) == Some(&'r') {
        raw = true;
        pos += 1;
    }
    if raw {
        let hashes_start = pos;
        while chars.get(pos) == Some(&'#') {
            pos += 1;
        }
        let hashes = pos - hashes_start;
        if chars.get(pos) != Some(&'"') {
            return None;
        }
        return Some(end_of_raw(chars, pos + 1, hashes));
    }
    // `b'x'` and `c'x'` are not real Rust, but `b'x'` is; either way the
    // character-literal arm below is the one that answers for them.
    if chars.get(pos) == Some(&'\'') {
        return end_of_char_literal(chars, pos);
    }
    if chars.get(pos) != Some(&'"') {
        return None;
    }
    Some(end_of_quoted(chars, pos + 1))
}

/// The index just past the `"` that closes a non-raw string opened at `from`,
/// honouring backslash escapes. An unterminated literal ends at end of input
/// rather than panicking: a file that does not compile must not crash a gate.
fn end_of_quoted(chars: &[char], from: usize) -> usize {
    let mut pos = from;
    while let Some(&c) = chars.get(pos) {
        pos += 1;
        if c == '\\' {
            pos += 1;
        } else if c == '"' {
            return pos;
        }
    }
    chars.len()
}

/// The index just past the `"#…#` that closes a raw string opened at `from`
/// with `hashes` hashes. No escapes exist inside one, which is precisely why
/// `r#"…*/…"#` is not a comment and `r"…\"` ends at the quote.
fn end_of_raw(chars: &[char], from: usize, hashes: usize) -> usize {
    let mut pos = from;
    while let Some(&c) = chars.get(pos) {
        if c == '"' && (1..=hashes).all(|n| chars.get(pos + n) == Some(&'#')) {
            return pos + 1 + hashes;
        }
        pos += 1;
    }
    chars.len()
}

/// The index just past a character literal opening at `i`, or `None` if `i`
/// opens a lifetime instead.
///
/// The two are told apart the way rustc's lexer does it: `'\` is always an
/// escape and therefore a literal, and otherwise a literal is exactly the
/// case where the quote closes two positions on (`'a'`). Everything else —
/// `'a`, `'static` — is a lifetime and stays code.
fn end_of_char_literal(chars: &[char], i: usize) -> Option<usize> {
    if chars.get(i + 1) == Some(&'\\') {
        let mut pos = i + 2;
        // One escaped character, then anything up to the closing quote:
        // `'\''`, `'\\'` and `'\u{27}'` all end at the next `'` after it.
        pos += 1;
        while let Some(&c) = chars.get(pos) {
            pos += 1;
            if c == '\'' {
                return Some(pos);
            }
        }
        return Some(chars.len());
    }
    if chars.get(i + 1).is_some() && chars.get(i + 2) == Some(&'\'') {
        return Some(i + 3);
    }
    None
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
// verify-sdk-parity
// ---------------------------------------------------------------------------

/// The matrix this check reads.
///
/// [ADR-0015](../../docs/adr/0015-sdk-parity.md): the two merchant SDKs must
/// offer the same capabilities with the same wire semantics, the matrix is
/// the record of where they do, and this function is the enforcement. Modelled
/// on [`verify_status`] — a machine check over a markdown table, so the
/// document a human reads is the document the build checks.
const SDK_PARITY_DOC: &str = "docs/sdks/parity.md";

/// The first header cell that marks a table as one of the parity matrices.
///
/// A marker rather than "every table in the file", for the same reason
/// [`STATUS_TOKEN_HEADING`] names a section: the document also carries a gap
/// ledger and a legend, and a check that tried to read those as matrices
/// would either fail on prose or force the prose out of the document.
const PARITY_TABLE_MARKER: &str = "Capability";

/// Directories that hold no first-party source and would only slow the walk
/// down — or, worse, contribute a test name from a vendored dependency and
/// let a ✅ cell be satisfied by somebody else's test.
const PARITY_SKIPPED_DIRS: [&str; 5] = ["node_modules", "dist", "target", ".git", "coverage"];

/// Extensions [`test_names_in`] knows how to read.
const PARITY_TS_EXTENSIONS: [&str; 5] = ["ts", "tsx", "mts", "mjs", "js"];

/// One parity table: the SDK trees its columns compare, and its rows.
struct ParityTable {
    /// Repo-relative SDK roots, one per column after `Capability`.
    columns: Vec<String>,
    rows: Vec<ParityRow>,
    /// 1-based line number of the header row, for failure messages.
    line: usize,
}

/// One capability row: what is being compared, and one cell per column.
struct ParityRow {
    capability: String,
    cells: Vec<String>,
    /// 1-based line number, so a failure can be pasted into an editor.
    line: usize,
}

/// Fail if the parity matrix claims something the SDK trees do not carry.
///
/// Three rules, one per way the matrix could start lying:
///
/// * a `✅` cell names the test(s) that prove the capability **in that SDK**,
///   and every one of them must exist there — a Rust `#[test]`/`#[tokio::test]`
///   function or a TypeScript `it("…")`/`test("…")` with that exact name.
///   Renaming a test without updating the matrix is the ordinary way a
///   proof-of-parity claim rots, and it fails here instead.
/// * a `⛔` cell must carry a date. ADR-0015 allows a capability to be
///   missing from one SDK; it does not allow the absence to be undated,
///   because an undated gap is indistinguishable from one nobody has looked
///   at since it was written.
/// * no cell may be blank. A blank cell is the only answer that says nothing,
///   and it is what an unfinished row looks like.
fn verify_sdk_parity(root: &Path) -> Result<(), String> {
    let path = root.join(SDK_PARITY_DOC);
    let doc = fs::read_to_string(&path).map_err(|e| {
        format!("{SDK_PARITY_DOC}: {e} (the parity matrix is mandatory — see docs/adr/0015-sdk-parity.md)")
    })?;

    let outcome = parity_outcome(root, &doc);
    if !outcome.problems.is_empty() {
        return Err(format!(
            "sdk parity violations:\n  - {}",
            outcome.problems.join("\n  - ")
        ));
    }

    println!(
        "verify-sdk-parity: ok — {} proving test(s) named in {SDK_PARITY_DOC} all exist, {} dated gap(s)",
        outcome.proven, outcome.gaps
    );
    Ok(())
}

/// What [`parity_outcome`] found: everything wrong, and the two counts the
/// success line reports.
struct ParityOutcome {
    problems: Vec<String>,
    proven: usize,
    gaps: usize,
}

/// Checks every cell of every parity table in `doc` against the SDK trees
/// under `root`.
///
/// Takes the document text rather than reading it, so the rules can be proven
/// against synthetic matrices — including matrices this repository does not
/// have and must never grow (a `✅` naming a test that was renamed away, a
/// blank cell, an undated `⛔`).
fn parity_outcome(root: &Path, doc: &str) -> ParityOutcome {
    let mut problems = Vec::new();
    let mut proven = 0usize;
    let mut gaps = 0usize;

    let tables = parity_tables(doc);
    if tables.is_empty() {
        problems.push(format!(
            "{SDK_PARITY_DOC} carries no table whose first column is `{PARITY_TABLE_MARKER}`; \
             the matrix is the record and this check reads it, so a document without one \
             would pass by checking nothing"
        ));
        return ParityOutcome {
            problems,
            proven,
            gaps,
        };
    }

    for table in &tables {
        let mut indexes = Vec::new();
        for column in &table.columns {
            let dir = root.join(column);
            if !dir.is_dir() {
                problems.push(format!(
                    "{SDK_PARITY_DOC}:{}: column `{column}` is not a directory in this repository",
                    table.line
                ));
            }
            indexes.push(test_names_in(&dir));
        }

        for row in &table.rows {
            if row.cells.len() != table.columns.len() {
                problems.push(format!(
                    "{SDK_PARITY_DOC}:{}: row `{}` has {} cell(s), the table has {} column(s)",
                    row.line,
                    row.capability,
                    row.cells.len(),
                    table.columns.len()
                ));
                continue;
            }
            for ((cell, column), index) in row.cells.iter().zip(&table.columns).zip(&indexes) {
                check_parity_cell(
                    cell,
                    column,
                    index,
                    row,
                    &mut problems,
                    &mut proven,
                    &mut gaps,
                );
            }
        }
    }

    ParityOutcome {
        problems,
        proven,
        gaps,
    }
}

/// The three rules on [`verify_sdk_parity`], applied to one cell.
fn check_parity_cell(
    cell: &str,
    column: &str,
    index: &BTreeSet<String>,
    row: &ParityRow,
    problems: &mut Vec<String>,
    proven: &mut usize,
    gaps: &mut usize,
) {
    let text = cell.trim();
    let at = format!(
        "{SDK_PARITY_DOC}:{} `{}` / {column}",
        row.line, row.capability
    );

    if text.is_empty() {
        problems.push(format!(
            "{at}: the cell is blank — every capability is answered ✅ (naming the test(s) \
             that prove it) or ⛔ (with a dated gap line)"
        ));
        return;
    }

    if text.starts_with('✅') {
        let names = code_spans(text);
        if names.is_empty() {
            problems.push(format!(
                "{at}: ✅ names no test — a ✅ cell lists the test(s) that prove it, each in \
                 backticks"
            ));
            return;
        }
        for name in names {
            if index.contains(&name) {
                *proven += 1;
            } else {
                problems.push(format!(
                    "{at}: names the test `{name}`, which does not exist under `{column}` \
                     (looked for a Rust `#[test]`/`#[tokio::test]` fn or a TypeScript \
                     `it(\"…\")`/`test(\"…\")` with exactly that name, ignoring anything \
                     `#[ignore]`d)"
                ));
            }
        }
        return;
    }

    if text.starts_with('⛔') {
        if contains_iso_date(text) {
            *gaps += 1;
        } else {
            problems.push(format!(
                "{at}: ⛔ with no date — a gap is recorded with the date it was found \
                 (YYYY-MM-DD), the reason, and who owns closing it"
            ));
        }
        return;
    }

    problems.push(format!(
        "{at}: the cell must begin with ✅ or ⛔, and begins `{}`",
        text.chars().take(16).collect::<String>()
    ));
}

/// Every table in `doc` whose first header cell is [`PARITY_TABLE_MARKER`].
///
/// The remaining header cells are the SDK roots the table compares, written
/// as code spans (`` `sdks/rust` ``) so the document reads as paths and the
/// check can take them literally.
fn parity_tables(doc: &str) -> Vec<ParityTable> {
    let lines: Vec<&str> = doc.lines().collect();
    let mut out = Vec::new();
    let mut i = 0usize;

    while i < lines.len() {
        let Some(header) = lines.get(i).and_then(|l| table_row(l)) else {
            i += 1;
            continue;
        };
        if header.first().map(String::as_str) != Some(PARITY_TABLE_MARKER) || header.len() < 2 {
            i += 1;
            continue;
        }
        let separator = lines.get(i + 1).and_then(|l| table_row(l));
        let Some(separator) = separator else {
            i += 1;
            continue;
        };
        if separator.len() != header.len() || !separator.iter().all(|c| is_separator_cell(c)) {
            i += 1;
            continue;
        }

        let columns: Vec<String> = header
            .iter()
            .skip(1)
            .map(|cell| {
                code_spans(cell)
                    .into_iter()
                    .next()
                    .unwrap_or_else(|| cell.clone())
            })
            .collect();

        let header_line = i + 1;
        let mut rows = Vec::new();
        let mut j = i + 2;
        while let Some(cells) = lines.get(j).and_then(|l| table_row(l)) {
            let capability = cells.first().cloned().unwrap_or_default();
            rows.push(ParityRow {
                capability,
                cells: cells.into_iter().skip(1).collect(),
                line: j + 1,
            });
            j += 1;
        }

        out.push(ParityTable {
            columns,
            rows,
            line: header_line,
        });
        i = j;
    }

    out
}

/// The cells of a markdown table row, or `None` if the line is not one.
///
/// A cell may not contain a `|`; the matrix has no need for one and
/// supporting `\|` would mean a second escaping rule that only this check
/// understands.
fn table_row(line: &str) -> Option<Vec<String>> {
    let trimmed = line.trim();
    if !trimmed.starts_with('|') || !trimmed.ends_with('|') || trimmed.len() < 2 {
        return None;
    }
    let inner = trimmed.strip_prefix('|')?.strip_suffix('|')?;
    Some(
        inner
            .split('|')
            .map(|cell| cell.trim().to_owned())
            .collect(),
    )
}

/// `---`, `:--`, `--:` and friends: the row that separates a header from its
/// body and marks the lines above and below as one table.
fn is_separator_cell(cell: &str) -> bool {
    !cell.is_empty() && cell.chars().all(|c| c == '-' || c == ':')
}

/// Every code span in `text`, honouring backtick runs.
///
/// A run of *n* backticks opens a span that the next run of exactly *n*
/// closes, which is how a test name that itself contains a backtick —
/// `` ``authenticates a real `stripe` client end to end`` `` — is written in
/// a cell at all. One leading and one trailing space are stripped when both
/// are present, as CommonMark does, so the delimiter can be held off the
/// content.
fn code_spans(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0usize;

    while i < chars.len() {
        if chars.get(i) != Some(&'`') {
            i += 1;
            continue;
        }
        let open_start = i;
        while chars.get(i) == Some(&'`') {
            i += 1;
        }
        let run = i - open_start;
        let content_start = i;

        let mut j = i;
        let mut close = None;
        while j < chars.len() {
            if chars.get(j) == Some(&'`') {
                let start = j;
                while chars.get(j) == Some(&'`') {
                    j += 1;
                }
                if j - start == run {
                    close = Some((start, j));
                    break;
                }
            } else {
                j += 1;
            }
        }

        let Some((close_start, close_end)) = close else {
            break; // An unclosed run is not a span; nothing after it can be one either.
        };
        let content: String = chars
            .get(content_start..close_start)
            .unwrap_or_default()
            .iter()
            .collect();
        out.push(strip_one_padding_space(content));
        i = close_end;
    }

    out
}

/// CommonMark's code-span rule: one leading and one trailing space are part
/// of the delimiter, not of the content, when both are present and the
/// content is not all spaces.
fn strip_one_padding_space(content: String) -> String {
    if content.starts_with(' ') && content.ends_with(' ') && !content.trim().is_empty() {
        let end = content.len().saturating_sub(1);
        return content.get(1..end).unwrap_or(&content).to_owned();
    }
    content
}

/// Whether `text` carries a `YYYY-MM-DD` anywhere.
///
/// Shape only, not validity: this exists so that a gap says *when* it was
/// found, and a check that also argued about leap years would fail rows for
/// a reason that has nothing to do with SDK parity.
fn contains_iso_date(text: &str) -> bool {
    let chars: Vec<char> = text.chars().collect();
    chars.windows(10).any(|window| {
        window.iter().enumerate().all(|(index, c)| match index {
            4 | 7 => *c == '-',
            _ => c.is_ascii_digit(),
        })
    })
}

/// Every test name declared under `dir`, by the conventions of whichever
/// language declared it.
fn test_names_in(dir: &Path) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for path in parity_sources(dir) {
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        match path.extension().and_then(|e| e.to_str()) {
            Some("rs") => rust_test_names(&text, &mut out),
            Some(extension) if PARITY_TS_EXTENSIONS.contains(&extension) => {
                ts_test_names(&text, &mut out);
            }
            _ => {}
        }
    }
    out
}

/// Every file under `dir` this check knows how to read, skipping
/// [`PARITY_SKIPPED_DIRS`].
fn parity_sources(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let skipped = path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| PARITY_SKIPPED_DIRS.contains(&n));
            if !skipped {
                out.extend(parity_sources(&path));
            }
        } else {
            out.push(path);
        }
    }
    out
}

/// Every `fn` in `text` that an attribute marks as a test, and that no
/// attribute marks as ignored.
///
/// `#[ignore]`d functions are deliberately **not** collected: AGENTS.md's
/// second rule makes an ignored test a declaration that the behaviour is
/// unbuilt, and a matrix cell that cited one would claim a capability is
/// proven by a test that never runs.
fn rust_test_names(text: &str, out: &mut BTreeSet<String>) {
    let lines: Vec<&str> = text.lines().collect();
    for (index, line) in lines.iter().enumerate() {
        let Some(name) = rust_fn_name(line) else {
            continue;
        };
        if attributes_mark_a_live_test(&lines, index) {
            out.insert(name);
        }
    }
}

/// The name of the function this line declares, if it declares one.
fn rust_fn_name(line: &str) -> Option<String> {
    let mut rest = line.trim();
    loop {
        let stripped = [
            "pub(crate) ",
            "pub ",
            "async ",
            "const ",
            "unsafe ",
            "extern ",
        ]
        .iter()
        .find_map(|prefix| rest.strip_prefix(prefix));
        match stripped {
            Some(next) => rest = next.trim_start(),
            None => break,
        }
    }
    let rest = rest.strip_prefix("fn ")?;
    let name: String = rest.chars().take_while(|c| is_ident_char(*c)).collect();
    if name.is_empty() {
        return None;
    }
    let after = rest.get(name.len()..)?.trim_start();
    if !after.starts_with('(') && !after.starts_with('<') {
        return None;
    }
    Some(name)
}

/// Walks back over the attributes and doc comments above `index`: true when
/// one of them names a test harness and none of them ignores it.
fn attributes_mark_a_live_test(lines: &[&str], index: usize) -> bool {
    let mut found_test = false;
    let mut i = index;
    while i > 0 {
        i -= 1;
        let Some(line) = lines.get(i) else {
            break;
        };
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }
        if !trimmed.starts_with("#[") {
            break; // Anything else ends the attribute block.
        }
        if trimmed.starts_with("#[ignore") {
            return false;
        }
        // `#[test]`, `#[tokio::test]`, `#[test_log::test(tokio::test)]`.
        if trimmed.contains("test") {
            found_test = true;
        }
    }
    found_test
}

/// Every `it("…")` / `test("…")` title in `text`.
///
/// Deliberately textual, like every other scan in this file. `it.skip(` and
/// `it.each(` do not match, because the character after the keyword must be
/// `(`; `submit(` and `unit(` do not match, because the character before it
/// must not be part of an identifier.
fn ts_test_names(text: &str, out: &mut BTreeSet<String>) {
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0usize;

    while i < chars.len() {
        let Some(after_keyword) = ts_test_keyword_at(&chars, i) else {
            i += 1;
            continue;
        };
        let mut j = after_keyword;
        while chars.get(j).is_some_and(|c| c.is_whitespace()) {
            j += 1;
        }
        let Some(&quote) = chars.get(j) else {
            break;
        };
        if quote != '"' && quote != '\'' && quote != '`' {
            i += 1;
            continue;
        }
        j += 1;

        let mut title = String::new();
        let mut closed = false;
        while let Some(&c) = chars.get(j) {
            j += 1;
            if c == '\\' {
                if let Some(&escaped) = chars.get(j) {
                    j += 1;
                    title.push(match escaped {
                        'n' => '\n',
                        't' => '\t',
                        'r' => '\r',
                        other => other,
                    });
                }
                continue;
            }
            if c == quote {
                closed = true;
                break;
            }
            if c == '\n' && quote != '`' {
                break; // An unterminated single-line string is not a title.
            }
            title.push(c);
        }

        if closed && !title.is_empty() {
            out.insert(title);
        }
        i = j.max(i + 1);
    }
}

/// If `it(` or `test(` starts at `i` and is not part of a longer identifier
/// or a member expression, the index just past the `(`.
fn ts_test_keyword_at(chars: &[char], i: usize) -> Option<usize> {
    for keyword in ["it", "test"] {
        let letters: Vec<char> = keyword.chars().collect();
        let end = i + letters.len();
        if chars.get(i..end) != Some(letters.as_slice()) {
            continue;
        }
        if chars.get(end) != Some(&'(') {
            continue;
        }
        let preceded = i
            .checked_sub(1)
            .and_then(|p| chars.get(p))
            .is_some_and(|c| is_ident_char(*c) || *c == '.');
        if preceded {
            continue;
        }
        return Some(end + 1);
    }
    None
}

// ---------------------------------------------------------------------------
// verify-links
// ---------------------------------------------------------------------------

/// Fail if a tracked Markdown file links to a path this repository does not
/// track.
///
/// # Why this is a gate and not a report
///
/// Until 2026-09-05 `just docs-check` ran `verify-status` and then printed
/// `note: link checking is not implemented yet`. Every claim this repository
/// makes about itself is machine-checked *except* the one it makes most
/// often: that the document you are reading points at the file it names.
/// `docs/plans/step9-notes/release-claims-review.md`'s own mutation table
/// (row M3) records breaking a link and watching `just docs-check` exit 0.
/// This is the check that row asked for.
///
/// # What is checked, and what is deliberately not
///
/// Checked: every inline link and image destination, and every reference
/// definition, in every file `git ls-files` reports with a `.md` extension,
/// whose destination is a repository path. "Resolves" means the path — after
/// a `#fragment` and a `:line` suffix are stripped and `.`/`..` are folded —
/// is either a tracked file or a directory that contains one. Tracked, not
/// merely present on disk: a link to a build artefact or to a scratch file
/// that is only in the working tree resolves on the author's machine and
/// nowhere else.
///
/// Deliberately **not** checked, so that nobody mistakes a green run for more
/// than it is:
///
/// * **`#anchor` fragments.** Resolving one means agreeing with GitHub's
///   heading-slug algorithm — including its emoji handling, its duplicate
///   suffixes and its `<a name>` support — and disagreeing with it silently
///   turns a correct link into a build failure. A fragment is stripped before
///   the path is resolved, so `docs/flows/money.md#rounding` proves the file
///   exists and says nothing about the heading.
/// * **`http(s)://` URLs.** They need the network, they go stale for reasons
///   outside this repository, and a gate that fails because someone else's
///   site is down gets disabled. `verify-citations` is the deliberate
///   exception: it resolves the *ids this repository cites as evidence*,
///   which are claims about our own history rather than about the internet.
/// * **`mailto:` targets.**
/// * **Reference *usages*** (`[text][label]` with no definition). Only the
///   definitions are resolved; a dangling label renders as literal text
///   rather than as a wrong link, which is visible to a reader in a way a
///   silently-wrong path is not.
fn verify_links(root: &Path) -> Result<(), String> {
    let tracked = tracked_paths(root)?;
    let files: BTreeSet<&str> = tracked.iter().map(String::as_str).collect();
    let dirs = ancestor_directories(&tracked);

    let markdown: Vec<&String> = tracked.iter().filter(|p| p.ends_with(".md")).collect();
    let mut problems = Vec::new();
    let mut checked = 0usize;

    for path in &markdown {
        let text = fs::read_to_string(root.join(path)).map_err(|e| format!("{path}: {e}"))?;
        for link in doc_links(&text) {
            let Some(target_path) = resolvable_path(&link.target) else {
                continue;
            };
            checked += 1;
            match resolve_against(path, &target_path) {
                Some(resolved) => {
                    if !files.contains(resolved.as_str()) && !dirs.contains(resolved.as_str()) {
                        problems.push(format!(
                            "{path}:{}: {} -> {resolved}",
                            link.line, link.target
                        ));
                    }
                }
                None => problems.push(format!(
                    "{path}:{}: {} -> escapes the repository root",
                    link.line, link.target
                )),
            }
        }
    }

    if !problems.is_empty() {
        return Err(format!(
            "{} broken link(s) — each is `file:line: target -> resolved path`:\n  - {}",
            problems.len(),
            problems.join("\n  - ")
        ));
    }

    println!(
        "verify-links: ok — {checked} repository link(s) in {} tracked markdown file(s) resolve to a tracked path (anchors and http(s) URLs are not checked)",
        markdown.len()
    );
    Ok(())
}

/// Every path `git ls-files` reports, repo-relative, in git's own order.
///
/// `git ls-files` rather than a directory walk, and `-z` rather than the
/// default: the default quotes and escapes any path that is not plain ASCII,
/// which would make a link to such a file look broken. Asking git also means
/// an untracked scratch file — a rendered diagram, a downloaded log, an
/// agent's notes — can never satisfy a link, which is the property that makes
/// a green `verify-links` mean anything on a fresh clone.
fn tracked_paths(root: &Path) -> Result<Vec<String>, String> {
    let output = std::process::Command::new("git")
        .args(["ls-files", "-z"])
        .current_dir(root)
        .output()
        .map_err(|e| format!("running `git ls-files`: {e}"))?;

    if !output.status.success() {
        return Err(format!(
            "`git ls-files` failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    Ok(String::from_utf8_lossy(&output.stdout)
        .split('\0')
        .filter(|entry| !entry.is_empty())
        .map(str::to_owned)
        .collect())
}

/// Every directory that contains a tracked file, at every depth.
///
/// Git tracks files, never directories, so a link to `docs/runbooks/` has
/// nothing to match against unless the directories are derived. Derived from
/// the tracked list rather than read from the filesystem, for the same reason
/// the file set is: an empty directory left behind by a build is not part of
/// the repository, and a link into one is broken for everybody else.
fn ancestor_directories(tracked: &[String]) -> BTreeSet<String> {
    let mut dirs = BTreeSet::new();
    for path in tracked {
        let mut current = path.as_str();
        while let Some((parent, _)) = current.rsplit_once('/') {
            if !dirs.insert(parent.to_owned()) {
                break;
            }
            current = parent;
        }
    }
    dirs
}

/// One link destination as written, and the line it was written on.
#[derive(Debug, PartialEq, Eq)]
struct DocLink {
    /// 1-based, so a failure can be pasted into an editor.
    line: usize,
    /// Exactly as written, before any fragment or `:line` suffix is stripped
    /// — a failure message must show the reader what is in their file, not
    /// what the checker made of it.
    target: String,
}

/// Every link destination in a Markdown document.
///
/// Inline links (`[text](target)`), images (`![alt](target)`) and reference
/// definitions (`[label]: target`), with fenced code blocks, HTML comments
/// and inline code spans masked out first — see [`mask_non_links`].
fn doc_links(text: &str) -> Vec<DocLink> {
    let masked = mask_non_links(text);
    let chars: Vec<char> = masked.chars().collect();
    let line_starts = line_start_indices(&chars);

    let mut links = inline_links(&chars, &line_starts);
    links.extend(reference_definitions(&masked));
    links
}

/// The index of the first character of each line, for [`line_of`].
fn line_start_indices(chars: &[char]) -> Vec<usize> {
    let mut starts = vec![0usize];
    for (i, c) in chars.iter().enumerate() {
        if *c == '\n' {
            starts.push(i + 1);
        }
    }
    starts
}

/// The 1-based line number containing character `index`.
fn line_of(line_starts: &[usize], index: usize) -> usize {
    line_starts.partition_point(|start| *start <= index)
}

/// Every `[text](destination)` — image or not — in already-masked text.
///
/// Scans for a balanced `[` … `]` immediately followed by `(`, rather than
/// for the two characters `](`: a stray `](` in prose is not a link, and a
/// gate that reports one teaches people to ignore it. The link *text* may
/// span lines (a formatter is free to wrap it); the destination may not,
/// which is CommonMark's own rule.
fn inline_links(chars: &[char], line_starts: &[usize]) -> Vec<DocLink> {
    let mut links = Vec::new();
    let mut i = 0usize;
    while i < chars.len() {
        match chars.get(i) {
            Some('\\') => {
                i += 2;
                continue;
            }
            Some('[') => {
                if let Some(close) = matching_square_bracket(chars, i)
                    && chars.get(close + 1) == Some(&'(')
                    && let Some((target, end)) = link_destination(chars, close + 2)
                {
                    links.push(DocLink {
                        line: line_of(line_starts, close + 2),
                        target,
                    });
                    i = end;
                    continue;
                }
            }
            _ => {}
        }
        i += 1;
    }
    links
}

/// The index of the `]` that closes the `[` at `open`, honouring nesting and
/// backslash escapes. `None` if it is never closed.
fn matching_square_bracket(chars: &[char], open: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut i = open;
    while let Some(c) = chars.get(i) {
        match c {
            '\\' => i += 1,
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// The destination of an inline link whose `(` sits just before `from`, and
/// the index just past the closing `)`.
///
/// Both CommonMark spellings: bare (`(docs/x.md)`, parentheses balanced so
/// `(a(b).md)` survives) and angle-bracketed (`(<path with spaces>)`, which
/// is the only way to write a destination containing a space). An optional
/// `"title"` may follow. A newline inside the destination is not a link, so
/// the scan gives up rather than swallowing the rest of the paragraph.
fn link_destination(chars: &[char], from: usize) -> Option<(String, usize)> {
    let mut i = from;
    while matches!(chars.get(i), Some(' ' | '\t')) {
        i += 1;
    }

    let mut destination = String::new();
    if chars.get(i) == Some(&'<') {
        i += 1;
        loop {
            match chars.get(i) {
                None | Some('\n') => return None,
                Some('>') => {
                    i += 1;
                    break;
                }
                Some('\\') => {
                    destination.push(*chars.get(i + 1)?);
                    i += 2;
                }
                Some(c) => {
                    destination.push(*c);
                    i += 1;
                }
            }
        }
    } else {
        let mut depth = 0usize;
        loop {
            match chars.get(i) {
                None | Some('\n') => return None,
                Some('\\') => {
                    destination.push(*chars.get(i + 1)?);
                    i += 2;
                }
                Some('(') => {
                    depth += 1;
                    destination.push('(');
                    i += 1;
                }
                Some(')') => {
                    if depth == 0 {
                        break;
                    }
                    depth -= 1;
                    destination.push(')');
                    i += 1;
                }
                Some(c) if c.is_whitespace() => break,
                Some(c) => {
                    destination.push(*c);
                    i += 1;
                }
            }
        }
    }

    // Whatever follows the destination — whitespace, then at most a title —
    // has to end at the `)` that closes the link, or this was never one.
    loop {
        match chars.get(i) {
            Some(')') => return Some((destination, i + 1)),
            Some(c) if c.is_whitespace() && *c != '\n' => i += 1,
            Some(quote @ ('"' | '\'')) => {
                let quote = *quote;
                i += 1;
                loop {
                    match chars.get(i) {
                        None => return None,
                        Some('\\') => i += 2,
                        Some(c) if *c == quote => {
                            i += 1;
                            break;
                        }
                        _ => i += 1,
                    }
                }
            }
            _ => return None,
        }
    }
}

/// Every `[label]: destination` definition in already-masked text.
///
/// Line-based, because that is what a definition is: it has to start a line
/// (up to three spaces of indent, CommonMark's rule) and its destination runs
/// to the first whitespace or to the closing `>`.
fn reference_definitions(masked: &str) -> Vec<DocLink> {
    let mut links = Vec::new();
    for (index, raw) in masked.lines().enumerate() {
        let line = raw.trim_start();
        if raw.len() - line.len() > 3 || !line.starts_with('[') {
            continue;
        }
        let Some(label_end) = line.find("]:") else {
            continue;
        };
        let rest = line.get(label_end + 2..).unwrap_or_default().trim_start();
        let target = if let Some(angled) = rest.strip_prefix('<') {
            match angled.find('>') {
                Some(end) => angled.get(..end).unwrap_or_default(),
                None => continue,
            }
        } else {
            rest.split_whitespace().next().unwrap_or_default()
        };
        if !target.is_empty() {
            links.push(DocLink {
                line: index + 1,
                target: target.to_owned(),
            });
        }
    }
    links
}

/// Blank out every region of a Markdown document in which link syntax is not
/// link syntax, preserving line breaks and character positions so a line
/// number survives.
///
/// Three regions, masked in this order, and the order is the point:
///
/// 1. **Fenced code blocks.** A shell transcript that contains `[a](b)` is
///    not a link, and this repository's runbooks are mostly transcripts.
/// 2. **Inline code spans**, one line at a time. A backtick run only opens a
///    span if a run of the same length closes it *on the same line*: a
///    multi-line span is legal CommonMark, but honouring it here means one
///    stray backtick can blank the rest of a document, and a masker that
///    deletes too much makes this gate pass by finding nothing.
/// 3. **HTML comments**, and only when terminated, for the same reason: an
///    unclosed `<!--` is left as ordinary text rather than swallowing the
///    file.
fn mask_non_links(text: &str) -> String {
    mask_html_comments(&mask_code_spans(&mask_fenced_blocks(text)))
}

/// Replace the contents of every ```` ``` ````/`~~~` fenced block — and the
/// fence lines themselves — with spaces.
fn mask_fenced_blocks(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut fence: Option<(char, usize)> = None;
    for line in text.split_inclusive('\n') {
        let marker = fence_marker(line);
        let inside = match (fence, marker) {
            (None, Some((c, len, _))) => {
                fence = Some((c, len));
                true
            }
            (Some((open_char, open_len)), Some((c, len, has_info)))
                if c == open_char && len >= open_len && !has_info =>
            {
                fence = None;
                true
            }
            (Some(_), _) => true,
            (None, None) => false,
        };
        if inside {
            out.extend(line.chars().map(|c| if c == '\n' { '\n' } else { ' ' }));
        } else {
            out.push_str(line);
        }
    }
    out
}

/// The fence character, its run length, and whether an info string follows,
/// if this line could open or close a fenced code block. Up to three spaces
/// of indent, three or more backticks or tildes.
///
/// The info-string flag is what keeps nested fences from unbalancing the
/// mask: CommonMark lets only a *bare* fence close a block, so a document
/// that quotes ```` ```rust ```` inside a longer fence does not end it three
/// lines early — and a mask that ended early would expose code as prose,
/// which is the direction that produces false failures.
///
/// # The clause that is not pedantry
///
/// CommonMark also says a backtick fence's info string may not itself
/// contain a backtick, and that clause is doing real work here.
/// `docs/status.md` line 69 begins
/// ```` ```` ```ignore ```` ```` — a four-backtick *code span* whose content
/// is a three-backtick fence, which is how this repository writes about
/// doctest fences. Read as an opening fence it never closes, and the mask
/// swallows 2 200 of that file's 2 268 lines: `verify-links` would have
/// reported `ok` while checking almost nothing in the most link-dense
/// document in the tree. Measured rather than reasoned about — the link
/// count went 591 → 672 when this clause was added, and
/// `a_backtick_run_whose_info_string_holds_backticks_opens_no_fence` fails
/// if it is taken out again.
fn fence_marker(line: &str) -> Option<(char, usize, bool)> {
    let trimmed = line.trim_start_matches(' ');
    if line.len() - trimmed.len() > 3 {
        return None;
    }
    let marker = trimmed.chars().next().filter(|c| matches!(c, '`' | '~'))?;
    let run = trimmed.chars().take_while(|c| *c == marker).count();
    let info = trimmed.get(run..).unwrap_or_default().trim();
    if run < 3 || (marker == '`' && info.contains('`')) {
        return None;
    }
    Some((marker, run, !info.is_empty()))
}

/// Replace every inline code span, delimiters included, with spaces.
fn mask_code_spans(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        let chars: Vec<char> = line.chars().collect();
        let mut masked: Vec<char> = chars.clone();
        let mut i = 0usize;
        while i < chars.len() {
            if chars.get(i) != Some(&'`') {
                i += 1;
                continue;
            }
            let open_len = chars
                .get(i..)
                .map_or(0, |rest| rest.iter().take_while(|c| **c == '`').count());
            let mut j = i + open_len;
            let close = loop {
                match chars.get(j) {
                    None => break None,
                    Some('`') => {
                        let run = chars
                            .get(j..)
                            .map_or(0, |rest| rest.iter().take_while(|c| **c == '`').count());
                        if run == open_len {
                            break Some(j + run);
                        }
                        j += run;
                    }
                    Some(_) => j += 1,
                }
            };
            match close {
                Some(end) => {
                    for slot in masked.get_mut(i..end).into_iter().flatten() {
                        if *slot != '\n' {
                            *slot = ' ';
                        }
                    }
                    i = end;
                }
                None => i += open_len,
            }
        }
        out.extend(masked);
    }
    out
}

/// Replace every terminated `<!-- … -->` with spaces, across lines.
fn mask_html_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("<!--") {
        let after = rest.get(start + 4..).unwrap_or_default();
        let Some(end) = after.find("-->") else {
            break;
        };
        out.push_str(rest.get(..start).unwrap_or_default());
        let comment = rest.get(start..start + 4 + end + 3).unwrap_or_default();
        out.extend(comment.chars().map(|c| if c == '\n' { '\n' } else { ' ' }));
        rest = rest.get(start + 4 + end + 3..).unwrap_or_default();
    }
    out.push_str(rest);
    out
}

/// The repository path a link destination points at, or `None` if this gate
/// does not resolve that kind of destination.
///
/// `None` for an empty destination, a pure `#anchor`, an `http(s)` URL and a
/// `mailto:` — see [`verify_links`] for why each is out of scope. Otherwise
/// the fragment and a trailing `:line` (or `:line:column`) suffix are removed
/// and percent escapes are decoded, because `docs/flows/errors.md#retry` and
/// `backends/crates/vpay-db/src/lib.rs:42` both name a file that either
/// exists or does not.
fn resolvable_path(target: &str) -> Option<String> {
    let target = target.trim();
    if target.is_empty() || target.starts_with('#') {
        return None;
    }
    let lower = target.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("mailto:")
    {
        return None;
    }

    let without_fragment = target.split('#').next().unwrap_or_default();
    let decoded = percent_decode(without_fragment);
    let path = strip_line_suffix(&decoded);
    (!path.is_empty()).then(|| path.to_owned())
}

/// `path` without a trailing `:42` or `:42:7`.
///
/// Editors and this repository's own prose cite a source location that way
/// (`src/lib.rs:42`), and the file is what a link checker can answer for.
fn strip_line_suffix(path: &str) -> &str {
    let once = strip_one_numeric_suffix(path);
    strip_one_numeric_suffix(once)
}

/// `path` without one trailing `:<digits>`, if it has one and something
/// precedes it.
fn strip_one_numeric_suffix(path: &str) -> &str {
    match path.rsplit_once(':') {
        Some((head, tail))
            if !head.is_empty() && !tail.is_empty() && tail.chars().all(|c| c.is_ascii_digit()) =>
        {
            head
        }
        _ => path,
    }
}

/// Decode `%XX` escapes. A malformed escape is left exactly as written — a
/// link that does not decode is a link that does not resolve, and showing the
/// author what they typed beats showing them a mangling of it.
fn percent_decode(target: &str) -> String {
    let bytes = target.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0usize;
    while let Some(byte) = bytes.get(i) {
        let decoded = (*byte == b'%')
            .then(|| {
                let hex = target.get(i + 1..i + 3)?;
                u8::from_str_radix(hex, 16).ok()
            })
            .flatten();
        match decoded {
            Some(value) => {
                out.push(value);
                i += 3;
            }
            None => {
                out.push(*byte);
                i += 1;
            }
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| target.to_owned())
}

/// `target` resolved against the directory holding `from`, folded to a
/// repo-relative path. `None` if it climbs above the repository root, which
/// is broken in a way no path can name.
///
/// A leading `/` is read as repo-root-relative: that is what a reader of a
/// rendered document expects, and if the repository does not hold the path,
/// the gate fails on the resolved name either way.
fn resolve_against(from: &str, target: &str) -> Option<String> {
    let mut segments: Vec<&str> = Vec::new();
    if !target.starts_with('/')
        && let Some((dir, _)) = from.rsplit_once('/')
    {
        segments.extend(dir.split('/'));
    }
    for segment in target.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop()?;
            }
            other => segments.push(other),
        }
    }
    Some(segments.join("/"))
}

// ---------------------------------------------------------------------------
// verify-citations
// ---------------------------------------------------------------------------

/// Names this repository has been published under, for reading a GitHub URL
/// written before a rename.
///
/// The live name is asked of `gh` rather than hard-coded (see
/// [`github_repository`]); these are the historical ones, and they are here
/// because the tree contains URLs under all of them — the remote is
/// `vymalo/vpay`, `gh` resolves it to `vaam-apps/vpay`, and
/// `docs/flows/deployment.md` still links `vaam-store/vpay`, the name it had
/// before 2026-09-05. GitHub redirects all three to the same repository, so
/// treating them as one is not a convenience: an id under any of them is a
/// claim about *our* history and has to resolve.
const REPOSITORY_ALIASES: [&str; 3] = ["vaam-apps/vpay", "vaam-store/vpay", "vymalo/vpay"];

/// `(file, id)` pairs that look like citations and are deliberately not
/// claims.
///
/// Three entries, one id, one finding: the Step 9 release-claims notes, their
/// review, and this command's own notes all record a mutation test in which a
/// real run id was replaced by `39999999999` to show that *nothing in the
/// repository noticed*. That finding is what this command exists to close,
/// and a document cannot state it without printing an id that does not exist.
/// The first two carry a dated note saying the gap is closed; the third is
/// the record of closing it.
///
/// A constant here rather than a marker in the prose, on purpose. A marker
/// (`<!-- verify-citations: ignore -->`) is invisible to a reader and can be
/// sprayed over a document by anyone who wants a gate to be quiet; a pair in
/// this array is a code change that shows up in review, and it is scoped to
/// one file, so the same eleven digits written anywhere else are still
/// checked.
const CITATIONS_THAT_ARE_NOT_CLAIMS: [(&str, &str); 3] = [
    ("docs/plans/exp6-notes/opus.md", "39999999999"),
    ("docs/plans/step9-notes/release-claims.md", "39999999999"),
    (
        "docs/plans/step9-notes/release-claims-review.md",
        "39999999999",
    ),
];

/// What a cited id names, and therefore which endpoint answers for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum CitationKind {
    /// A GitHub Actions workflow run.
    Run,
    /// A pull request. Resolved against `/pulls/{n}`, which 404s for a plain
    /// issue — so "PR #11" fails when 11 is an issue, which is the whole
    /// point of tracking the cue rather than lumping both together.
    Pull,
    /// An issue. Resolved against `/issues/{n}`, which answers for pull
    /// requests too, so an `#n` cited as an issue is satisfied either way.
    Issue,
}

impl CitationKind {
    /// The `gh api` path that resolves an id of this kind against the
    /// repository `gh` is pointed at.
    fn api_path(self, id: &str) -> String {
        match self {
            Self::Run => format!("repos/{{owner}}/{{repo}}/actions/runs/{id}"),
            Self::Pull => format!("repos/{{owner}}/{{repo}}/pulls/{id}"),
            Self::Issue => format!("repos/{{owner}}/{{repo}}/issues/{id}"),
        }
    }

    /// How the id is spelled back to a human in a report line.
    fn label(self) -> &'static str {
        match self {
            Self::Run => "run",
            Self::Pull => "PR",
            Self::Issue => "issue",
        }
    }
}

/// One id a document cites as evidence.
#[derive(Debug, PartialEq, Eq)]
struct Citation {
    kind: CitationKind,
    id: String,
    /// 1-based line of the occurrence.
    line: usize,
}

/// Fail if a document cites a workflow run, pull request or issue id that
/// does not exist.
///
/// # Why this needs the network, and why it still must not be skippable
///
/// This repository's documents argue from evidence: "run `33929374661`
/// pushed four digests", "PR #31 reopened the event vocabulary". An id is
/// the strongest kind of claim a document here makes, because a reader can
/// check it — and the only thing that can check it is GitHub. So this is a
/// gate that needs the network, which makes it unfit for `just ci` (a CI job
/// that fails when the API rate-limits is a job people learn to re-run
/// without reading). It is opt-in: `just docs-check-citations`.
///
/// What it will **not** do is print "skipped" and exit 0. A check that
/// downgrades itself when its dependency is missing reports success for a
/// run in which nothing was checked, and that is indistinguishable, in a log,
/// from a run in which everything passed. Without `gh`, or without
/// authentication, this command fails and says which.
///
/// # What counts as a citation
///
/// * **A run:** any standalone eleven-digit number, and any
///   `actions/runs/<digits>` in a URL naming this repository. Eleven digits
///   rather than the word `run` followed by digits, and that is a deliberate
///   widening: this repository writes runs in comma-separated lists
///   (``Runs `33772512791`, `33784613048`, `33789060270` ``), where a
///   cue-word rule would check the first and ignore the rest — which is
///   exactly where a wrong id hides. Every eleven-digit number in the tree is
///   a run id; `sha-33929374661` and `339293746612` are not standalone and do
///   not count.
/// * **A pull request:** `#n` whose nearest preceding word is `PR`, `PRs`,
///   `pull`, `pulls` or `pull request(s)`, plus any `#n` continuing such a
///   run through nothing but separators (`PRs #16–#17`, `PR #27 and #28`);
///   and `pull/<n>` in a URL naming this repository.
/// * **An issue:** the same, cued by `issue`/`issues`, plus `issues/<n>` in a
///   URL naming this repository.
///
/// # What is deliberately not a citation
///
/// A cue is required because `#n` on its own is ambiguous in this tree and
/// resolving the ambiguous ones would fail the gate on correct prose:
/// `Order #42` and `Order #1234` are example payloads in the runbooks,
/// ``Commit `#7` `` and `(#1, CLI/env config)` number commits rather than
/// pull requests, and `AGENTS.md open question #4` numbers a question. A `#n`
/// that begins a line is a Markdown heading. `PKCS#8` and `authkestra#287`
/// are excluded by the character before the `#`: the second is a
/// cross-repository reference, and resolving one means knowing which
/// repository it belongs to — out of scope here, and better wrong-loudly
/// than wrong-quietly.
///
/// The cost of the cue rule, stated rather than hidden: an id cited *only*
/// without a cue is not checked. On 2026-09-05 that set is empty — every
/// bold `**#17**` in `docs/roadmap.md`'s third addendum is also written
/// `PR #17` elsewhere in the same file, so every one is resolved.
fn verify_citations(root: &Path) -> Result<(), String> {
    verify_citations_via(root, GH)
}

/// The GitHub CLI this command shells out to. A constant so a test can pass a
/// name that is not on `PATH` and prove the "it never skips" paragraph above
/// is behaviour rather than prose — see
/// `a_missing_gh_fails_the_gate_rather_than_skipping`.
const GH: &str = "gh";

/// [`verify_citations`], with the CLI to shell out to named.
///
/// The seam exists for one test and is worth it: "fails rather than skips when
/// `gh` is missing" was the property with no guard on it when this command
/// landed, which is the shape of gap CLAUDE.md's "a skipped test is not a
/// passing test" is about. Every other caller uses [`GH`].
fn verify_citations_via(root: &Path, gh: &str) -> Result<(), String> {
    let repository = github_repository(root, gh)?;
    let mut aliases: Vec<String> = REPOSITORY_ALIASES.iter().map(|a| (*a).to_owned()).collect();
    aliases.push(repository.clone());

    let tracked = tracked_paths(root)?;
    let markdown: Vec<&String> = tracked.iter().filter(|p| p.ends_with(".md")).collect();

    // Deduped by (kind, id) so the API is asked once however often a document
    // repeats an id — `33929374661` appears eleven times in this tree.
    let mut cited: BTreeMap<(CitationKind, String), Vec<String>> = BTreeMap::new();
    for path in &markdown {
        let text = fs::read_to_string(root.join(path)).map_err(|e| format!("{path}: {e}"))?;
        for citation in citations_in(&text, &aliases) {
            if CITATIONS_THAT_ARE_NOT_CLAIMS
                .iter()
                .any(|(file, id)| *file == path.as_str() && *id == citation.id)
            {
                continue;
            }
            cited
                .entry((citation.kind, citation.id))
                .or_default()
                .push(format!("{path}:{}", citation.line));
        }
    }

    let mut missing = Vec::new();
    for ((kind, id), sites) in &cited {
        let status = gh_status(root, gh, &kind.api_path(id))?;
        if status == 200 {
            println!(
                "  ok    {} {id} ({} citation(s))",
                kind.label(),
                sites.len()
            );
        } else {
            println!("  MISS  {} {id} — HTTP {status}", kind.label());
            missing.push(format!(
                "{} {id} does not exist (HTTP {status}), cited at {}",
                kind.label(),
                sites.join(", ")
            ));
        }
    }

    if !missing.is_empty() {
        return Err(format!(
            "{} cited id(s) do not exist in {repository}. A citation that does not \
             resolve is a false claim: strike it through with a dated correction \
             rather than replacing it with an id you have not checked.\n  - {}",
            missing.len(),
            missing.join("\n  - ")
        ));
    }

    println!(
        "verify-citations: ok — {} unique id(s) cited by {} markdown file(s) all resolve against {repository}",
        cited.len(),
        markdown.len()
    );
    Ok(())
}

/// The `owner/repo` `gh` resolves the current checkout to.
///
/// This is also the preflight: it is the call that fails when `gh` is absent,
/// unauthenticated, or pointed at a repository it cannot read, so every later
/// failure can be reported as what it is — a citation that does not exist —
/// rather than as an outage.
fn github_repository(root: &Path, gh: &str) -> Result<String, String> {
    let output = std::process::Command::new(gh)
        .args(["api", "repos/{owner}/{repo}", "--jq", ".full_name"])
        .current_dir(root)
        .output()
        .map_err(|e| {
            format!(
                "verify-citations needs the GitHub CLI and cannot run without it: {e}. \
                 Install `gh` and run `gh auth login`. This command never skips — a \
                 citation nothing resolved is a citation nothing checked."
            )
        })?;

    if !output.status.success() {
        return Err(format!(
            "`gh api repos/{{owner}}/{{repo}}` failed ({}): {}. Run `gh auth status`; \
             this command needs a token that can read this repository.",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    let name = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if name.is_empty() {
        return Err("`gh api repos/{owner}/{repo}` returned no repository name".to_owned());
    }
    Ok(name)
}

/// The HTTP status GitHub answers `path` with.
///
/// `gh api -i` rather than the exit code: the two answers this gate has to
/// tell apart — "that id does not exist" (404, a false claim, and the
/// failure) and "GitHub would not answer" (403, 429, 5xx, an outage) — are
/// both a non-zero exit. Anything that is not a 200 or a 404 stops the whole
/// command rather than being reported per id, because a rate-limited run
/// would otherwise report every remaining citation as missing and send
/// somebody to delete true claims.
fn gh_status(root: &Path, gh: &str, path: &str) -> Result<u16, String> {
    let output = std::process::Command::new(gh)
        .args(["api", "-i", path])
        .current_dir(root)
        .output()
        .map_err(|e| format!("running `gh api -i {path}`: {e}"))?;

    let head = String::from_utf8_lossy(&output.stdout);
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or_else(|| {
            format!(
                "could not read a status line from `gh api -i {path}`: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )
        })?;

    classify_gh_status(path, status)
}

/// Which of GitHub's answers is a finding and which is an outage.
///
/// Pure, and separate from [`gh_status`], so the distinction this gate turns
/// on can be tested without the network: a 404 is the *finding* this command
/// exists to report (a cited id that does not exist), and everything else that
/// is not a 200 is an outage that must stop the run. Reporting a rate-limited
/// batch as "missing" would send somebody to delete true claims, which is the
/// worst thing this command could do.
fn classify_gh_status(path: &str, status: u16) -> Result<u16, String> {
    match status {
        200 | 404 => Ok(status),
        403 | 429 => Err(format!(
            "GitHub refused `{path}` with HTTP {status} — rate limited or out of scope \
             for this token. Nothing was concluded about the remaining citations; \
             re-run when the limit resets rather than treating them as unresolved."
        )),
        other => Err(format!("GitHub answered `{path}` with HTTP {other}")),
    }
}

/// Every citation in a document, with its 1-based line.
///
/// Unlike [`doc_links`], this reads the text as written — fenced blocks and
/// code spans included. An id inside a fence is still a claim: this
/// repository's runbooks cite evidence by pasting the command that produces
/// it (``gh run view `33929374661` ``), and the ids are almost always inside
/// backticks.
fn citations_in(text: &str, repository_aliases: &[String]) -> Vec<Citation> {
    let mut citations = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let line_number = index + 1;
        for (kind, id) in repository_url_citations(line, repository_aliases) {
            citations.push(Citation {
                kind,
                id,
                line: line_number,
            });
        }
        for id in run_id_citations(line) {
            citations.push(Citation {
                kind: CitationKind::Run,
                id,
                line: line_number,
            });
        }
        for (kind, id) in hash_citations(line) {
            citations.push(Citation {
                kind,
                id,
                line: line_number,
            });
        }
    }
    citations
}

/// Citations written as a GitHub URL, but only when the URL names *this*
/// repository. `github.com/marcjazz/authkestra/issues/185` is somebody
/// else's tracker and resolving it here would ask GitHub the wrong question.
fn repository_url_citations(line: &str, aliases: &[String]) -> Vec<(CitationKind, String)> {
    let mut found = Vec::new();
    let mut rest = line;
    while let Some(at) = rest.find("github.com/") {
        let tail = rest.get(at + "github.com/".len()..).unwrap_or_default();
        rest = tail;
        let mut segments = tail.split('/');
        let (Some(owner), Some(repo)) = (segments.next(), segments.next()) else {
            continue;
        };
        if !aliases
            .iter()
            .any(|alias| *alias == format!("{owner}/{repo}"))
        {
            continue;
        }
        let kind = match segments.next() {
            Some("pull") => CitationKind::Pull,
            Some("issues") => CitationKind::Issue,
            Some("actions") if segments.next() == Some("runs") => CitationKind::Run,
            _ => continue,
        };
        let digits: String = segments
            .next()
            .unwrap_or_default()
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        if !digits.is_empty() {
            found.push((kind, digits));
        }
    }
    found
}

/// The number of digits a GitHub Actions run id has had for this
/// repository's whole history, and the width every id in this tree is
/// written at.
const RUN_ID_DIGITS: usize = 11;

/// Every standalone eleven-digit number on a line.
///
/// "Standalone" excludes a run that is part of a longer word, path or
/// dotted number: `sha-33929374661` names an image tag, `runs/33929374661`
/// is answered for by [`repository_url_citations`] (which knows *whose* run
/// it is), and a twelve-digit number is not a run id.
fn run_id_citations(line: &str) -> Vec<String> {
    let chars: Vec<char> = line.chars().collect();
    let mut ids = Vec::new();
    let mut i = 0usize;
    while i < chars.len() {
        if !chars.get(i).is_some_and(char::is_ascii_digit) {
            i += 1;
            continue;
        }
        let mut end = i;
        while chars.get(end).is_some_and(char::is_ascii_digit) {
            end += 1;
        }
        let before = i.checked_sub(1).and_then(|k| chars.get(k)).copied();
        let after = chars.get(end).copied();
        let attached_before =
            before.is_some_and(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '/' | '.'));
        let attached_after =
            after.is_some_and(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '/'));
        if end - i == RUN_ID_DIGITS && !attached_before && !attached_after {
            ids.push(chars.get(i..end).unwrap_or_default().iter().collect());
        }
        i = end;
    }
    ids
}

/// Every `#n` on a line that a cue marks as a pull request or an issue.
///
/// See [`verify_citations`] for the rule and for what it deliberately does
/// not claim.
fn hash_citations(line: &str) -> Vec<(CitationKind, String)> {
    let chars: Vec<char> = line.chars().collect();
    let mut found = Vec::new();
    let mut previous: Option<(CitationKind, usize)> = None;
    let mut i = 0usize;
    while i < chars.len() {
        if chars.get(i) != Some(&'#') {
            i += 1;
            continue;
        }
        let mut end = i + 1;
        while chars.get(end).is_some_and(char::is_ascii_digit) {
            end += 1;
        }
        if end == i + 1 {
            i += 1;
            continue;
        }
        let before = i.checked_sub(1).and_then(|k| chars.get(k)).copied();
        let after = chars.get(end).copied();
        let attached_before = before.is_some_and(|c| c.is_alphanumeric() || matches!(c, '_' | '/'));
        let attached_after = after.is_some_and(|c| c.is_alphanumeric() || matches!(c, '-' | '_'));
        let heading = chars
            .get(..i)
            .is_some_and(|prefix| prefix.iter().all(|c| c.is_whitespace()));
        if attached_before || attached_after || heading {
            i = end;
            continue;
        }

        let kind = continued_citation(&chars, previous, i)
            .or_else(|| cue_before(chars.get(..i).unwrap_or_default()));
        if let Some(kind) = kind {
            found.push((
                kind,
                chars.get(i + 1..end).unwrap_or_default().iter().collect(),
            ));
            previous = Some((kind, end));
        }
        i = end;
    }
    found
}

/// Characters that may sit between a cue word and the `#` it cues —
/// formatting and opening punctuation, nothing that carries meaning.
const CUE_SKIPPED: [char; 7] = ['(', '[', '{', '*', '_', '`', '~'];

/// The kind a `#n` at `hash` inherits from the citation before it, when the
/// two are separated by nothing but list punctuation.
///
/// `PRs #16–#17`, `PRs #23, #24` and `PR #27 and #28` all cite two pull
/// requests and carry one cue between them. Anything with a word other than
/// `and` in the gap — `PR #20) — the real Order #42` — is not a
/// continuation, which is what keeps a cue from bleeding down a line.
fn continued_citation(
    chars: &[char],
    previous: Option<(CitationKind, usize)>,
    hash: usize,
) -> Option<CitationKind> {
    let (kind, end) = previous?;
    let gap: String = chars.get(end..hash)?.iter().collect();
    let without_and = gap.to_lowercase().replace("and", " ");
    without_and
        .chars()
        .all(|c| {
            c.is_whitespace() || matches!(c, ',' | '&' | '+' | '–' | '—' | '-' | '/' | '*' | '`')
        })
        .then_some(kind)
}

/// The kind the word before a `#n` cues, if it cues one.
fn cue_before(prefix: &[char]) -> Option<CitationKind> {
    let word = trailing_word(prefix)?;
    match word.0.as_str() {
        "pr" | "prs" | "pull" | "pulls" => Some(CitationKind::Pull),
        "issue" | "issues" => Some(CitationKind::Issue),
        // `pull request #14` — the cue is two words, and only the second one
        // is adjacent.
        "request" | "requests" => match trailing_word(prefix.get(..word.1).unwrap_or_default()) {
            Some((previous, _)) if previous == "pull" => Some(CitationKind::Pull),
            _ => None,
        },
        _ => None,
    }
}

/// The last alphabetic word in `prefix`, lowercased, and the index at which
/// it starts — so a caller can ask for the word before it.
fn trailing_word(prefix: &[char]) -> Option<(String, usize)> {
    let mut end = prefix.len();
    while end > 0
        && prefix
            .get(end - 1)
            .is_some_and(|c| c.is_whitespace() || CUE_SKIPPED.contains(c))
    {
        end -= 1;
    }
    let mut start = end;
    while start > 0 && prefix.get(start - 1).is_some_and(|c| c.is_alphabetic()) {
        start -= 1;
    }
    if start == end {
        return None;
    }
    Some((
        prefix
            .get(start..end)
            .unwrap_or_default()
            .iter()
            .collect::<String>()
            .to_lowercase(),
        start,
    ))
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
    ///
    /// `pub(crate)` because `sdk_parity_tests` builds a synthetic SDK tree
    /// with it: two test modules needing a temp directory is not a reason to
    /// have two temp-directory guards.
    pub(crate) struct TempDir(PathBuf);

    impl TempDir {
        pub(crate) fn new(label: &str) -> Self {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock is after the unix epoch")
                .as_nanos();
            let path = std::env::temp_dir()
                .join(format!("vpay-xtask-{label}-{}-{nanos}", std::process::id()));
            fs::create_dir_all(&path).expect("temp dir is creatable");
            Self(path)
        }

        /// The directory itself, for a caller that builds a tree inside it.
        pub(crate) fn path(&self) -> &Path {
            &self.0
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
    use crate::signing_key_tests::TempDir;

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

    /// The two *inputs* `verify_status` compares, over synthetic sources —
    /// that `scan_not_implemented` sees an undeclared token and that
    /// `declared_tokens` sees a bullet no code carries.
    ///
    /// It stops there: the comparison itself is re-implemented in the body
    /// below, so this test passes whatever `verify_status` does with the two
    /// sets — deleting the docs→code half of it outright leaves this green.
    /// [`verify_status_reports_both_directions_from_the_gate_itself`] is the
    /// one that reads what the gate prints.
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

    /// Both directions again, this time through `verify_status` itself over
    /// a two-file tree on disk.
    ///
    /// [`the_status_check_fails_in_both_directions`] compares the two sets a
    /// second time in its own body, which is exactly the shape that cannot
    /// notice the comparison going missing. This one asserts on the message
    /// the gate prints, so removing either half of it fails here.
    #[test]
    fn verify_status_reports_both_directions_from_the_gate_itself() {
        let dir = TempDir::new("verify-status");
        let root = dir.path();
        let src = root.join("backends/crates/probe/src");
        fs::create_dir_all(&src).expect("the temp tree is creatable");
        fs::create_dir_all(root.join("docs")).expect("the temp tree is creatable");
        fs::write(
            src.join("lib.rs"),
            "fn f() { Err(ProviderError::NotImplemented(\"new::gap\")) }\n",
        )
        .expect("the source file is writable");

        // One token in code the page does not declare, one bullet on the
        // page no code carries. A gate missing either half reports one.
        fs::write(
            root.join("docs/status.md"),
            format!("{STATUS_TOKEN_HEADING}\n\n- `built::already`\n"),
        )
        .expect("the status page is writable");
        let error = verify_status(root).expect_err("both halves are wrong");
        assert!(
            error.contains("missing from docs/status.md") && error.contains("new::gap"),
            "the code→docs half must name the undeclared token: {error}"
        );
        assert!(
            error.contains("no shipping code carries them") && error.contains("built::already"),
            "the docs→code half must name the stale bullet: {error}"
        );

        // And the corrected page passes, so neither half fires on nothing.
        fs::write(
            root.join("docs/status.md"),
            format!("{STATUS_TOKEN_HEADING}\n\n- `new::gap`\n"),
        )
        .expect("the status page is writable");
        verify_status(root).expect("a page that matches the code passes");
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

    /// The characterising test for the four *non-code* places a token can be
    /// written. `searchable` handled two of them (a leading `///`/`//` line
    /// and a `/* */` block); the other two counted as shipping code, so an
    /// adapter that explained its gap in a trailing comment, or a fixture
    /// that carried the token in a string literal, forced a phantom bullet
    /// into `docs/status.md` — or, worse, got the honest prose deleted to
    /// keep the gate green.
    #[test]
    fn a_token_outside_code_is_never_a_shipping_claim() {
        let text = concat!(
            "//! ProviderError::NotImplemented(\"module::doc\")\n",
            "/// ProviderError::NotImplemented(\"item::doc\")\n",
            "// ProviderError::NotImplemented(\"line::comment\")\n",
            "/* ProviderError::NotImplemented(\"block::comment\") */\n",
            "fn f() {\n",
            "    let _ = 1; // ProviderError::NotImplemented(\"trailing::comment\")\n",
            "    let _ = \"ProviderError::NotImplemented(\\\"string::literal\\\")\";\n",
            "    let _ = r#\"ProviderError::NotImplemented(\"raw::string\")\"#;\n",
            "}\n",
        );
        assert_eq!(
            scan_not_implemented(&searchable(text)),
            Vec::<String>::new(),
            "a token that is only ever mentioned in prose or data is not code"
        );
    }

    /// The six shapes that tell a lexer from a pair of `contains` calls.
    /// Each one is a place where the naive reading of the *other* three
    /// states is wrong, and four of them fail in the dangerous direction —
    /// they delete live code, so the gate passes by finding nothing.
    #[test]
    fn the_lexer_tells_the_four_states_apart() {
        // 1. A comment containing a quote opens no string: the code after it
        //    must still be visible. An *odd* number of `"` is the shape that
        //    matters — a lexer that let a comment end at a quote would read
        //    from there to the next `"` in the file as one literal and
        //    swallow the call underneath. The apostrophe rides along, because
        //    a char-literal reader makes the same mistake with `isn't`.
        let text = "// the rope is 6\" long, and it isn't a string\n\
                    Err(ProviderError::NotImplemented(\"after::comment\"))";
        assert_eq!(
            scan_not_implemented(&searchable(text)),
            vec!["after::comment"],
            "a `\"` or a `'` in a comment must not swallow the code after it"
        );

        // 2. A string containing `//` opens no comment: the rest of the line
        //    is code. This is the case the old line-based stripper refused
        //    to risk, and the reason it left trailing comments alone.
        let text = "let u = \"https://example.test\"; \
                    Err(ProviderError::NotImplemented(\"after::url\"))";
        assert_eq!(
            scan_not_implemented(&searchable(text)),
            vec!["after::url"],
            "a URL's `//` is not a comment"
        );

        // 3. `*/` inside a raw string closes no block comment.
        let text = "let r = r#\"a */ b\"#; Err(ProviderError::NotImplemented(\"after::raw\"))";
        assert_eq!(
            scan_not_implemented(&searchable(text)),
            vec!["after::raw"],
            "`*/` inside a raw string closes nothing"
        );

        // 4. A character literal that *is* a quote opens no string.
        let text = "let q = '\"'; Err(ProviderError::NotImplemented(\"after::char\"))";
        assert_eq!(
            scan_not_implemented(&searchable(text)),
            vec!["after::char"],
            "`'\"'` is a character, not a quote"
        );

        // 5. A doc comment ends at its newline; the code under it counts.
        let text = "/// ProviderError::NotImplemented(\"doc::only\")\n\
                    fn f() { Err(ProviderError::NotImplemented(\"real::gap\")) }";
        assert_eq!(
            scan_not_implemented(&searchable(text)),
            vec!["real::gap"],
            "the doc line is prose; the line under it is the claim"
        );

        // 6. Block comments nest, as Rust's do. The token sits in the
        //    *tail* of the outer comment, after the inner one has closed:
        //    a lexer that stopped at the first `*/` would hand that tail to
        //    the scanner as code and report a gap nothing has.
        let text = "/* outer /* inner */ ProviderError::NotImplemented(\"prose::tail\") */ \
                    Err(ProviderError::NotImplemented(\"after::nested\"))";
        assert_eq!(
            scan_not_implemented(&searchable(text)),
            vec!["after::nested"],
            "an inner `*/` closes the inner comment only"
        );
    }

    /// A lifetime is not a character literal. Reading `'static` as one would
    /// swallow everything up to the next `'` in the file — the
    /// delete-too-much failure, which passes the gate silently.
    #[test]
    fn a_lifetime_is_not_a_character_literal() {
        let text = "fn f<'a>(_: &'a str) -> Never { \
                    Err(ProviderError::NotImplemented(\"after::lifetime\")) }";
        assert_eq!(
            scan_not_implemented(&searchable(text)),
            vec!["after::lifetime"]
        );
        assert_eq!(end_of_literal(&"'a>".chars().collect::<Vec<_>>(), 0), None);
        assert_eq!(
            end_of_char_literal(&"'\\''".chars().collect::<Vec<_>>(), 0),
            Some(4)
        );
    }

    /// `#[doc = "…"]` is the attribute spelling of a doc comment, and a token
    /// inside one is the same prose by another syntax.
    #[test]
    fn a_token_in_a_doc_attribute_is_not_a_shipping_claim() {
        let text = "#[doc = r#\"Returns ProviderError::NotImplemented(\"attr::only\") one day.\"#]\n\
                    fn f() {}\n";
        assert!(scan_not_implemented(&searchable(text)).is_empty());
    }

    /// The scanner reads the token out of the literal the call actually
    /// carries, raw or not — and a prefixed literal elsewhere in the line is
    /// still skipped over rather than read.
    #[test]
    fn the_token_is_read_from_the_calls_own_literal() {
        let text = "let b = b\"NotImplemented(\\\"byte::string\\\")\"; \
                    Err(ProviderError::NotImplemented(r\"raw::arg\"))";
        assert_eq!(scan_not_implemented(&searchable(text)), vec!["raw::arg"]);
    }

    /// `verify-docs` counts `///` lines, so its stripper must leave them
    /// alone while the gates' stripper removes them. One lexer, two modes;
    /// this is the test that keeps the modes from collapsing into each other.
    #[test]
    fn only_the_gates_stripper_removes_doc_lines() {
        let text = "/// doc\n/* block */\nfn f() {}\n";
        assert!(strip_block_comments(text).contains("/// doc"));
        assert!(!strip_block_comments(text).contains("block"));
        assert!(!strip_comments(text).contains("doc"));
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
    // eagerly, not vpay_db::connect_lazy — see ADR-0006
    let repositories = vpay_db::connect(&args.database_url).await?; // not connect_lazy
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
            "a comment naming the function — leading or trailing, which the line-based \
             stripper this replaced could not tell apart — and a call under `#[cfg(test)]`, \
             are not calls from a shipping path"
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

#[cfg(test)]
mod sdk_parity_tests {
    use super::*;
    use crate::signing_key_tests::TempDir;

    /// Builds a synthetic two-SDK tree: `sdks/rust` with one live `#[test]`
    /// and one `#[ignore]`d one, `sdks/nodejs` with one `it(...)`.
    ///
    /// A real directory rather than an in-memory index, because the walk
    /// (which extension is read how, which directories are skipped) is half
    /// of what this check does and an in-memory fixture would prove none of
    /// it.
    fn synthetic_sdks(label: &str) -> TempDir {
        let dir = TempDir::new(label);
        let rust = dir.path().join("sdks/rust/tests");
        let node = dir.path().join("sdks/nodejs/src");
        let vendored = dir.path().join("sdks/nodejs/node_modules/other");
        fs::create_dir_all(&rust).expect("the rust fixture directory is creatable");
        fs::create_dir_all(&node).expect("the node fixture directory is creatable");
        fs::create_dir_all(&vendored).expect("the vendored fixture directory is creatable");

        fs::write(
            rust.join("resources.rs"),
            "#[tokio::test]\n\
             async fn a_confirm_reaches_the_rail() {}\n\
             \n\
             /// Doc comment, then an attribute block.\n\
             #[test]\n\
             fn the_body_is_encoded_exactly() {}\n\
             \n\
             #[test]\n\
             #[ignore = \"not implemented: see docs/status.md\"]\n\
             fn a_refund_settles() {}\n\
             \n\
             fn a_plain_helper() {}\n",
        )
        .expect("the rust fixture is writable");

        fs::write(
            node.join("client.test.ts"),
            "describe(\"resources\", () => {\n  \
               it(\"encodes the body exactly\", async () => {});\n  \
               it.skip(\"a refund settles\", async () => {});\n  \
               it(\n    \"is wrapped across two lines\",\n    async () => {},\n  );\n  \
               it(\"authenticates a real \\`stripe\\` client end to end\", () => {});\n  \
               const submit = () => {};\n\
             });\n",
        )
        .expect("the node fixture is writable");

        fs::write(
            vendored.join("vendor.test.ts"),
            "it(\"a vendored dependency's own test\", () => {});\n",
        )
        .expect("the vendored fixture is writable");

        dir
    }

    const HEADER: &str = "| Capability | `sdks/rust` | `sdks/nodejs` |\n|---|---|---|\n";

    fn problems(dir: &TempDir, doc: &str) -> Vec<String> {
        parity_outcome(dir.path(), doc).problems
    }

    #[test]
    fn a_matrix_whose_cells_all_name_tests_that_exist_passes() {
        let dir = synthetic_sdks("parity-pass");
        let doc = format!(
            "{HEADER}\
             | confirm | ✅ `a_confirm_reaches_the_rail` | ✅ `encodes the body exactly` |\n\
             | encoding | ✅ `the_body_is_encoded_exactly` | ✅ `is wrapped across two lines` |\n"
        );
        let outcome = parity_outcome(dir.path(), &doc);
        assert!(outcome.problems.is_empty(), "{:?}", outcome.problems);
        assert_eq!(outcome.proven, 4);
        assert_eq!(outcome.gaps, 0);
    }

    /// The revert-proof property, in a unit test: rename a test in the matrix
    /// and the check names the cell that now lies.
    #[test]
    fn a_tick_naming_a_test_that_does_not_exist_fails_and_names_the_cell() {
        let dir = synthetic_sdks("parity-missing");
        let doc = format!(
            "{HEADER}| confirm | ✅ `a_confirm_reaches_the_railway` | ✅ `encodes the body exactly` |\n"
        );
        let found = problems(&dir, &doc);
        assert_eq!(found.len(), 1, "{found:?}");
        let message = found.first().map(String::as_str).unwrap_or_default();
        assert!(
            message.contains("a_confirm_reaches_the_railway"),
            "{message}"
        );
        assert!(message.contains("confirm"), "{message}");
        assert!(message.contains("sdks/rust"), "{message}");
    }

    #[test]
    fn a_blank_cell_fails() {
        let dir = synthetic_sdks("parity-blank");
        let doc = format!("{HEADER}| confirm | ✅ `a_confirm_reaches_the_rail` |  |\n");
        let found = problems(&dir, &doc);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(
            found
                .first()
                .is_some_and(|m| m.contains("blank") && m.contains("sdks/nodejs")),
            "{found:?}"
        );
    }

    #[test]
    fn a_gap_without_a_date_fails_and_one_with_a_date_passes() {
        let dir = synthetic_sdks("parity-gap");
        let undated = format!(
            "{HEADER}| stripe authenticator | ⛔ no async-stripe equivalent | ✅ `encodes the body exactly` |\n"
        );
        let found = problems(&dir, &undated);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(
            found.first().is_some_and(|m| m.contains("no date")),
            "{found:?}"
        );

        let dated = format!(
            "{HEADER}| stripe authenticator | ⛔ 2026-09-03 — no async-stripe equivalent (owner: SDK maintainers) | ✅ `encodes the body exactly` |\n"
        );
        let outcome = parity_outcome(dir.path(), &dated);
        assert!(outcome.problems.is_empty(), "{:?}", outcome.problems);
        assert_eq!(outcome.gaps, 1);
    }

    #[test]
    fn a_cell_that_is_neither_a_tick_nor_a_gap_fails() {
        let dir = synthetic_sdks("parity-prose");
        let doc =
            format!("{HEADER}| confirm | partly, see below | ✅ `encodes the body exactly` |\n");
        let found = problems(&dir, &doc);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(
            found.first().is_some_and(|m| m.contains("must begin with")),
            "{found:?}"
        );
    }

    #[test]
    fn a_tick_that_names_no_test_at_all_fails() {
        let dir = synthetic_sdks("parity-empty-tick");
        let doc = format!("{HEADER}| confirm | ✅ | ✅ `encodes the body exactly` |\n");
        let found = problems(&dir, &doc);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(
            found.first().is_some_and(|m| m.contains("names no test")),
            "{found:?}"
        );
    }

    /// A document with no matrix must fail rather than pass by checking
    /// nothing — the same failure mode `declared_tokens` guards against when
    /// its heading is renamed away.
    #[test]
    fn a_document_with_no_capability_table_fails_rather_than_passing_vacuously() {
        let dir = synthetic_sdks("parity-no-table");
        let doc = "# Parity\n\nProse only.\n\n| Gap | Owner |\n|---|---|\n| none | nobody |\n";
        let found = problems(&dir, doc);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(
            found.first().is_some_and(|m| m.contains("no table")),
            "{found:?}"
        );
    }

    #[test]
    fn a_column_that_is_not_a_directory_fails() {
        let dir = synthetic_sdks("parity-bad-column");
        let doc = "| Capability | `sdks/kotlin` |\n|---|---|\n| confirm | ✅ `a_confirm_reaches_the_rail` |\n";
        let found = problems(&dir, doc);
        assert!(
            found.iter().any(|m| m.contains("is not a directory")),
            "{found:?}"
        );
    }

    /// An `#[ignore]`d test proves nothing (AGENTS.md rule 2), so a cell may
    /// not cite one — and `it.skip` is the TypeScript spelling of the same
    /// thing.
    #[test]
    fn an_ignored_or_skipped_test_cannot_satisfy_a_tick() {
        let dir = synthetic_sdks("parity-ignored");
        let doc = format!("{HEADER}| refunds | ✅ `a_refund_settles` | ✅ `a refund settles` |\n");
        let found = problems(&dir, &doc);
        assert_eq!(found.len(), 2, "{found:?}");
    }

    /// A vendored dependency's tests are not this SDK's proof.
    #[test]
    fn a_test_inside_node_modules_does_not_satisfy_a_tick() {
        let dir = synthetic_sdks("parity-vendored");
        let doc = format!(
            "{HEADER}| confirm | ✅ `a_confirm_reaches_the_rail` | ✅ `a vendored dependency's own test` |\n"
        );
        let found = problems(&dir, &doc);
        assert_eq!(found.len(), 1, "{found:?}");
    }

    /// A test name containing a backtick is written with a double-backtick
    /// span; `stripe-auth.test.ts` really does carry one.
    #[test]
    fn a_test_name_carrying_a_backtick_is_readable_from_a_double_backtick_span() {
        let dir = synthetic_sdks("parity-backtick");
        let doc = format!(
            "{HEADER}| stripe | ⛔ 2026-09-03 — no equivalent | ✅ ``authenticates a real `stripe` client end to end`` |\n"
        );
        let outcome = parity_outcome(dir.path(), &doc);
        assert!(outcome.problems.is_empty(), "{:?}", outcome.problems);
        assert_eq!(outcome.proven, 1);
    }

    #[test]
    fn a_plain_helper_function_is_not_a_test() {
        let mut names = BTreeSet::new();
        rust_test_names("fn a_plain_helper() {}\n", &mut names);
        assert!(names.is_empty());
    }

    #[test]
    fn a_member_call_and_a_longer_identifier_are_not_test_declarations() {
        let mut names = BTreeSet::new();
        ts_test_names(
            "it.each([1])(\"parameterised\", () => {});\nsubmit(\"not a test\");\nawait(\"neither\");\n",
            &mut names,
        );
        assert!(names.is_empty(), "{names:?}");
    }

    #[test]
    fn the_repositorys_own_matrix_passes() {
        let root = repo_root();
        let doc = fs::read_to_string(root.join(SDK_PARITY_DOC)).expect("the matrix is readable");
        let outcome = parity_outcome(&root, &doc);
        assert!(outcome.problems.is_empty(), "{:#?}", outcome.problems);
        assert!(
            outcome.proven > 50,
            "the matrix should name many tests, named {}",
            outcome.proven
        );
    }
}

#[cfg(test)]
mod link_tests {
    use super::*;
    use crate::signing_key_tests::TempDir;

    /// A throwaway git repository holding `files`, all staged.
    ///
    /// Staged rather than committed: `git ls-files` reads the index, so this
    /// needs no `user.email` and no commit, and the tests drive the real
    /// [`tracked_paths`] rather than a stand-in for it — which is the point,
    /// because "tracked" is the rule the gate turns on.
    fn repo_with(files: &[(&str, &str)]) -> TempDir {
        let dir = TempDir::new("verify-links");
        write_all(dir.path(), files);
        git(dir.path(), &["init", "-q"]);
        git(dir.path(), &["add", "-A"]);
        dir
    }

    fn write_all(root: &Path, files: &[(&str, &str)]) {
        for (path, contents) in files {
            let full = root.join(path);
            if let Some(parent) = full.parent() {
                fs::create_dir_all(parent).expect("the parent directory is creatable");
            }
            fs::write(&full, contents).expect("the file is writable");
        }
    }

    fn git(root: &Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .args(args)
            .current_dir(root)
            .status()
            .expect("git is on PATH — verify-links needs it too");
        assert!(status.success(), "git {args:?} failed");
    }

    /// The decisive negative: one wrong path, and the gate fails naming the
    /// file, the line, what was written and what it resolved to.
    #[test]
    fn one_broken_relative_link_fails_the_gate() {
        let repo = repo_with(&[("docs/a.md", "See [b](b.md).\n")]);
        let error = verify_links(repo.path()).expect_err("b.md is not tracked");
        assert!(
            error.contains("docs/a.md:1: b.md -> docs/b.md"),
            "the message must locate the link and show the resolution: {error}"
        );
    }

    /// The other half of the pair: the same document passes once the file it
    /// names exists, so the failure above is about the target and not about
    /// the parser refusing every link.
    #[test]
    fn the_same_link_passes_once_its_target_is_tracked() {
        let repo = repo_with(&[("docs/a.md", "See [b](b.md).\n"), ("docs/b.md", "# B\n")]);
        assert!(verify_links(repo.path()).is_ok());
    }

    /// A file that exists on this machine and nowhere else does not satisfy a
    /// link. This is what `git ls-files` buys over a directory walk, and it
    /// is the rule that keeps a green run meaningful on a fresh clone.
    #[test]
    fn a_file_that_is_present_but_untracked_does_not_satisfy_a_link() {
        let repo = repo_with(&[("docs/a.md", "See [b](b.md).\n")]);
        write_all(repo.path(), &[("docs/b.md", "# B, never staged\n")]);
        let error = verify_links(repo.path()).expect_err("b.md is on disk but untracked");
        assert!(error.contains("docs/a.md:1: b.md -> docs/b.md"), "{error}");
    }

    #[test]
    fn a_link_inside_a_fenced_block_is_not_a_link() {
        let repo = repo_with(&[(
            "docs/a.md",
            "Prose.\n\n```text\n[gone](gone.md)\n```\n\nMore prose.\n",
        )]);
        assert!(verify_links(repo.path()).is_ok());
    }

    /// A fence's info string may not itself contain a backtick — the clause
    /// that keeps ```` ```` ```ignore ```` ```` (a four-backtick code span, and
    /// how `docs/status.md` line 69 is written) from opening a block that
    /// never closes. Delete it from [`fence_marker`] and this test passes
    /// vacuously while 2 200 of `docs/status.md`'s lines stop being checked.
    #[test]
    fn a_backtick_run_whose_info_string_holds_backticks_opens_no_fence() {
        // Line 1 is `docs/status.md`'s line 69, shape for shape: a
        // four-backtick code span, at the start of a line, whose content is a
        // three-backtick fence.
        let repo = repo_with(&[(
            "docs/a.md",
            "```` ```ignore ```` doctest fences are reported.\n\nSee [gone](gone.md).\n",
        )]);
        let error = verify_links(repo.path()).expect_err("the link after the code span is checked");
        assert!(
            error.contains("docs/a.md:3: gone.md -> docs/gone.md"),
            "{error}"
        );
    }

    /// A closing fence carries no info string, so an inner ```` ```rust ````
    /// does not end the block three lines early and expose code as prose.
    #[test]
    fn an_inner_fence_with_an_info_string_does_not_close_the_block() {
        let repo = repo_with(&[(
            "docs/a.md",
            "````markdown\n```rust\n[gone](gone.md)\n```\n````\n",
        )]);
        assert!(verify_links(repo.path()).is_ok());
    }

    #[test]
    fn a_link_inside_a_code_span_or_an_html_comment_is_not_a_link() {
        let repo = repo_with(&[(
            "docs/a.md",
            "Write it `[gone](gone.md)` like this.\n\n<!-- [also-gone](also-gone.md) -->\n",
        )]);
        assert!(verify_links(repo.path()).is_ok());
    }

    /// An unterminated `<!--` is left as ordinary text rather than masking
    /// the rest of the file: a masker that deletes too much makes this gate
    /// pass by finding nothing.
    #[test]
    fn an_unterminated_html_comment_does_not_swallow_the_document() {
        let repo = repo_with(&[(
            "docs/a.md",
            "<!-- opened and never closed\n\n[gone](gone.md)\n",
        )]);
        assert!(verify_links(repo.path()).is_err());
    }

    #[test]
    fn a_fragment_and_a_line_suffix_resolve_to_the_file_itself() {
        let repo = repo_with(&[
            (
                "docs/a.md",
                "[h](b.md#a-heading) and [src](../src/lib.rs:42) and [col](../src/lib.rs:42:7)\n",
            ),
            ("docs/b.md", "# A heading\n"),
            ("src/lib.rs", "fn main() {}\n"),
        ]);
        assert!(verify_links(repo.path()).is_ok());
    }

    /// The fragment is stripped, never resolved — so a *file* that is gone is
    /// still reported, with the target shown as the author wrote it.
    #[test]
    fn a_fragment_does_not_excuse_a_missing_file() {
        let repo = repo_with(&[("docs/a.md", "[h](gone.md#a-heading)\n")]);
        let error = verify_links(repo.path()).expect_err("gone.md is not tracked");
        assert!(
            error.contains("docs/a.md:1: gone.md#a-heading -> docs/gone.md"),
            "{error}"
        );
    }

    #[test]
    fn a_reference_definition_is_checked() {
        let repo = repo_with(&[(
            "docs/a.md",
            "See [the flow][flow].\n\n[flow]: gone.md \"A title\"\n",
        )]);
        let error = verify_links(repo.path()).expect_err("the definition names a missing file");
        assert!(
            error.contains("docs/a.md:3: gone.md -> docs/gone.md"),
            "{error}"
        );
    }

    #[test]
    fn an_angle_bracketed_target_with_spaces_parses() {
        let repo = repo_with(&[
            ("docs/a.md", "[x](<a file.md>) and [y](<gone file.md>)\n"),
            ("docs/a file.md", "# Spaces\n"),
        ]);
        let error = verify_links(repo.path()).expect_err("the second target is missing");
        assert!(
            error.contains("docs/a.md:1: gone file.md -> docs/gone file.md"),
            "{error}"
        );
        assert!(
            !error.contains("docs/a file.md"),
            "the first target resolves: {error}"
        );
    }

    #[test]
    fn an_image_target_is_checked_like_any_other() {
        let repo = repo_with(&[("docs/a.md", "![a diagram](diagram.svg)\n")]);
        assert!(verify_links(repo.path()).is_err());
    }

    #[test]
    fn a_link_to_a_directory_that_holds_a_tracked_file_resolves() {
        let repo = repo_with(&[
            ("docs/a.md", "See [the runbooks](../docs/runbooks).\n"),
            ("docs/runbooks/one.md", "# One\n"),
        ]);
        assert!(verify_links(repo.path()).is_ok());
    }

    #[test]
    fn a_target_that_climbs_above_the_repository_root_is_reported() {
        let repo = repo_with(&[("docs/a.md", "[out](../../elsewhere.md)\n")]);
        let error = verify_links(repo.path()).expect_err("the target leaves the repository");
        assert!(error.contains("escapes the repository root"), "{error}");
    }

    #[test]
    fn http_mailto_and_bare_anchor_targets_are_skipped() {
        let repo = repo_with(&[(
            "docs/a.md",
            "[a](https://example.invalid/x) [b](HTTP://EXAMPLE.INVALID) \
             [c](mailto:nobody@example.invalid) [d](#a-heading)\n",
        )]);
        assert!(verify_links(repo.path()).is_ok());
    }

    #[test]
    fn a_percent_escaped_target_is_decoded_before_it_is_resolved() {
        let repo = repo_with(&[
            ("docs/a.md", "[x](a%20file.md)\n"),
            ("docs/a file.md", "# Spaces\n"),
        ]);
        assert!(verify_links(repo.path()).is_ok());
    }

    // ---- the parser, driven directly ------------------------------------

    #[test]
    fn a_stray_close_bracket_and_paren_in_prose_is_not_a_link() {
        assert!(doc_links("the closing ]( of a thing\n").is_empty());
    }

    #[test]
    fn a_destination_may_carry_balanced_parentheses_and_a_title() {
        assert_eq!(
            doc_links("[x](docs/a(b).md \"why\")\n"),
            vec![DocLink {
                line: 1,
                target: "docs/a(b).md".to_owned()
            }]
        );
    }

    /// Link *text* may wrap; the line reported is the destination's, which is
    /// the line an author has to edit.
    #[test]
    fn link_text_may_wrap_across_lines() {
        assert_eq!(
            doc_links("see [the long\ntext](docs/a.md)\n"),
            vec![DocLink {
                line: 2,
                target: "docs/a.md".to_owned()
            }]
        );
    }

    /// A destination may not contain a newline, so an unclosed `(` gives up
    /// instead of consuming the paragraph.
    #[test]
    fn a_destination_containing_a_newline_is_not_a_link() {
        assert!(doc_links("[x](docs/\na.md)\n").is_empty());
    }

    #[test]
    fn a_line_suffix_is_stripped_only_when_it_is_digits() {
        assert_eq!(strip_line_suffix("src/lib.rs:42"), "src/lib.rs");
        assert_eq!(strip_line_suffix("src/lib.rs:42:7"), "src/lib.rs");
        assert_eq!(strip_line_suffix("docs/a:b.md"), "docs/a:b.md");
    }

    #[test]
    fn a_root_relative_target_resolves_from_the_repository_root() {
        assert_eq!(
            resolve_against("docs/plans/a.md", "/docs/status.md").as_deref(),
            Some("docs/status.md")
        );
        assert_eq!(
            resolve_against("docs/plans/a.md", "../status.md").as_deref(),
            Some("docs/status.md")
        );
        assert_eq!(resolve_against("a.md", "../../x.md"), None);
    }
}

#[cfg(test)]
mod citation_tests {
    use super::*;

    /// The aliases the extraction is given in these tests. The live name is
    /// asked of `gh` in production; here it is fixed so the patterns can be
    /// proven with no network at all.
    fn aliases() -> Vec<String> {
        REPOSITORY_ALIASES.iter().map(|a| (*a).to_owned()).collect()
    }

    fn kinds_and_ids(text: &str) -> Vec<(CitationKind, String)> {
        citations_in(text, &aliases())
            .into_iter()
            .map(|c| (c.kind, c.id))
            .collect()
    }

    /// The reason the rule is "eleven digits" rather than "the word `run`
    /// followed by digits": this repository writes runs in lists, and a
    /// cue-word rule checks the first and ignores the three places a wrong id
    /// would actually hide.
    #[test]
    fn every_run_id_in_a_list_is_a_citation_not_just_the_first() {
        assert_eq!(
            run_id_citations(
                "Runs `33772512791`, `33784613048`, `33789060270` and `33792230539` are green."
            ),
            vec!["33772512791", "33784613048", "33789060270", "33792230539"]
        );
    }

    #[test]
    fn digits_attached_to_something_else_are_not_a_run_id() {
        // An image tag, a URL path (answered for by the URL rule, which knows
        // *whose* run it is), a twelve-digit number, and a dotted one.
        assert!(run_id_citations("pushed as `:sha-33929374661`").is_empty());
        assert!(
            run_id_citations("https://github.com/marcjazz/authkestra/actions/runs/33929374661")
                .is_empty()
        );
        assert!(run_id_citations("the number 339293746612").is_empty());
        assert!(run_id_citations("version 1.33929374661").is_empty());
    }

    #[test]
    fn a_pull_request_cue_and_an_issue_cue_choose_different_endpoints() {
        assert_eq!(
            kinds_and_ids("Delivered in PR #15 and tracked by Issue #11.\n"),
            vec![
                (CitationKind::Pull, "15".to_owned()),
                (CitationKind::Issue, "11".to_owned()),
            ]
        );
        assert_eq!(
            CitationKind::Pull.api_path("15"),
            "repos/{owner}/{repo}/pulls/15"
        );
        assert_eq!(
            CitationKind::Issue.api_path("11"),
            "repos/{owner}/{repo}/issues/11"
        );
        assert_eq!(
            CitationKind::Run.api_path("33929374661"),
            "repos/{owner}/{repo}/actions/runs/33929374661"
        );
    }

    /// One cue can carry a list. All three separators this tree actually
    /// uses.
    #[test]
    fn a_second_number_continues_the_first_citations_cue() {
        assert_eq!(
            hash_citations("Steps 2–3, PRs #16–#17"),
            vec![
                (CitationKind::Pull, "16".to_owned()),
                (CitationKind::Pull, "17".to_owned())
            ]
        );
        assert_eq!(
            hash_citations("an audit and a CI timeout fix (PRs #23, #24)"),
            vec![
                (CitationKind::Pull, "23".to_owned()),
                (CitationKind::Pull, "24".to_owned())
            ]
        );
        assert_eq!(
            hash_citations("(Steps 0–8 merged, PR #27 and #28)"),
            vec![
                (CitationKind::Pull, "27".to_owned()),
                (CitationKind::Pull, "28".to_owned())
            ]
        );
    }

    /// A cue must not bleed down the line past a word: the second number here
    /// is an example payload in a runbook, not a pull request.
    #[test]
    fn a_cue_does_not_carry_past_intervening_prose() {
        assert_eq!(
            hash_citations("Delivered in PR #20 — then `-d \"description=Order #1234\"`"),
            vec![(CitationKind::Pull, "20".to_owned())]
        );
    }

    /// Every uncued `#n` this tree actually contains, and why none of them is
    /// a citation. If the cue rule is ever loosened, this is the test that
    /// fails first.
    #[test]
    fn an_uncued_hash_number_is_not_a_citation() {
        for line in [
            // Runbook example payloads.
            "        description: \"Order #42 (rush)\"",
            "  -d \"description=Order #1234\"",
            // Commit ordinals in docs/roadmap.md, not pull requests.
            "`237c716` (#1, CLI/env config) → `9e92d02` (#2, kebab-case rename)",
            "**Status.** In progress. Commit `#7`",
            // A numbered question in ADR-0009.
            "`AGENTS.md` open question #4 records, as of that writing:",
        ] {
            assert!(
                hash_citations(line).is_empty(),
                "must not be read as a citation: {line}"
            );
        }
    }

    #[test]
    fn a_hash_attached_to_a_word_is_a_cross_repository_reference_or_not_a_number_at_all() {
        // Someone else's tracker, and a cryptography standard.
        assert!(hash_citations("RFC 9449 DPoP replay tracking, authkestra#291").is_empty());
        assert!(hash_citations("a PEM RSA private key (PKCS#8 or PKCS#1)").is_empty());
    }

    #[test]
    fn a_markdown_heading_and_an_anchor_link_are_not_citations() {
        assert!(hash_citations("### 9 the known flake").is_empty());
        assert!(hash_citations("#9 would be a heading if it were one").is_empty());
        assert!(hash_citations("Was [§9](#9-the-known-flake-a-real-defect)").is_empty());
    }

    #[test]
    fn the_two_word_pull_request_cue_is_recognised() {
        assert_eq!(
            hash_citations("the first green `ci` on this fix's own pull request (#14)"),
            vec![(CitationKind::Pull, "14".to_owned())]
        );
    }

    #[test]
    fn a_url_is_a_citation_only_when_it_names_this_repository() {
        assert_eq!(
            repository_url_citations("https://github.com/vaam-apps/vpay/issues/11", &aliases()),
            vec![(CitationKind::Issue, "11".to_owned())]
        );
        assert_eq!(
            repository_url_citations("https://github.com/vaam-store/vpay/pull/24", &aliases()),
            vec![(CitationKind::Pull, "24".to_owned())]
        );
        assert_eq!(
            repository_url_citations(
                "https://github.com/vymalo/vpay/actions/runs/33929374661",
                &aliases()
            ),
            vec![(CitationKind::Run, "33929374661".to_owned())]
        );
        // Somebody else's repository. Resolving it here asks GitHub the wrong
        // question, and a wrong question with a confident answer is worse
        // than no check.
        assert!(
            repository_url_citations(
                "https://github.com/marcjazz/authkestra/issues/185",
                &aliases()
            )
            .is_empty()
        );
    }

    /// Unlike [`doc_links`], a citation inside a fence or a code span still
    /// counts: the runbooks cite evidence by pasting the command that
    /// produces it, and the ids are almost always inside backticks.
    #[test]
    fn a_citation_inside_a_code_fence_is_still_a_claim() {
        let text = "```console\n$ gh run view 33929374661\n```\n";
        assert_eq!(
            kinds_and_ids(text),
            vec![(CitationKind::Run, "33929374661".to_owned())]
        );
    }

    #[test]
    fn a_citation_carries_the_line_it_was_written_on() {
        let text = "first line\n\nDelivered in PR #15.\n";
        assert_eq!(
            citations_in(text, &aliases()),
            vec![Citation {
                kind: CitationKind::Pull,
                id: "15".to_owned(),
                line: 3,
            }]
        );
    }

    /// The escape hatch is three named pairs of one id, not a marker anyone
    /// can write. All three are mutation records: a document that reports
    /// "substituting `39999999999` leaves every gate green" has to be able to
    /// print the id in order to say so. An exemption for any other id, or for
    /// a document that is not one of those records, fails here and has to
    /// argue for itself in review.
    /// The decisive negative for the property this command's own doc comment
    /// puts first: **it fails rather than skips when `gh` is missing**. Until
    /// 2026-09-05 that was prose with no guard — every other test here is a
    /// pattern test, so replacing the `?` in `verify_citations_via` with
    /// `println!("skipped"); return Ok(())` passed the whole suite. It does
    /// not now.
    #[test]
    fn a_missing_gh_fails_the_gate_rather_than_skipping() {
        let error = verify_citations_via(&repo_root(), "gh-that-is-not-installed")
            .expect_err("a citation nothing resolved is a citation nothing checked");
        assert!(
            error.contains("needs the GitHub CLI and cannot run without it"),
            "the failure must name the missing dependency: {error}"
        );
    }

    /// The other half of never-skipping: a rate limit is an outage, not a
    /// finding. A 404 is the finding this gate reports; a 403 or 429 has to
    /// stop the run, because reporting the rest of the batch as missing sends
    /// somebody to delete claims that are true.
    #[test]
    fn a_refused_request_stops_the_run_rather_than_reporting_a_missing_id() {
        assert_eq!(classify_gh_status("p", 200), Ok(200));
        assert_eq!(classify_gh_status("p", 404), Ok(404));
        for refused in [403u16, 429] {
            let error = classify_gh_status("p", refused).expect_err("an outage is not a finding");
            assert!(error.contains("Nothing was concluded"), "{error}");
        }
        assert!(classify_gh_status("p", 500).is_err());
    }

    #[test]
    fn every_exempt_id_is_the_one_invented_run_in_a_mutation_record() {
        assert_eq!(CITATIONS_THAT_ARE_NOT_CLAIMS.len(), 3);
        for (file, id) in CITATIONS_THAT_ARE_NOT_CLAIMS {
            assert_eq!(id, "39999999999");
            assert!(
                file.starts_with("docs/plans/step9-notes/release-claims")
                    || file == "docs/plans/exp6-notes/opus.md",
                "an exemption outside the mutation records needs its own reasoning: {file}"
            );
        }
    }
}
