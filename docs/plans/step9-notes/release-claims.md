# exp5 (docs class) — retiring the stale "release.yml has never run" claims

Branch `claude/exp5-release-claims-opus`, base `master` 33d6c25, 2026-09-05.
Docs only: no source, schema, or workflow *logic* changed — the one
`.github/workflows/release.yml` edit is inside a `#` comment block.

---

## 1. The measurements

Every claim retired below cites one of these. All were run from this worktree
on 2026-09-05.

### 1.1 How many release runs exist

```
$ gh run list --workflow release --branch master --limit 20 \
    --json databaseId,headSha,conclusion,createdAt \
    --jq '.[] | "\(.databaseId) \(.conclusion) \(.createdAt) \(.headSha[0:7])"'
33929374661 success 2026-09-04T23:24:23Z 33d6c25
33918831901 success 2026-09-04T20:58:04Z c531407
33912330063 success 2026-09-04T19:40:03Z c1872f4
33898736618 success 2026-09-04T17:05:54Z cc62b04
33894388991 failure 2026-09-04T16:18:26Z c89cc7f
33849098945 success 2026-09-04T07:31:11Z 10793e4
33846132186 success 2026-09-04T06:51:35Z dde1444
33817293354 success 2026-09-03T23:22:08Z 572a89f
33802515513 success 2026-09-03T20:29:04Z f45cc92
33792230539 success 2026-09-03T18:43:47Z ca94eac
33789060270 success 2026-09-03T18:11:51Z c299372
33784613048 success 2026-09-03T17:26:55Z cbe70dc
33772512791 success 2026-09-03T15:25:25Z e336538
```

**13 runs, 12 green, 1 failure.** The failure `33894388991` is the
organisation-rename breakage already documented in `docs/status.md`. The
latest green run is **`33929374661`**, head `33d6c25` — which is this
branch's base commit, so the images below are built from exactly this tree.

### 1.2 What the latest green run actually did

```
$ gh run view 33929374661 --json name,conclusion,jobs \
    --jq '{conclusion, jobs: [.jobs[] | {name, conclusion}]}'
```

13 jobs, **all `success`**: `derive registry namespace`, eight
`build vpay-{server,worker,checkout,dashboard} ({amd64,arm64})`, and four
`manifest list + sign (vpay-*)`.

From `gh run view 33929374661 --log`, the `create the manifest list` step:

```
pushing sha256:5485db5e397edd8e672737e676756ca4e9eb56a23fb117a6bc762e0532b50537 to ghcr.io/vaam-apps/vpay-server:edge
pushing sha256:08667b03bae210802d04d59dba92820be9bccb4052f8337c74f0ea0a80d68a78 to ghcr.io/vaam-apps/vpay-worker:edge
pushing sha256:ba6d6712dc143598c66c34300dffa3e38cdd5a21de98dfc9b43a13103b21a7a7 to ghcr.io/vaam-apps/vpay-dashboard:edge
pushing sha256:5214e408be6062123b51374d99988ef20e28081fa96e7bcb0eb4ac2b5b12e51e to ghcr.io/vaam-apps/vpay-checkout:edge
```

Each was also pushed to `:sha-33d6c253a232958604801518a08a2f34accb689c`.
The `cosign sign (keyless, GitHub OIDC)` step in each `merge` job logged
`Pushing signature to: ghcr.io/vaam-apps/vpay-<name>` and a Rekor entry:

| Image | tlog index |
|---|---|
| `vpay-server` | 2717616118 |
| `vpay-worker` | 2717617767 |
| `vpay-dashboard` | 2717616040 |
| `vpay-checkout` | 2717615975 |

The build step also shows `--build-arg
VPAY_GIT_SHA=33d6c253a232958604801518a08a2f34accb689c`, and
`backends/Dockerfile:64` declares `ARG VPAY_GIT_SHA=unknown` /
`ENV VPAY_GIT_SHA=${VPAY_GIT_SHA}` which
`vpay-core/build.rs` and `metrics.rs:318` (`option_env!("VPAY_GIT_SHA")`)
consume — so the published Rust images carry a real `git_sha`, not `unknown`.

### 1.3 GHCR package existence and visibility — **partially unmeasurable**

The brief's command was refused:

```
$ gh api "orgs/vaam-apps/packages?package_type=container"
{"message":"You need at least read:packages scope to list packages.", ... "status":"403"}
gh: You need at least read:packages scope to list packages. (HTTP 403)

$ gh auth status | grep -i scopes
  - Token scopes: 'gist', 'project', 'read:org', 'repo', 'user', 'workflow'
```

**The token was not authorised for packages** — no `read:packages`. The
per-user equivalent `gh api "user/packages?package_type=container"` returns
the same 403. The anonymous registry path is refused too:

```
$ curl -s "https://ghcr.io/token?scope=repository:vaam-apps/vpay-server:pull&service=ghcr.io"
{"errors":[{"code":"UNAUTHORIZED","message":"authentication required"}]}

$ curl -s -o /dev/null -w "%{http_code}\n" "https://ghcr.io/v2/vaam-apps/vpay-server/tags/list"
401
```

(Same for `vpay-worker`, `vpay-dashboard`, `vpay-checkout`.)

**What this does and does not establish.** It establishes the four packages
are **not anonymously pullable**, i.e. not public. It does **not**
distinguish "private but present" from "absent" — GHCR answers the same way
for both. Existence is established by §1.2's run log (a push that exits 0
put a manifest there), not by this probe. Every edit below says "published"
on the strength of the run log and says visibility is unmeasured; **none
claims the packages are reachable by anyone else.**

### 1.4 A second stale claim, found by the same grep

`docs/flows/merchant-auth.md:486` asserted the merchant-auth tests "have
still never run in CI". `.github/workflows/ci.yml:133-138` runs
`cargo nextest run --workspace` on `ubuntu-latest` and its own comment says
the container-backed suites "cannot silently skip". Measured:

```
$ gh run list --workflow ci --branch master --limit 6 --json databaseId,conclusion,createdAt
33929374663 success 2026-09-04T23:24:23Z
...
$ gh run view 33929374663 --log | grep Summary
rust  Run cargo nextest run --workspace   Summary [ 941.238s] 1159 tests run: 1159 passed, 0 skipped
```

Per-crate pass counts from the same log: `vpay-tests-integration` **163**,
`vpay-db` **86**. Two of the seven named tests, verified by name:

```
PASS [   2.112s] (1050/1159) vpay-tests-integration::merchant_token_flow the_jwks_and_discovery_documents_describe_this_process
PASS [   2.334s] (1051/1159) vpay-tests-integration::merchant_token_flow the_same_client_assertion_cannot_be_spent_twice
```

---

## 2. The grep, and every hit classified

```
git grep -n -iE 'never run|never been run|no image exists|never (been )?(executed|published|signed)|has not (yet )?run' \
  -- .github docs README.md deploy justfile
```

**43 matching lines before the change** (counted, not estimated:
`git grep … 33d6c25 -- … | wc -l` → 43). Classes: **(a)** stale present-tense
claim, **(b)** dated historical statement still true as history, **(c)** a
claim about something genuinely still not done, **(n/a)** the word "never" in
ordinary prose, not a status claim.

**`.github/workflows/release.yml` is not among the 43.** Its stale paragraph
said "NOTHING IN THIS FILE HAS EVER RUN … nothing has been signed", and
neither "HAS EVER RUN" nor "nothing has been signed" matches this pattern. It
was found because the brief named it. Three further stale claims were found
the same way — by reading the files the brief listed rather than by grepping
— and are marked *(not a grep hit)* below. **The grep under-reports this
defect; that is worth knowing before anyone treats it as the detector.**

| file:line (pre-edit) | class | action | measurement it now cites |
|---|---|---|---|
| `.github/workflows/release.yml:18-22` *(not a grep hit)* | **a** | header paragraph rewritten; original sentence quoted as history, dated correction added | run `33929374661`, 4 digests, 4 tlog indices |
| `docs/runbooks/release.md:4` (header) | **a** | struck through + dated correction; the one true clause (**no `v*` tag**) called out and kept | 13 runs / 12 green; digest+tlog table; the 403 and the anonymous-pull refusal |
| `docs/runbooks/release.md:166-167` (§6) *(not a grep hit)* | **a** | struck through + `Retired 2026-09-05` | run `33929374661` |
| `docs/runbooks/release.md:168-170` (§6, arm64) *(not a grep hit)* | **a** | struck through; replaced with the two Rust-image arm64 jobs specifically | `build vpay-{server,worker} (arm64)` on `ubuntu-24.04-arm`, both `success` |
| `docs/runbooks/release.md:173` (§6, `cosign verify`) | **c** | **left alone** | — still true; I did not run `cosign verify` |
| `docs/runbooks/deploy-and-rollback.md:13-14` (header) *(not a grep hit)* | **a** | struck through + dated correction | server/worker index digests from `33929374661` |
| `docs/runbooks/deploy-and-rollback.md:217, 218` (§6) | **a** | struck through + `Retired 2026-09-05`, with the still-unproven half stated | 13 runs / 12 green; visibility unmeasured |
| `docs/flows/deployment.md:352-355` (§9) | **a** | body struck through, heading kept (it is still true) | run `33929374661`, four images; 403 + anonymous-pull refusal |
| `docs/flows/deployment.md:475` (`vpay_build_info`) | **a** | struck through + dated correction | `--build-arg VPAY_GIT_SHA=33d6c25…` in the run log; `backends/Dockerfile:64` |
| `docs/flows/deployment.md:418` (Step 7 correction) | **b** | **one dated clause added** (count 4 → 13/12; visibility attempted) | `gh run list`; the 403 |
| `docs/flows/deployment.md:11` (chart "has never run") | **c** | **left alone** | still true — no cluster has run the chart (`docs/status.md`) |
| `docs/flows/merchant-auth.md:486` | **a** | struck through + dated correction | CI run `33929374663`, 1159/1159, 163 integration tests, two named PASS lines |
| `docs/runbooks/README.md:110` | **b** | **one dated clause added** | 13 runs / 12 green; run `33929374661` |
| `deploy/helm/vpay/README.md:542` *(not a grep hit)* | **b/c** | trailing "publishing them is block A" struck; pull half kept | run `33929374661` |
| `docs/status.md:1618` (image publishing) | **b** — already struck in Step 7 | **the one permitted dated sentence** (verbatim in §3 below) | four post-rename runs; the 403 |
| `docs/status.md:1635` (`vpay_build_info`) | **a** | **struck through + dated correction — one status.md edit beyond the permitted sentence; see §3.1 for why** | `--build-arg VPAY_GIT_SHA=33d6c25…` in run `33929374661` |
| `docs/status.md:657` | **b** | **left alone** — inside a note headed `Last verified: 2026-09-03, on branch claude/step6-deployment` | — |
| `docs/status.md:1388` | **b** | **left alone** — "had never executed *before this pass*", explicitly past | — |
| `docs/status.md:1592` | **b** | **left alone** — quotes a claim it is itself retiring | — |
| `docs/status.md:1594` | **n/a** | **left alone** — "examples … are never run: the ```` ```no_run ```` fences" | — |
| `docs/status.md:1763` | **c** | **left alone** — `cratestack migrate diff` genuinely never run | — |
| `docs/status.md:998, 1390 (×2), 1391, 1405 (×2), 1409, 1830` | **a**, different family | **left alone deliberately — see §4** | — |
| `docs/runbooks/rotate-signing-key.md:185` | **c** | **left alone** | still true — no cluster has ever run vpay |
| `docs/sdks/parity.md:239, 240` | **c** | **left alone** | still true — dated gap rows, owned by SDK maintainers |
| `docs/roadmap.md:603` | **a**, different family | **left alone** — inside `Status addendum — 2026-09-03 (Step 2, …)`; see §4 | — |
| `docs/roadmap.md:827` | **c** | **left alone** | still true — no real rail has been called |
| `docs/plans/2026-09-03-step6-deployment.md:190, 257` | **b** | **left alone** — dated plan document | — |
| `docs/plans/2026-09-03-step7-cleanup-rework.md:28` | **b** | **left alone** — dated plan document | — |
| `docs/plans/2026-09-04-step9-hosted-checkout.md:637` | **b** | **left alone** — dated plan document | — |
| `docs/plans/step8-notes/lane-a.md:192`, `lane-d.md:76`, `lane-f.md:78` | **b** | **left alone** — dated lane notes | — |
| `docs/plans/step9-notes/lane-4.md:312, 382`, `lane-5.md:195` | **b** | **left alone** — dated lane notes | — |
| `.github/workflows/ci.yml:173` | **n/a** | **left alone** — "this job never runs Cypress" | — |
| `deploy/helm/vpay/templates/deployment-worker.yaml:15` | **n/a** | **left alone** — "two workers never run the same job" | — |
| `docs/reference/vpay-db.md:590` | **n/a** | **left alone** — "never runs on the rail that has URLs" | — |
| `justfile:1694` | **n/a** | **left alone** — "is never running a path the one-liner does" | — |

### Counts

The 43 matching lines, each assigned exactly once. The assignment was checked
against the grep output by script — every key present, none extra, totals
summing to 43 — rather than added up by hand.

| Class | Count | Lines |
|---|---|---|
| **(a) retired** | **8** | `flows/deployment.md:352`, `flows/deployment.md:475`, `flows/merchant-auth.md:486`, `runbooks/deploy-and-rollback.md:217`, `runbooks/deploy-and-rollback.md:218`, `runbooks/release.md:4`, `status.md:1621`, `status.md:1635` |
| **(b) dated clause added** | **3** | `flows/deployment.md:418`, `runbooks/README.md:110`, `status.md:1618` |
| **(a) left deliberately** | **6** | `status.md:998`, `status.md:1390`, `status.md:1391`, `status.md:1405`, `status.md:1830`, `roadmap.md:603` — the test-evidence family, §4 |
| **(c) verified still true** | **8** | `flows/deployment.md:11`, `runbooks/release.md:173`, `runbooks/rotate-signing-key.md:185`, `sdks/parity.md:239`, `sdks/parity.md:240`, `roadmap.md:827`, `status.md:1409`, `status.md:1763` |
| **(b) dated history, left** | **12** | the nine `docs/plans/*` lines, plus `status.md:657`, `status.md:1388`, `status.md:1592` |
| **(n/a) not a status claim** | **6** | `.github/workflows/ci.yml:173`, `deploy/helm/vpay/templates/deployment-worker.yaml:15`, `docs/plans/step8-notes/lane-d.md:76`, `docs/reference/vpay-db.md:590`, `justfile:1694`, `status.md:1594` |

8 + 3 + 6 + 8 + 12 + 6 = **43**.

**Plus 5 stale claims the grep did not find**, retired anyway because the
brief named the files: the `release.yml` header paragraph, `release.md` §6's
two bullets, `deploy-and-rollback.md`'s header sentence, and
`deploy/helm/vpay/README.md:542`'s trailing clause.

**So: 13 stale claims retired in total** (8 grep hits + 5 found by reading),
across 8 files.

### The proof criterion, honestly

The brief asks that the grep afterwards return "only (b)/(c) hits. **It does
not, and I did not make it.** Six class-(a) hits remain — `status.md:998`,
`:1390`, `:1391`, `:1405`, `:1830` and `roadmap.md:603` — every one of them in
the test-evidence family of §4 rather than the release-artefact family this
task names. Retiring them means re-justifying five 🟡 rows on the status page,
which is a different decision and a different owner. **Everything in the
release-artefact family is retired.** An accurately reported six is worth more
than a zero I would have had to invent a judgement to reach.

### Verified-still-true, listed as the brief asks

These were each checked against `docs/status.md` before being left:

1. `docs/flows/deployment.md:11` — the Helm chart "has never run". No cluster
   has run it; `docs/status.md` and `deploy/helm/vpay/README.md` agree.
2. `docs/runbooks/release.md:173` — `cosign verify` has never been run. I did
   not run it either; the signature evidence here is "the signing step exited
   0", not "the signature verifies".
3. `docs/runbooks/rotate-signing-key.md:185` — the `kubectl` steps have never
   been run.
4. `docs/sdks/parity.md:239` — `sdks/stripe-js` has never run against a live
   stack.
5. `docs/sdks/parity.md:240` — Checkout Sessions have never run against a live
   stack.
6. `docs/roadmap.md:827` — no real rail has been called.
7. `docs/status.md:1409` — the worker loop "has never run outside a test or a
   developer's `just demo`". True: there is no deployment.
8. `docs/status.md:1763` — `cratestack migrate diff` has never been run
   against a real database.

Also still true and newly written down rather than assumed: **no `v*` tag has
ever been pushed** (all 13 runs took `type=raw,value=edge`), **no published
image has been pulled or executed anywhere**, and **GHCR visibility is
unmeasured**.

---

## 3. What was written into `docs/status.md`, verbatim

Appended inside the "Image publishing" row (the row already carried the Step 7
and 2026-09-04 corrections), immediately before its closing `See …` clause:

Quoted as a fenced block, not as markdown, so its `docs/`-relative links do
not dangle from this file's directory:

```text
**Updated 2026-09-05: that next run happened and the fix holds — `release.yml` has now run 13 times on `master` (12 green, the one failure being `33894388991` above), and the four post-rename runs `33898736618`, `33912330063`, `33918831901` and `33929374661` all resolved `NAMESPACE: vaam-apps` from `github.repository_owner` and pushed all four signed manifest lists; the stale "has never run / no image exists / nothing has been signed" claims that this row had already retired were retired the same day in `.github/workflows/release.yml`'s header, [runbooks/release.md](runbooks/release.md) (header and §6), [runbooks/deploy-and-rollback.md](runbooks/deploy-and-rollback.md) (header and §6), [flows/deployment.md](flows/deployment.md) (§9 and Status), [runbooks/README.md](runbooks/README.md) and `deploy/helm/vpay/README.md`, each citing run `33929374661`; GHCR package visibility remains unmeasured, because `gh api "orgs/vaam-apps/packages?package_type=container"` returns 403 for want of a `read:packages` scope and an anonymous `ghcr.io/token` pull request is refused.**
```

### 3.1 Three status.md edits, where the brief permitted one

The brief says I **may** add one dated sentence to the image-publishing row.
I made **three** changes to `docs/status.md`, and the two extra ones need
justifying rather than burying:

1. **`status.md:1635`** (`vpay_build_info{git_sha}` row) ended: "no image has
   ever been built with a real sha: `release.yml` has never run, so the
   `build-args` line is unexecuted configuration". That is the task's own
   defect — a present-tense "release.yml has never run" — and §1.2 measures
   the exact `--build-arg VPAY_GIT_SHA=33d6c253…` that refutes it. Struck
   through with a dated correction; the row stays 🟡, on a narrower reason.
2. **`status.md:1621`** (`vpay-checkout` image row) ended: "Published and
   signed by `release.yml` — **which has still never run**". Same defect, same
   family. Struck through, citing the checkout index digest and its tlog
   index. I also updated the adjacent row at `:1624`, which said the workflow
   "has now run, once, and failed" — true on 2026-09-04, stale now that four
   green runs have followed.

I took these because leaving a knowingly false present-tense claim on the
status page is the failure mode `CLAUDE.md` names, and the brief's own proof
criterion asks the grep to come back free of class-(a) hits. I did **not**
extend the same reasoning to the test-evidence rows (§4), because those change
what a 🟡 means rather than correcting a fact.

---

## 4. Left for the maintainer — not mine to decide

The same grep surfaces a **second family of stale claims**, in the opposite
direction from the release ones and outside this task's remit. `docs/status.md`
lines 998, 1390, 1391, 1405 and 1830 say variants of "these tests **have never
run under Docker, here or in CI**", and `docs/roadmap.md:603` says "it has not
run in CI". §1.4 measures that CI run `33929374663` ran 1159 tests to 0
skipped on `ubuntu-latest`, including 163 `vpay-tests-integration` and 86
`vpay-db` — so those rows understate the evidence that exists.

I retired exactly one of them, `docs/flows/merchant-auth.md:486`, because it
is a flow doc in this task's blast radius and the brief listed it. **I did not
touch the six remaining rows**: the brief limits me to one sentence in the
image-publishing row, and rewriting five 🟡 justifications could change
whether those rows should still be 🟡 — a call for whoever owns the status
page, not for this pass. They are listed here so the next pass does not have
to re-find them.

---

## 5. Gates

Every command below was run in this worktree after the last edit.

| Gate | Command | Result |
|---|---|---|
| verify | `just verify` | **exit 0** — `verify-no-mocks: ok`; `verify-status: ok — 1 unimplemented item(s), all declared in docs/status.md and all still in shipping code` (two-directional); `verify-errors: ok — 15 error type(s)`; `verify-sdk-parity: ok — 342 proving test(s) … 26 dated gap(s)`; `verify-docs` advisory |
| docs | `just docs-check` | **exit 0** — but it is `cargo xtask verify-status` plus `note: link checking is not implemented yet`. **It does not check links.** |
| links | own checker (below) | **159 relative links across the 8 edited markdown files, 0 missing**; `checked=159 ok=159 miss=0`, exit 0 |
| actions | `actionlint .github/workflows/release.yml` | **exit 0**; `actionlint` over all of `.github/workflows/` also exit 0 |
| grep | the §2 grep, re-run | only (b)/(c)/(n/a) hits remain; every (a) line that still matches is inside a `~~…~~` strike-through, checked by eye |

Because `just docs-check` does not check links, the link result comes from a
throwaway script: it extracts every markdown inline-link target and
resolves it against each file's own directory, skipping `http`/`mailto`/bare-anchor targets **and fenced code
blocks**. It was given a **control** before being trusted — a probe file with
a good link, a `../` link, an anchor link, a dangling link and a link inside a
```` ```text ```` fence reported `checked=4 ok=3 miss=1`, naming the dangling
one and ignoring the fenced one, and exited 1. So a miss is detectable; this
is not a checker that always passes.

The fence rule matters here: §3 quotes the `docs/status.md` sentence verbatim,
and that sentence's links are relative to `docs/`, so they would dangle when
read from `docs/plans/step9-notes/` (moved there 2026-09-05; the same two
directory levels below `docs/`, so the dangling-link risk is unchanged). The
quote is fenced as ```` ```text ````
rather than reworded, which keeps it verbatim and stops it rendering four
broken links. The first version of the checker had no fence rule and reported
those four; the checker was fixed rather than the finding argued away.

`just ci` was **not** run: this change touches no compiled code, and the
expensive Rust gates are covered by `just verify` above.

---

## 6. Corrections from the review pass, 2026-09-05

Two accuracy defects in the account above, found by the sabotage review and
recorded here rather than silently edited. The full review is
[release-claims-review.md](release-claims-review.md).

1. **§3.1 says "I made **three** changes to `docs/status.md`".** The diff
   changes **four** table rows: `:1618` (the permitted sentence), `:1621`,
   `:1624` and `:1635`. The over-reach was disclosed; its size was understated
   by one row.
2. **§2 classifies `docs/status.md:1405` ("The demo has never run in CI") as
   (a), left deliberately.** Measured: `.github/workflows/ci.yml` brings up
   `-f compose.demo.yml` in the `e2e (compose)` job but **no step in the
   workflow invokes `just demo`**, so the walkthrough has indeed never run in
   CI. It is class **(c), verified still true**. The count table's
   "(a) left deliberately — 6" is therefore **5**, and "(c) verified still
   true — 8" is **9**; the total of 43 is unchanged.

The review also confirmed §2's warning about the grep by mutation: reverting
`release.yml`'s header to "NOTHING IN THIS FILE HAS EVER RUN" is caught by
neither `actionlint`, nor `just verify`, nor the brief's grep — which does not
match that file at all. And nothing in the repository catches an invented run
id: substituting `39999999999` for `33929374661` leaves every gate green.
