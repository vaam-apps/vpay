# AGENTS.md

Instructions for any agent or human contributing to vpay. Read this before
writing code.

If this is your first change here, start with
[CONTRIBUTING.md](CONTRIBUTING.md) — the short version, and the smallest safe
first change. Come back to this file before touching money, persistence, rails,
authentication, public API or wire types, the UI system, or dependencies; every
one of those is covered below and none of it is optional.

---

## The two rules

These are not style preferences. Both are machine-enforced by `just verify`, and
CI runs it.

`just verify` is the gates the `verify` recipe lists in the `justfile`, and
one report. **The recipe is the list; this paragraph is a description of it,
and it has gone stale at nearly every count it has carried** — see below. On
this commit the gates are fourteen (`verify-no-mocks`, `verify-status`,
`verify-errors`, `verify-sdk-parity`, `verify-links`, `verify-npm-scope`,
`check-schema`, `verify-serde`, `verify-repositories`, `verify-toolchain`,
`verify-ui`, `verify-migrations`, `verify-versions`,
`verify-privacy-inventory`) and they fail the build. If that list and the
recipe disagree, the recipe is right: read it, and fix this paragraph in the
same commit. The report
(`verify-docs`) never does — it prints doc-comment volume per crate, in-file
comment volume per crate, the number of `#[doc = include_str!]` modules, the
production functions of 80 lines or more, every ` ```ignore ` doctest
fence and every `#[allow]`/`#[expect]`, and nothing more. Read it; it is not
a gate you can pass or fail.

This paragraph said "three gates" until 2026-09-05, and had been wrong since
`verify-sdk-parity` landed on 2026-09-03; `verify-links` made it wrong by two.
It then said "five" for the rest of that day, because `verify-npm-scope` and
`check-schema` both landed on 2026-09-05 on branches that did not see each
other — the count was corrected to seven where the two met, and `verify-serde`
and `verify-repositories` ([ADR-0016](docs/adr/0016-engineering-standards.md),
2026-09-05) made it nine. `verify-toolchain` makes it ten, later the same
day, out of the review of the 1.95.0 -> 1.98.0 toolchain bump: it fails when
`backends/Dockerfile`'s `FROM rust:` version and `rust-toolchain.toml`'s
`channel` disagree, a mismatch that was measured to pass every other gate.
`verify-migrations` makes it **twelve** on 2026-09-07 (after `verify-ui`, the eleventh, from the exp26 UI revamp the same day), out of issue #76: it
fails when a migration file's SHA-256 no longer matches
`backends/migrations/MANIFEST.sha256`, because `sqlx::migrate!` checksums a
migration's whole bytes and a comment reflowed after the file shipped stops
every database that applied the original from booting — which is what PR #39
did, with every job in CI green.
`verify-versions` makes it **thirteen** on 2026-09-17
([#201](https://github.com/vaam-apps/vpay/pull/201)): every version
release-please owns must agree, and every line it has to rewrite must still
carry its `x-release-please-version` comment. _(This paragraph and the list
above said "twelve" from then until 2026-09-18, wrong from the moment the
recipe grew its thirteenth entry — which is exactly the staleness the first
sentence of this section exists to warn about, earned by the change that added
the warning's newest example.)_ Since 2026-09-18 it also refuses an
`extra-files` entry written as a bare string, which is what destroyed
`deploy/helm/vpay/Chart.yaml` on the first release
([#204](https://github.com/vaam-apps/vpay/pull/204)); § Releasing has the
mechanism. And `verify-privacy-inventory` makes it **fourteen**, the same day
([#187](https://github.com/vaam-apps/vpay/pull/187), issue #144): every
database column the migrations create is classified in
`schemas/privacy-inventory.yaml`, and every classification names a live
column, in both directions.

The two thirteenth gates were written on branches that never saw each other,
exactly as `verify-npm-scope` and `check-schema` were on 2026-09-05, and the
count is reconciled where they meet — here. _(Both sides were stale in their
own way, and the note above is #206's account of `master`'s side. This branch's
side: at `de11c9cf` this paragraph said "thirteen" and the sentence below it
said "Eleven of the thirteen", which was right for a tree that had not seen
#201 and wrong for one that had. `master` at `eb078020` said "twelve" and "Ten
of the twelve". Neither side was right for the merge, which is fourteen and
twelve.)_
Twelve of the fourteen are `cargo xtask` commands; `check-schema` and
`verify-ui` are justfile recipes — the first shells out to the CrateStack CLI,
a binary this workspace does not build, and the second is a handful of
`git grep`s.
There is one more check, `cargo xtask verify-citations` (`just
docs-check-citations`), which is a gate but **not** part of `just verify` or
`just ci`: it needs the network and a GitHub token. Run it when you add or
edit a document that cites a CI run id, a pull request or an issue.

One further gate runs in CI's `web` job and in neither `just verify` nor
`just ci`, so a green local run does not predict it: `just test-storybook`,
which renders every checkout story in a real Chromium and fails on an axe
accessibility violation. It is out of `just ci` because it needs the network
the first time (Playwright fetches a ~115 MB Chromium) and is slow. Run it
before opening a PR that touches a checkout screen, a story or the theme.

_This paragraph said "two further gates" and named `just audit-web` as the
other, and had been wrong since 2026-09-11: issue #103 added `audit-web` to
the `ci` recipe in commit `96ebd20c`, in the same change that lowered it from
`--audit-level=high` to `--audit-level=moderate`. Corrected 2026-09-16.
**`just ci` therefore needs the network**, which the `ci` recipe's own comment
now says out loud — three other recipes justify their exclusion from `just ci`
on the grounds that it runs offline, and that premise no longer holds on its
own._

It is the only thing in this repository that returns a colour-contrast
**verdict** for the screens a payer sees: the jsdom axe suites compute no
colour, and issue #73's Cypress attempt only ever got `incomplete` out of the
real page. It cannot be silently switched off — and, more to the point, it
cannot silently measure the wrong thing:
`frontends/apps/checkout/src/a11y-gate.test.ts` runs in `just test-web`, so in
`just ci`, and fails if the addon goes, if a violation stops failing, if the
preview stops painting the document shell the real page paints, or if
`app/globals.css` stops importing the theme before its other at-rules. That
last one is the defect that made this suite render every story unstyled for
six runs while passing all of them.

### 1. No test doubles in shipping processes

No mock, fake, stub or dummy may be reachable from `vpay-server` — the one
shipping binary since 2026-09-07 (issue #77), in any of its modes (`serve`,
`worker`, `staff`). It was two, `vpay-server` and `vpay-worker-bin`.

- `vpay-testkit`, `wiremock`, `testcontainers`, `mockall`, `fake` may appear
  **only** under `[dev-dependencies]`.
- A stub rail is a **WireMock host in configuration** (`compose.yml`), reached
  over HTTP exactly as a real rail is. It is never a linked implementation or a
  conditionally-compiled variant.
- `cargo xtask verify-no-mocks` fails the build otherwise.

Why: a mock compiled into the server is a code path that exists only outside
production. It diverges silently, and "passes in CI, breaks in prod" becomes
structurally possible.

### 2. Never claim a feature is done when it is not

- Unwritten code returns `ProviderError::NotImplemented("<crate>::<fn>")`. It
  **never** returns a plausible-looking success, an empty list, or a zero.
- Every such token must appear in `docs/status.md`. `cargo xtask verify-status`
  fails the build otherwise — and it fails in both directions.
- Tests for unbuilt behaviour are `#[ignore = "not implemented: … — see
docs/status.md"]`, so a green run never overstates coverage.
- When you finish something, update the status pages in the same commit. A
  status page that lags is worse than none, because people trust it. **Since
  2026-09-11 that is more than one file:** `docs/status.md` is the current-state
  page and carries the declaration this gate reads; the row your change belongs
  to lives on a page under `docs/status/`, and your `just ci` evidence goes on a
  dated page under `docs/status/verification/`. `docs/status.md` § "Where a new
  row goes" says which is which, and
  [docs/status/README.md](docs/status/README.md) is the index.

If you are unsure whether something counts as done: would a test fail if it
broke? If no, it is not done.

---

## Standards

Six engineering standards, written down in
[ADR-0016](docs/adr/0016-engineering-standards.md) with the rationale for each,
what enforces it, and what only a reviewer can judge. Read the ADR once. These
are the parts you apply on **every** change:

1. **Errors** — a library crate's own failures are `thiserror` enums, each with
   one `impl vpay_core::error::Classify`; a layer that consumes several crates
   `#[from]`s them and delegates rather than re-deciding; `anyhow` only in
   `backends/apps/*` `main()` and `[dev-dependencies]`.
   ([ADR-0011](docs/adr/0011-error-modelling.md), `verify-errors`)
2. **Adapters** — a rail is reached only through `ProviderAdapter`, and its
   failures are _mapped_ into `FailureCode`/`ProviderError`, never flattened
   into a `String`. ([ADR-0002](docs/adr/0002-provider-port.md))
3. **serde** — every type deriving `Serialize`/`Deserialize` under
   `backends/crates/*/src` carries `#[serde(rename_all = "snake_case")]`, or
   renames every field/variant itself, or gets a row with a reason in
   ADR-0016's exemption table. Visibility is not part of the rule. `rename_all`
   is a statement about _our_ wire — a rail's casing is the rail's.
   (`verify-serde`; the table is checked in both directions, so a stale
   exemption fails too)
4. **SOLID and DRY** — no gate, and the ADR says so. Ask what would have to
   change for a piece of code to be wrong, and whether that lives in one place.
5. **Repositories** — `vpay-db` exposes traits; every implementation is
   `pub(crate)` and reached through `vpay_db::connect` or a function returning
   `impl Trait`. Name the trait, never `PgRepositories` or a `Sql…Store`.
   (`verify-repositories`)
6. **Docs** — an example in a doc comment is compiled (`just test-doc`); never
   reach for ` ```ignore ` to make one build. Long reasoning goes in
   `docs/reference/<crate>.md`, not in an 80-line module header; a module doc
   that _is_ a document uses `#[doc = include_str!("…md")]`. Prefer one more
   `///` explaining why over one more `//` restating what.

**The migration rule: existing code is migrated as it is touched; a new crate
complies from its first commit.** Do not open a sweep. Do not, in particular,
add an exemption row to make a gate pass on code you did not have to touch.

---

## Architecture rules

**Rails live behind the port.** `if provider == "mtn_momo"` outside
`backends/crates/vpay-adapter-*` is a defect. Branch on capability _values_
(`flow`, `supports_refunds`), never on a provider code. ([ADR-0002](docs/adr/0002-provider-port.md))

**No environment branching.** No `if (sandbox)`, no `NODE_ENV` check, no
profile-selected bean. A profile selects a _config file_, never a _code path_.
Sandbox and production are two deployments of the same image.
([ADR-0003](docs/adr/0003-yaml-configuration.md))

**Money is integer minor units.** XAF is zero-decimal: `5000` means 5,000 FCFA.
Floating-point arithmetic is denied workspace-wide. One conversion function,
`Money::to_provider_string`. ([docs/flows/money.md](docs/flows/money.md))

**Never let a payer act on a transaction you cannot name.** Push rails: persist
the reference before submitting. Redirect rails: persist the rail's token before
redirecting. ([docs/flows/crash-safety.md](docs/flows/crash-safety.md))

**One charge per intent, forever.** Enforced by a plain unique index. Retry
means a new PaymentIntent.

**Callbacks are hints.** `parse_callback` returns identifiers only, never a
status. The authenticated status query is the only thing that moves money.

---

## Rust conventions

- Edition 2024, resolver 3. MSRV in `Cargo.toml`; toolchain pinned in
  `rust-toolchain.toml`.
- `unwrap`, `expect`, `panic`, `todo`, `unimplemented`, float arithmetic: denied
  in production code. Tests are exempt via `clippy.toml`. `unsafe` is forbidden.
  ([ADR-0007](docs/adr/0007-lint-policy.md))
- Errors are typed at the leaves, composed per layer, classified once
  ([ADR-0011](docs/adr/0011-error-modelling.md),
  [docs/flows/errors.md](docs/flows/errors.md)). A library crate defines closed
  `thiserror` enums for its own concerns and each one implements
  `vpay_core::error::Classify`; a layer that consumes several crates defines a
  composite that `#[from]`s the leaves and _delegates_ its classification
  rather than re-deciding it; `anyhow` appears only at the boundary — in
  `backends/apps/*` `main()`, and in `[dev-dependencies]`. Status, retry
  policy, log severity and exit code are all derived from `Category`, never
  chosen at a call site. `cargo xtask verify-errors` fails the build if a `pub`
  `…Error`/`…Rejection` type in `backends/crates` has no `impl Classify`, or if
  a library crate lists `anyhow` under `[dependencies]`.
- TLS: rustls only. `openssl`, `openssl-sys` and `native-tls` are banned in
  `deny.toml`; do not add a dependency that needs them without a new ADR.
- Doc comments on every public item, explaining _why_, not restating the name.
  An example in one is compiled and run — `just test-doc` (`cargo test --doc
--workspace`) is part of `just ci` and of CI's `rust` job, because
  `cargo nextest` runs no doctests, so until 2026-09-03 not one of them had
  ever been compiled by CI. Do not reach for ` ```ignore ` or ` ```no_run `
  to make an example compile: an example nothing runs is a claim nothing
  checks. Use ` ```text ` if it is not Rust, and otherwise make it real.
- The reasoning behind a piece of code goes in `docs/reference/<crate>.md`, not
  in an 80-line module header. One paragraph of what and why plus a link;
  `# Errors`, `# Panics` and `# Examples` stay in the source.

## TypeScript conventions

- TS strict, plus `noUncheckedIndexedAccess` and `exactOptionalPropertyTypes`.
- Components: `class-variance-authority` for variants, `@base-ui/react` for
  behaviour, daisyUI 5 on Tailwind 4 for tokens (theme `bumblebee`). Do not
  hand-roll a component `@base-ui/react` already solves. **Corrected
  2026-09-07:** this line named Headless UI, framer-motion and vaul, none of
  which is a dependency of any `package.json` in this repository, and no
  motion or sheet library is. `just verify-ui` is the gate on the daisyUI
  half. **Corrected again, 2026-09-12:** `frontends/packages/ui` (`@vpay/ui`)
  was deleted — both apps now compose the published `@vaam-apps/ui` instead.
  `@base-ui/react` leaves the repository entirely with it; `@vaam-apps/ui`'s
  behaviour comes from Headless UI (`@headlessui/react`) and Radix
  (`@radix-ui/react-dialog`), and its one theme registers under daisyUI's
  built-in name `dark`, not `bumblebee` — `frontends/apps/checkout` keeps a
  runtime brand-colour retarget on top of it (`src/config/theme.ts`), the
  dashboard does not. `class-variance-authority` remains a real dependency
  (of `@vaam-apps/ui` itself) but neither app calls it directly any more.
- Status colour and copy come from `@vpay/tokens`. Never inline a status colour
  in a component — a status must not be green in one view and grey in another.
  **Extended 2026-09-12:** this is now machine-enforced for the first time —
  `just verify-ui`'s check 7a-ii refuses any of `@vaam-apps/ui`'s
  `text-state-<hue>-fg`/`bg-state-<hue>-bg`/`border-state-<hue>-border`/
  `text-destructive` tokens written directly in an app; status presentation
  goes through `defineStatusSystem` (`StatusPill`/`StateChip`) or an
  `InlineBanner` variant, both of which carry the hue for the caller.
- The dashboard never holds a merchant API key. It calls `/dash/v1` server-side
  under an OIDC session. ([ADR-0008](docs/adr/0008-dashboard-scope.md))

## Testing

| Layer               | Tool                                 | Where                        |
| ------------------- | ------------------------------------ | ---------------------------- |
| Rust unit           | `cargo nextest`                      | alongside the code           |
| Rust doctests       | `cargo test --doc` (`just test-doc`) | in `///` examples            |
| Rust integration    | testcontainers                       | `backends/tests/integration` |
| Adapter conformance | shared suite                         | `backends/tests/conformance` |
| TS unit             | vitest                               | alongside the code           |
| Browser e2e         | Cypress against `compose.e2e.yml`    | `frontends/tests/e2e`        |

**The conformance suite is one suite, parameterised over every adapter.** Adding
a rail means making it pass — not writing a new suite. If you find yourself
writing rail-specific conformance tests, the port leaked.

**`just test-rust` needs one thing on macOS that it does not need on Linux.**
Four cases in `backends/tests/integration/tests/staff_sign_in.rs` simulate
distinct client source addresses by binding them, and macOS assigns only
`127.0.0.1` to `lo0` where Linux assigns the whole `127.0.0.0/8` to `lo`. Run
`just loopback-aliases` once per boot; CI is unaffected, and the suite now says
so itself rather than failing with `os error 49` (2026-09-18). The full note is
in [CLAUDE.md](CLAUDE.md) § "Things that will waste your time".

Do not stub inside the browser in Cypress. The rails are stubbed at the
infrastructure layer, so the app under test is the app that ships.

## Documentation

Every flow and process gets a document in `docs/flows/`, answering: what
happens, in what order, what can go wrong, and what invariant holds throughout.
Each ends with a **Status** section stating what is actually built.

Two things about a document are machine-checked, since 2026-09-05. Every
relative link in a tracked `*.md` must resolve to a **tracked** file or
directory (`cargo xtask verify-links`, in `just verify`), so a link satisfied
by an untracked scratch file fails rather than passing on your machine alone.
And every CI run id, pull request and issue a document cites as evidence must
exist (`cargo xtask verify-citations`, opt-in because it needs the network).
A citation that does not resolve is a false claim: strike it through with a
dated correction. Do not replace it with an id you have not checked.

**`verify-links` checks a destination path and never a `#anchor`.** A link to a
heading that has been renamed, or that moved to another page, still passes the
build. So re-read the anchors pointing into a document whose heading you rename.
A sweep on 2026-09-11 found **four** stale ones across the tree — three left
behind by heading renames on 2026-09-06 and 2026-09-07, and one naming a section
that does not exist. Three are fixed; the fourth is named in
[docs/README.md](docs/README.md) rather than quietly left.

**A document that has outgrown one sitting becomes an overview and a
directory**, not a shorter document. Eleven did on 2026-09-11: the page keeps
its own path and its own **Status** section, and indexes pages carrying what was
moved there **verbatim** — no dated measurement, struck-through claim or "this
said X until date Y and was wrong" may be dropped or paraphrased in the move,
because those are what make the page worth trusting.
[docs/README.md](docs/README.md) lists which pages, and what they were.

- A decision that has been made → an ADR (immutable; supersede, never edit).
- A process → a flow doc, as above.
- Why a piece of code is shaped the way it is → `docs/reference/<crate>.md`.
- A proposal under discussion → an RFC.
- Something an on-call person must do → a runbook.
- What an **agent** must know before it changes this repository → a skill in
  [vaam-apps/vpay-skills](https://github.com/vaam-apps/vpay-skills), below.

### Docs↔skills parity

**A feature lands in three places or it has not landed: the code, the docs, and
the skills.**

The agent skills in [vaam-apps/vpay-skills](https://github.com/vaam-apps/vpay-skills)
are a sixth documentation tier with a different audience. A flow doc describes a
process to a reader who will decide what to do. A skill briefs an agent that is
already doing it, and is therefore judged on a different question: not "is this
accurate and complete" but "would an agent that read only this do the right
thing on its first attempt". That is why they are a separate repository — a
briefing that has to clear fourteen gates to be corrected is a briefing nobody
corrects — and why drift between them and this tree is gated rather than
trusted.

The same reasoning as rule 2. A status page that lags is worse than none,
because people trust it; a skill that lags is worse still, because an agent does
not merely trust it — it acts on it, at machine speed, in every session that
loads it.

Change a skill in the same piece of work when your change:

| The change here                     | The skill there                                          |
| ----------------------------------- | -------------------------------------------------------- |
| A new or deleted `docs/flows/` page | Whichever skill claims it in `coverage.json`             |
| A route mounted or unmounted        | `vpay-merchant-api` or `vpay-dashboard`                  |
| A `NotImplemented` token retired    | Every skill that described it as unbuilt — grep for it   |
| A new gate, or one that changed     | `vpay-tooling`; `vpay-troubleshooting` if it fails oddly |
| A toolchain pin bumped              | `vpay-tooling`                                           |
| An SDK capability added             | `vpay-sdks`, beside the `docs/sdks/parity.md` row        |
| A path renamed or moved             | Whatever `verify-coverage` names                         |

`vpay-skills`' `node tools/verify-coverage.mjs <path-to-vpay>` fails in **both**
directions — a `docs/flows/` page no skill covers, and a path a skill claims
that no longer exists here — and its CI runs against this repository's `master`
daily. So a merge that outruns the skills surfaces there as a red build rather
than as a confidently wrong agent three weeks later. Do not leave it to the
cron: open the `vpay-skills` PR alongside yours and link them.

This repository dogfoods the skills. They install into `.agents/skills/` and pin
in `skills-lock.json`, the same mechanism `vaam-ui` already uses:

```bash
npx skills add https://github.com/vaam-apps/vpay-skills --skill vpay
```

## Commits and PRs

- Conventional commits (`feat:`, `fix:`, `docs:`, `chore:`, `refactor:`) — and
  since release-please landed, the **pull request title** is the one that
  matters, because this repository squash-merges with `PR_TITLE` and that
  title becomes the commit subject on `master`. `.github/workflows/pr-title.yml`
  enforces it. `wip:` stays fine on a commit inside your branch; it is not a
  PR title.
- A PR that changes behaviour updates the status pages and the relevant flow doc
  in the same PR. `docs/status.md` § "Where a new row goes" names the page for
  each kind of change; [docs/README.md](docs/README.md) is the index of the
  whole documentation tree.
- A PR that changes behaviour an agent has to know about opens its companion PR
  against [vaam-apps/vpay-skills](https://github.com/vaam-apps/vpay-skills) and
  links the two. See § "Docs↔skills parity" above for which skill.
- `just ci` must pass locally before review.

## Releasing

Nobody hand-edits a version. Every merge to `master` updates a standing
`chore: release X.Y.Z` pull request; merging it creates the `vX.Y.Z` tag, which
is what `release.yml`'s `type=semver` path triggers on — a path that, until
this existed, had never once been taken.

**The trap, if you are ever tempted to bump a version by hand.** `deny.toml`'s
`[bans] wildcards = "deny"` forces every internal Cargo dependency to carry a
`version = "X.Y.Z"` beside its `path`. A bare `"0.1.0"` is `^0.1.0`, and a 0.x
caret range does **not** cross a minor boundary — so moving
`[workspace.package].version` to `0.2.0` while those stay behind does not look
untidy, it fails to resolve:

```
error: failed to select a version for the requirement `vpay-core = "^0.1.0"`
candidate versions found which didn't match: 0.2.0
```

There are **fourteen** such pins: eleven in the root manifest, and three more in
`vpay-api`, `vpay-worker` and `backends/tests/integration` that were found only
by running `cargo metadata`, not by reading. All eighteen version lines this
repository owns carry an `x-release-please-version` comment, and
`just verify-versions` (in `just ci`, via `verify`) fails if any is missing —
including on a _new_ internal dependency, which is the realistic way this gets
armed for the next person.

Deliberately not bumped, each for a stated reason: `Chart.yaml`'s own
`version:` (the chart's separate lifecycle — its comment says "bumped by hand",
and it is already ahead of the app), every `0.0.0` private package, the Flutter
plugin's podspec/gradle boilerplate (`0.0.1` / `1.0-SNAPSHOT`, never wired to
`pubspec.yaml` and already inconsistent with each other), and the Flutter
example's `pubspec.lock` (nothing enforces it; `flutter pub get` rewrites it).

**Added 2026-09-19: publishing the chart makes that hand-bump load-bearing.**
`release.yml`'s `publish-chart` job pushes `deploy/helm/vpay` to
`oci://ghcr.io/vaam-apps/charts/vpay`, and `helm push` derives the artifact's
tag from `Chart.yaml`'s `version:` directly — there is no second name, and no
release-please output, that could stand in for it. A release that forgets to
bump it no longer passes quietly: the job's own guard checks whether that
version is already published before pushing and fails the release if it is,
rather than silently overwriting a chart people already have (`helm push`
overwrites an existing OCI tag with no warning). See
[docs/flows/deployment.md](docs/flows/deployment.md) §2a.

**The first tag.** `.release-please-manifest.json` seeds `0.1.0` — what every
manifest already says while unreleased — so the next release is `0.1.1` or
`0.2.0`, _not_ `0.1.0`. To make the first tag exactly `v0.1.0`, put
`Release-As: 0.1.0` in a commit footer; release-please honours it. That is a
maintainer's call.

**Setup this needs once**: a GitHub App with `contents: write` and
`pull-requests: write` on this repository, its id and private key stored as
`RELEASE_PLEASE_APP_CLIENT_ID` — the App's **Client ID** (`Iv23li…`), not its
numeric App ID; `actions/create-github-app-token` deprecated the `app-id` input
in favour of `client-id`, and the two are different values on the same settings
page — and `RELEASE_PLEASE_APP_PRIVATE_KEY`. Not optional and
not a fallback: GitHub raises no workflow events for anything done with the
default `GITHUB_TOKEN`, so with it the tag would be created and `release.yml`
would never run — no image built, none signed, and nothing failing to say so.

**A bare-string `extra-files` entry is a trap, and it cost this repo its
Chart.yaml once.** release-please does not give a bare string the
annotation-only Generic updater — `base.ts` infers an updater from the file
extension, and `.yaml`/`.yml` gets
`CompositeUpdater(GenericYaml('$.version'), Generic)`. `GenericYaml` reparses
and re-serialises the document. On the **v0.1.1** release that turned
`deploy/helm/vpay/Chart.yaml` from 48 lines into 13 — every comment gone,
including the one explaining that `version:` is the chart's own hand-bumped
lifecycle — then set that `version:` from 0.2.0 to 0.1.1 (a downgrade) because
`$.version` is the top-level key, and left `appVersion` untouched because the
`x-release-please-version` annotation had just been serialised away.
`sdks/flutter/.../pubspec.yaml` lost its comments the same way.

`vsms` escaped only by luck: its two `.yaml` extra-files are compose files,
which have no top-level `version:` key, so `GenericYaml` found nothing to
change and left them alone.

Every entry is therefore written as `{"type": "generic", "path": …}`, which
routes to release-please's `case 'generic'` and runs the Generic updater
alone. `cargo xtask verify-versions` **refuses** a bare string outright, naming
this incident, so the next `.yaml` file added here cannot repeat it.

**Known gap**: merge commits are still enabled, and `merge_commit_title` is
`MERGE_MESSAGE`, so a PR merged that way lands as `Merge pull request #N …`,
which release-please ignores; its individual commit subjects are what count.
Squash is the path `pr-title.yml` actually covers.

## Before you open a PR

```bash
just fmt
just ci
```

Then ask yourself the one question this repo cares about most: **does anything I
wrote imply something works that I have not actually seen work?** If so, fix the
claim, not just the code.
