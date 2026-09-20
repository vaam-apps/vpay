# Release: cut a tag, verify a signature, pin a digest

~~**Nobody has done this.** No tag has been pushed, `.github/workflows/release.yml`
has never run, no image exists at `ghcr.io/vaam-apps/vpay-*`, and nothing has
been signed.~~

**Corrected 2026-09-05.** Three of those four clauses are no longer true.
`gh run list --workflow release --branch master --limit 20` returns **13 runs,
12 green** (2026-09-03 15:25 UTC → 2026-09-04 23:24 UTC; the one failure,
`33894388991`, is the organisation rename described in
[../status.md](../status.md)). In the most recent, **`33929374661`** (head
`33d6c25`), all 13 jobs succeeded, and its log records the four manifest lists
being pushed and then signed:

| Image                                                                                        | Index digest pushed to `:edge` and `:sha-33d6c25…`                        | Rekor tlog index |
| -------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------- | ---------------- |
| `ghcr.io/vaam-apps/vpay-server`                                                              | `sha256:5485db5e397edd8e672737e676756ca4e9eb56a23fb117a6bc762e0532b50537` | 2717616118       |
| `ghcr.io/vaam-apps/vpay-worker` (**retired 2026-09-07, issue #77 — this is its last build**) | `sha256:08667b03bae210802d04d59dba92820be9bccb4052f8337c74f0ea0a80d68a78` | 2717617767       |
| `ghcr.io/vaam-apps/vpay-dashboard`                                                           | `sha256:ba6d6712dc143598c66c34300dffa3e38cdd5a21de98dfc9b43a13103b21a7a7` | 2717616040       |
| `ghcr.io/vaam-apps/vpay-checkout`                                                            | `sha256:5214e408be6062123b51374d99988ef20e28081fa96e7bcb0eb4ac2b5b12e51e` | 2717615975       |

Digests come from the `create the manifest list` step's `pushing <digest> to
<image>:edge` lines; tlog indices from each `cosign sign (keyless, GitHub
OIDC)` step in the matching `manifest list + sign (<image>)` job.

**GHCR package visibility is still unmeasured, and not for want of trying.**
`gh api "orgs/vaam-apps/packages?package_type=container"` returns HTTP 403
`You need at least read:packages scope to list packages` — the available token
carries `gist, project, read:org, repo, user, workflow` and was not authorised
for packages. An unauthenticated pull is refused too: `GET
https://ghcr.io/token?scope=repository:vaam-apps/vpay-server:pull` answers
`UNAUTHORIZED`, and the tags endpoint 401s. That is evidence the four packages
are **not anonymously pullable**; it does not by itself distinguish "private"
from "absent", and the run log above is what establishes that they exist.

~~**What is still true, and it is the fourth clause:** **no `v*` tag has been
pushed.**~~ **Retired 2026-09-19, and it had been wrong for two weeks.** Three
tags exist — `v0.1.1`, `v0.2.0` and `v0.2.1` — and `release.yml` ran on every
one of them:

| Tag      | Run           | Outcome                                         |
| -------- | ------------- | ----------------------------------------------- |
| `v0.1.1` | `35275194212` | every job succeeded                             |
| `v0.2.0` | `35361829971` | `merge` succeeded; both npm publish jobs failed |
| `v0.2.1` | `35430925331` | `merge` succeeded; both npm publish jobs failed |

So §2's semver table **is** exercised: `{{version}}` and `{{major}}.{{minor}}`
have produced real tags. The failures on the two most recent runs are confined
to `publish-node-sdk` and `publish-stripe-js-sdk`; every `build` and every
`merge` job succeeded on all three, which is the half a chart release depends
on (§5 — `publish-chart` is `needs: merge`).

What remains true is narrower and worth keeping: §3's `cosign verify` has
still never been run by anyone against anything this repository produced, so
that section is still written from Fulcio's documented identity format rather
than from a certificate somebody read. Read [../status.md](../status.md)
before you trust a step here.

---

## 1. What a release is

A `v*` tag on `master`. Pushing it runs
[`release.yml`](../../.github/workflows/release.yml), which builds three
images on two architectures, merges each pair into a manifest list, applies
the tags, and signs each manifest-list digest with cosign.

| Trigger                  | Tags produced (per image)      |
| ------------------------ | ------------------------------ |
| `git push origin v1.2.3` | `1.2.3`, `1.2`, `sha-<40 hex>` |
| a merge to `master`      | `edge`, `sha-<40 hex>`         |

There is deliberately no `latest`. A real deployment pins a digest (§4).

**A `v*` tag does not stop at images.** It also publishes `deploy/helm/vpay`
itself, as an OCI artifact of its own that the table above does not show,
because it is not "per image" and it is not produced on a `master` merge at
all — only a tag publishes a chart, deliberately, so there is no `edge` chart
the way there is an `edge` image. See §8.

The three images are `ghcr.io/vaam-apps/vpay-server`, `-dashboard` and
`-checkout` (step-6 decision (1), plus `vpay-checkout` from Step 9 D3 on
2026-09-04 — this section said "three" until the 2026-09-05 review pass
noticed the count had moved, and then said "four" until 2026-09-19, which
is the error described next). ~~The four images are … `-worker` …~~ **The
count moved back on 2026-09-07 and this page did not follow it for twelve
days.** Issue #77 retired `vpay-worker`: there is one backend image, and the
worker Deployment runs `vpay-server` with `args: ["worker"]`. `release.yml`'s
`build` and `merge` matrices have listed three images since, and
`just release-dry-run` builds three. Corrected 2026-09-19.

The chart deploys `-server` always — as both the server and the worker
workload — and `-checkout` behind `checkout.enabled` (default false);
`vpay-dashboard` is published and **not** templated — see
`deploy/helm/vpay/README.md` for why.

## 2. Cutting a tag

Before you tag, the thing worth checking is the ones CI cannot: that `master`
is green, that the chart's `appVersion` and the tag agree (because
`values.yaml`'s `images.*.tag` defaults to `.Chart.AppVersion`), and that
`Chart.yaml`'s `version:` has actually moved since the last release that
published a chart. That third check is new (2026-09-19, `publish-chart`): a
release whose chart `version:` is unchanged does not merely look wrong, it
**fails the run** at `publish-chart`'s republish guard, after the three
images have already built and pushed — see §8. Catch it here, before tagging,
not there.

```bash
just verify              # the two self-checks
just ci                  # everything CI runs, in CI's order
just release-dry-run     # the three images for THIS host's arch, then the chart
gh run list --branch master --limit 3

grep -n '^appVersion:' deploy/helm/vpay/Chart.yaml   # must match the tag, sans `v`
grep -n '^version:' deploy/helm/vpay/Chart.yaml      # must differ from the last published chart version, or §8's guard fails the run

git tag -a v1.2.3 -m 'vpay 1.2.3'
git push origin v1.2.3
gh run watch "$(gh run list --workflow release --limit 1 --json databaseId --jq '.[0].databaseId')"
```

`just release-dry-run` builds for the host architecture only. The other
architecture is built by a native `ubuntu-24.04-arm` runner and by nothing
else (step-6 decision (8), [ADR-0014](../adr/0014-builder-host-musl-triple.md));
there is no local rehearsal for it that is not QEMU.

### If the run fails

| Symptom                                                             | Almost certainly                                                                                                                                                                                              |
| ------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `failed to load manifest for workspace member` in the backend build | a new `[workspace] members` entry that `backends/Dockerfile` does not `COPY`. Add the `COPY`; see the Dockerfile's header                                                                                     |
| `denied: permission_denied` on push                                 | `packages: write` missing, or the package's visibility/permissions in GHCR do not let this repository push                                                                                                    |
| the run is green but nobody can pull the image                      | **the first push creates the GHCR package as private.** `GITHUB_TOKEN` can create and push it; making it public is a one-time change in the package's settings, done by a human, and no workflow here does it |
| `imagetools create` says a digest is not found                      | one architecture's `build` job failed; `fail-fast: false` means the other still uploaded its digest                                                                                                           |
| cosign asks for a key or fails on the OIDC token                    | `id-token: write` missing at the job or workflow level                                                                                                                                                        |

A failed run publishes nothing usable: the per-architecture manifests are
pushed by digest and untagged, so no tag moves until `imagetools create`
succeeds. **Re-run the workflow; do not re-cut the tag.** A tag that has been
pushed once is public history.

## 3. Verifying a signature

Signing is keyless (step-6 decision (3)): there is no public key. The
certificate binds the image to _this workflow file at this ref_, so
verification names the workflow, not a key.

```bash
IMAGE=ghcr.io/vaam-apps/vpay-server:1.2.3

cosign verify \
  --certificate-identity-regexp '^https://github\.com/vaam-apps/vpay/\.github/workflows/release\.yml@refs/(tags/v.*|heads/master)$' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com \
  "$IMAGE"
```

Read the output rather than the exit code alone: the claims that matter are
`Subject` (the workflow identity above), `githubWorkflowRef` (the tag it was
built from) and `githubWorkflowSha` (the commit). An image built from a branch
you did not expect satisfies a loose regexp and fails this reading.

**Tighten the regexp for a production gate.** The one above accepts `edge`
builds from `master`. A policy that should only admit tagged releases uses
`@refs/tags/v.*$` and nothing else.

Two things this verification does **not** establish:

- **Only the manifest-list (index) digest is signed**, not the per-architecture
  child manifests. Verifying a tag or the index digest is covered; pinning a
  child manifest's own digest is not.
- Renaming or moving `release.yml` changes the certificate identity and breaks
  every command in this section. That is the cost decision (3) accepted. **So
  does renaming the GitHub organisation**, because the identity is a full URL
  including the owner: `https://github.com/<owner>/vpay/...`. The regexp
  above already reflects the 2026-09-04 rename (`vaam-store` -> `vaam-apps`);
  any signature made before that date carries `vaam-store` in its `Subject`
  and will not match it, and `cosign verify` against an old image needs the
  old regexp — the identity is fixed at signing time, not re-derived later.

Provenance and SBOM ride in the image index (`provenance: mode=max`,
`sbom: true`), and are read separately:

```bash
cosign download attestation "$IMAGE"                       # in-toto attestations
docker buildx imagetools inspect "$IMAGE" --format '{{json .Provenance}}'
```

## 4. Pinning a digest in Helm values

Tags move; a `v1.2.3` tag can be force-pushed and `edge` moves on every merge.
Anything real pins the digest.

```bash
IMAGE=ghcr.io/vaam-apps/vpay-server:1.2.3
docker buildx imagetools inspect "$IMAGE" --format '{{json .Manifest}}' | jq -r .digest
# sha256:<64 hex>
```

That is the same digest the release run wrote to its job summary, and the one
cosign signed. Put it in your values file:

```yaml
images:
  server: { digest: "sha256:<64 hex>" }
```

This block carried a second line, `worker: { digest: … }`, until issue #77
(2026-09-07). Copying it now is not a harmless leftover: `values.schema.json`
is `additionalProperties: false` and `images.worker` was removed with the
image, so a values file carrying it fails `helm lint` with `images:
Additional property worker is not allowed` — deliberately, so that a
pinned-but-unpulled digest cannot sit in a values file looking load-bearing.

A digest wins over a tag in this chart, and the `image-digest-format` template
guard rejects anything that is not `sha256:` + 64 hex at template time rather
than at image-pull time in the cluster. Verify the render before you install:

```bash
helm template vpay deploy/helm/vpay -f your-values.yaml | grep -n 'image:'
```

**One digest pins both workloads, and that is a change.** Until issue #77
(2026-09-07) this read "the two workloads are pinned independently and must be
pinned together", because `vpay-server` and `vpay-worker` were separate images
sharing a database schema and a migration set, and running two versions
against one database is not a supported configuration. There is one image now
and both Deployments resolve `images.server`, so that particular hazard is
gone by construction rather than by an instruction you have to follow. It is
_not_ gone for a rolling upgrade that spans a migration, which is §5's
subject.

**The chart itself is pinned and installed the same way its images are** —
by an explicit `--version` rather than a moving reference, because there is
no `latest`-equivalent for a chart either. See §8.

## 5. Rolling back

Roll back by pinning the previous digest and upgrading — not by moving a tag.

```bash
helm upgrade vpay deploy/helm/vpay -f your-values.yaml   # with the older digests
```

Two rollbacks are **not** safe and are documented where they bite:

- **A migration that has run is not rolled back by an older image.** There is no
  down-migration path in this repository.
- **Rolling back to a retired signing-key `kid` crash-loops the server with
  exit 78** (`DbError::SigningKeyRetired`), not 69. Roll forward. See
  [../flows/deployment.md](../flows/deployment.md) §7.

## 6. What is unproven

Everything above. Specifically:

- ~~No `release.yml` run exists. Not one image has been built by it, pushed,
  merged into a manifest list or signed.~~ **Retired 2026-09-05: 13 runs
  exist, 12 green, the latest `33929374661` — see the correction at the top of
  this page for the digests and tlog indices.**
- ~~`aarch64-unknown-linux-musl` has never been compiled — not in CI, not
  locally. The arm64 half of every manifest list is unbuilt code paths in a
  workflow file.~~ **Retired 2026-09-05:** in `33929374661` all four
  `build … (arm64)` jobs ran on `ubuntu-24.04-arm` and succeeded — including
  `build vpay-server (arm64)` and `build vpay-worker (arm64)`, the two built
  from `backends/Dockerfile`, which is where the musl triple is actually
  compiled. So the triple builds and the arm64 half of each manifest list is
  real. **How that is read, because the string
  `aarch64-unknown-linux-musl` appears nowhere in the run log:**
  `backends/Dockerfile` deliberately does not name a target and builds the
  builder's own host triple (see its header), the builder resolves to
  `docker.io/library/rust:1.95.0-alpine3.22` on `linux/arm64` in that job, and
  the job logs `Compiling vpay-server v0.1.0` then ``Finished `dist` profile
[optimized] target(s) in 4m 32s`` — an alpine (musl) rust image on an arm64
  host has exactly one host triple. Nobody has run `rustc -vV` in that image
  and read the triple back
  ([ADR-0014](../adr/0014-builder-host-musl-triple.md) still records why the
  `+crt-static` entry is needed).
- ~~**No `v*` tag has ever been pushed.** Every run took the
  `type=raw,value=edge` branch, so the semver tag path (`{{version}}`,
  `{{major}}.{{minor}}`) in §1's table has never produced a tag.~~ **Retired
  2026-09-19:** three tags have been pushed and `release.yml` ran on each —
  see the correction at the top of this page for the runs and what failed in
  them. The semver tag path is exercised.
- ~~**`publish-chart` has never executed its steps (2026-09-19).** It has been
  _evaluated_ once: in run `35454036800`, the first `master` run to carry it,
  it reported `skipped` while the other thirteen jobs succeeded — which is
  the whole of the evidence that its `if:` gate works and that adding it
  disturbs nothing on a merge. Everything inside the job is still unrun.
  Nothing has been pushed to `oci://ghcr.io/vaam-apps/charts`, no chart has
  been signed, and no `cosign verify` has been read against one — see §8.~~
  **Retired 2026-09-20: the next tag was its first real execution, and it
  failed.** `v0.2.2` (run `35491807158`) pushed `charts/vpay:0.2.1` and then
  failed at `cosign sign` with `UNAUTHORIZED: unauthenticated: User cannot be
authenticated with the token provided` — a real, published, unsigned chart,
  and also a mislabelled one: it was pushed while `version:` still read
  `0.2.1` against an `appVersion` of `0.2.2`. Measured against the registry
  the same day: one tag (`0.2.1`), no `.sig` manifest (HTTP 404), and an
  anonymous manifest pull that answers HTTP 200 — the package is **public**,
  which corrects the standing assumption below that a first push leaves a
  private package needing a human to flip a setting. §8 now describes the
  guard fix this failure prompted and what a maintainer should do about a run
  stuck the same way; **the resume branch that fix adds has still never
  run** — the next tag, `v0.3.0` (run `35492982589`), published and signed
  cleanly on its first attempt, so it took the guard's "not found → push"
  branch, not a resume. What remains true, narrower than before: a signature
  now exists on `0.3.0` and can be downloaded, but no `cosign verify` has
  been read against it or any other vpay chart — see §8.

  _This bullet first said "for the same reason" as the retired no-tag bullet
  above, which was wrong twice over: tags do exist, and the actual reason was
  narrower — `publish-chart` landed on `master` after `v0.2.1` was cut, so no
  tag had yet been pushed **with the job present**. The next tag, `v0.2.2`,
  was that first real execution, and it is recorded above rather than
  predicted here now that it has happened._

- **No image from any run has been pulled or executed anywhere**, and GHCR
  package visibility is unmeasured (the token lacks `read:packages`;
  anonymous pull is refused). A green push is not a reachable image.
- `cosign verify` has never been run against anything from this repository, so
  the regexp in §3 is derived from the workflow's `on:` block and Fulcio's
  documented identity format, not from a certificate anyone has read.
- `just release-dry-run` exercises the Dockerfiles and the chart. It exercises
  neither the registry, nor the attestations, nor the signature.
- **No GitHub Actions cache-hit rate has ever been read for either
  Dockerfile**, before or after the 2026-09-05 cargo-chef change (§7). What
  §7 reports was measured on an authoring host with a local
  `docker-container` builder. `type=gha` behaves differently — it is a
  network-backed store with an eviction policy and a per-repository budget —
  and nobody has looked at a `build-push-action` log to see which layers it
  actually restored.

## 7. What a release run recompiles (the build cache)

Added 2026-09-05, when `backends/Dockerfile` gained
[cargo-chef](https://github.com/LukeMathWalker/cargo-chef). Read this before
changing that file, because the _order_ of its instructions is now part of
its behaviour.

The Rust image is built in four stages:

| Stage             | What it does                                                                                                                                                                                                         | When it re-runs                                                                           |
| ----------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------- |
| `chef`            | `rust:1.98.0-alpine3.22` (was `1.95.0-alpine3.22` until 2026-09-05; the Alpine base deliberately did not move with the compiler), `apk add musl-dev pkgconfig`, `cargo install cargo-chef --locked --version 0.1.78` | the base image tag or the cargo-chef pin changes                                          |
| `planner`         | copies the workspace, runs `cargo chef prepare` → `recipe.json` (manifests + `Cargo.lock`, **no source**)                                                                                                            | every build; it compiles nothing and takes ~0.1 s                                         |
| `builder` (cook)  | `cargo chef cook --profile dist --target <host triple> -p vpay-server` (it named `-p vpay-worker-bin` too until issue #77) — compiles the ~317-package dependency graph into `target/`                               | `recipe.json` changes (a manifest or the lockfile moved), or `.cargo/config.toml` changes |
| `builder` (build) | `ARG VPAY_GIT_SHA`, copy the real source, `cargo build`, `cp` to `/out`                                                                                                                                              | any source edit, or a different `VPAY_GIT_SHA`                                            |

Three properties this shape depends on. Two of the ways to break them are
silent — the build stays _correct_, it just stops caching — and one is loud;
each entry says which, because they were established by mutation:

1. **The cook's flags must match the build's.** Same `--profile dist`, same
   `--target` (read from `rustc -vV`, never hardcoded — see the Dockerfile's
   header and [ADR-0014](../adr/0014-builder-host-musl-triple.md)), same
   `-p` selection, and `.cargo/` copied in first so `+crt-static` applies. A
   cook under different rustflags writes fingerprints the real build rejects.
   The two halves fail differently, and only one of them is visible: dropping
   `--target` kills the cook in under a second (`cannot produce proc-macro
for async-trait ... x86_64-unknown-linux-musl does not support these crate
types`), while dropping `--profile dist` **succeeds**, cooks the `dev`
   profile, and leaves the next `cargo build` to recompile everything — 305 s
   for a rebuild that costs 105 s when the flags match.
2. **`ARG VPAY_GIT_SHA` must stay below the cook.** `release.yml` passes a
   different `github.sha` on every push; an `ARG`/`ENV` pair above the cook
   invalidates the dependency layer on every release build.
3. **`planner` and `builder` must copy the same workspace directories.** The
   recipe has to describe the workspace the next stage compiles. Loud:
   dropping `.xtask`/`sdks/rust` from the planner fails `cargo chef prepare`
   in a second (`failed to read /build/sdks/rust/Cargo.toml`) rather than
   emitting a shrunken recipe. The one deliberate difference is `.cargo`,
   which only `builder` copies — so the recipe's `config_file` is `null` and
   `.cargo/config.toml` reaches the cook through that stage's own `COPY`,
   which is also what makes a change to it invalidate the cook.

Measured on the authoring host on 2026-09-05, `linux/amd64`, on a dedicated
`docker-container` buildx builder pruned before the cold run — see
[../plans/exp8-notes/opus.md](../plans/exp8-notes/opus.md) for the logs:

| Build                                                       | Before (one-stage) | After (cargo-chef)        |
| ----------------------------------------------------------- | ------------------ | ------------------------- |
| cold, empty builder cache                                   | 254 s              | 238 s                     |
| one comment line added to `vpay-server/src/main.rs`         | 260 s              | **125 s** (cook `CACHED`) |
| `--build-arg VPAY_GIT_SHA` changed, nothing else            | —                  | **116 s** (cook `CACHED`) |
| the same, with rule 2 violated (`ARG` moved above the cook) | —                  | 251 s (cook re-ran)       |

**The cold row above is the one number that did not survive review, and the
direction matters.** It was a single unpaired sample. Re-measured the same
day as two matched pairs — the same isolated builder pruned between the two
runs of each pair, the two runs back to back, and the second pair in the
reverse order to control for a host that several agents were building on:

| Pair                 | one-stage | cargo-chef |
| -------------------- | --------- | ---------- |
| 1 (one-stage first)  | 193 s     | 256 s      |
| 2 (cargo-chef first) | 212 s     | 248 s      |

**A cold build is 36-63 s slower than it was**, which is `cargo install
cargo-chef` (32-58 s here) plus the cook's own pass over the graph. The warm
numbers were reproduced in the same pass on the busier host, twice each —
105 s and 114 s for a source touch, 101 s and 112 s for a sha-only rebuild,
215 s for the same sha-only rebuild with rule 2 violated, and 1 s for a
change to a `docs/` file, which `.dockerignore` keeps out of the context
altogether. So the trade is: every
cold build pays about 45 s; every warm one saves about 150 s.

The runtime images are unchanged: `vpay-server` is 15.9 MB before and after,
two layers both times, the `config/` layer the same digest in both, and
`docker export` of it lists `config/` and `/vpay-server` and nothing else
this build put there (the tar also carries the `/dev`, `/etc`, `/proc`,
`/sys` and `.dockerenv` stubs the runtime creates for any container, which
are not layers). cargo-chef is in the builder only. Re-verified in review
against an image built from the pre-cargo-chef Dockerfile as well as this
one, and both `scratch` images answer `--version` with exit 0.

**The saving is bounded by `[profile.dist]`, and the number above is the
honest one.** `dist` inherits `release`: `lto = "fat"`, `codegen-units = 1`.
A fat-LTO link re-consumes every dependency's LLVM IR, so the final
`cargo build` costs about two minutes however much of the graph is already
compiled. Halving an incremental build is what cargo-chef buys here — not the
near-instant rebuild it buys a project without fat LTO.

**An open question this change deliberately leaves open.** Nothing records
why `[profile.release]` sets `lto = "fat"` — no ADR mentions LTO and
`Cargo.toml` carries no comment on that line — so whether `[profile.dist]`
should override it with `lto = "thin"` (much cheaper links, a slower binary
by an unmeasured amount) is a maintainer's decision about the shipped
artefact, not a build-plumbing one. It is named here because it, and not
cargo-chef, is what now bounds a release rebuild.

## 8. Publishing the chart

Added 2026-09-19: `release.yml` gained a `publish-chart` job. This section is
numbered `8` rather than slotted in as a new `5` — where chart publishing
would belong, next to §4's pinning — for a reason that will not go away with
a more thorough pass. Live pages cite §5, §6 and §7 by number
(`docs/runbooks/rotate-signing-key.md`,
`docs/runbooks/restore-from-backup.md`, and a `See … §7` in
`docs/status/infrastructure.md`) and could be updated; the notes under
`docs/plans/` cite §4, §6 and §7 as well, and those are dated records of what
this page said at the time, so editing them to match a later numbering would
falsify the archive rather than maintain it. The numbering is therefore
append-only. §1 and §4 above link forward to this section instead.

**What publishes it, and when.** `publish-chart` (`needs: [namespace,
merge]`) packages `deploy/helm/vpay` and pushes it as an OCI artifact, same as
an image. `needs: [namespace, merge]` is not incidental: the chart cannot
exist before the three images it names, because `values.yaml` defaults
`images.*.tag` to `.Chart.AppVersion`, so `publish-chart` waits for `merge` —
the job that assembles and tags the manifest lists — to finish first. Its
`if:` is `startsWith(github.ref, 'refs/tags/v')` and nothing else: unlike the
images, this chart has no `edge` build at all. A merge to `master` publishes
three images and zero charts. See §1.

**Where it lands.** `oci://ghcr.io/<namespace>/charts/vpay`, under the same
`namespace` job the images use (lowercased `github.repository_owner`) — today
that is `oci://ghcr.io/vaam-apps/charts/vpay`.

**The tag is `Chart.yaml`'s `version:` — and since #225, that no longer moves
independently of the app version or the git tag.** `helm push` derives the
OCI tag from the chart it is packaging; there is no second name for the job
to set. `publish-chart` reads both `version:` and `appVersion` and refuses to
proceed unless `version == appVersion == tag`. That check used to cover only
`appVersion` (`values.yaml`'s `images.*.tag` defaults to `.Chart.AppVersion`,
so a mismatch there would ship a chart that defaults to an image this release
never built) — `version:` was hand-bumped and deliberately left unchecked,
on the theory that the chart had its own release lifecycle separate from the
app's (`Chart.yaml`'s own comment argued for exactly this, 0.2.0 -> 0.2.1).
It no longer does: release-please owns both lines now
(`release-please-config.json`'s `extra-files`), so they move together by
construction, and the added check is what catches a tag cut by hand or an
annotation that silently stopped being rewritten — which is exactly how this
repository lost `Chart.yaml` once (`Chart.yaml`'s own history records it).
**The chart can no longer be released independently of the application.**
See §2 for the check to run on `version:` before you tag.

**`helm push` silently overwrites an existing version — measured, not
assumed.** Against a throwaway local `registry:2` (2026-09-19), pushing the
same packaged chart twice returned exit 0 both times, with no warning either
time. Left alone, a release that forgot to bump `version:` would silently
replace a chart people already have with one that resolves to different
images — the same hazard `Chart.yaml`'s 0.2.0 -> 0.2.1 comment already
describes, arriving through the registry instead of through a clone. So
before packaging anything, the job runs:

```bash
helm show chart oci://ghcr.io/vaam-apps/charts/vpay --version "$VERSION"
```

and reads **four** outcomes out of it, not three. It was three until
2026-09-20, and the missing one cost a release: push-then-sign is not atomic,
and a guard that can only tell "published" from "not published" cannot tell
"somebody already released this" from "this job died halfway through
releasing it" — so a signing failure turned into a release the pipeline could
not finish (below). The fourth outcome is what a signature answers, verified
against the real unsigned chart and specifically under cosign **v2.6.1** —
the version `cosign-installer` puts on the runner, not whatever a
maintainer's own machine has:

| `helm show chart`                                                      | Reads as                                                                                    | Job does                                                 |
| ---------------------------------------------------------------------- | ------------------------------------------------------------------------------------------- | -------------------------------------------------------- |
| exit 0, `cosign download signature <image>@<digest>` finds a signature | a real collision — this version is already published and signed                             | fail                                                     |
| exit 0, **no** signature found                                         | a previous run died between its push and its `cosign sign`                                  | **resume**: skip the push, sign the digest already there |
| exit non-zero, output contains `not found`                             | not published yet — a missing version and a missing chart name are textually identical here | proceed to package and push                              |
| exit non-zero, anything else (`connection refused`, TLS, a 5xx)        | the registry did not answer the question                                                    | fail rather than push past it                            |

The digest for the resume row comes off `helm show chart`'s own **stderr**,
which carries a `Digest:` line the same way `helm push`'s does. Re-packaging
to recompute it would not work: `helm package` is not byte-reproducible, so a
second package of identical content hashes differently and a signature made
against it would attach to a copy nobody pulls.

**The remedy depends on which `exit 0` row fired.** A real collision (already
signed) still means one thing: bump `version:` in `deploy/helm/vpay/Chart.yaml`
— since #225 that is release-please's job, not a hand-edit, so reaching this
in practice means the annotation was dropped or a tag was cut by hand — then
cut the tag again. **A died-between-push-and-sign collision (published, no
signature) needs nothing but a re-run of the job on the same tag.** It
resumes on its own: skips the push, signs the digest already in the registry,
and completes the release. It does not need a version bump or a new tag.

**This is not hypothetical — it is what happened.** Run `35491807158` (tag
`v0.2.2`, 2026-09-20) is exactly the died-between-push-and-sign case:
`cosign sign` failed with `UNAUTHORIZED: unauthenticated: User cannot be
authenticated with the token provided` after the push had already succeeded,
leaving a real, published, unsigned `charts/vpay:0.2.1`. The three-outcome
guard that existed then could not tell that apart from a real collision, and
refused every re-run of that tag — see §6. The resume path above is the fix.

**The next tag, `v0.3.0` (run `35492982589`, `chore: release master`),
published and signed cleanly on its first attempt** — all fourteen jobs
green, including `publish-chart`. That is the "not found → push" row of the
table above, not a resume: `0.3.0` had never been published before, so there
was nothing to resume from. It does mean the first row of the table —
`published AND signed` → real collision → fail — is no longer untested:
checked against the real registry now that a signed `0.3.0` exists, the
guard correctly reports a collision and refuses to re-push. **What is still
true is narrower: the resume row itself has never actually fired in a CI
run.** Nothing has yet died between push and sign a second time for it to
resume from.

Past the guard, the job packages, pushes, and reads the digest back out of
`helm push`'s own log — measured to print both its `Pushed:` and `Digest:`
lines to **stderr**, with stdout empty, which is why the job redirects rather
than piping stdout:

```bash
helm package deploy/helm/vpay --destination "$RUNNER_TEMP/chart"
helm push "$RUNNER_TEMP/chart/vpay-$VERSION.tgz" oci://ghcr.io/vaam-apps/charts
```

The digest is the chart manifest's own — measured to carry
`config.mediaType: application/vnd.cncf.helm.config.v1+json` and a single
layer of `application/vnd.cncf.helm.chart.content.v1.tar+gzip`, and
`cosign triangulate` resolves it, confirming cosign addresses a chart exactly
as it addresses an image. `publish-chart` signs that digest the same way
`merge` signs an image's: keyless, against the workflow's GitHub OIDC token.

**Verifying the signature** is §3's command with the reference changed to the
chart, and — unlike §3's own image example — the identity regexp tightened to
tags only, because a tag is the only thing that ever publishes a chart:

```bash
IMAGE=ghcr.io/vaam-apps/charts/vpay:<VERSION>

cosign verify \
  --certificate-identity-regexp '^https://github\.com/vaam-apps/vpay/\.github/workflows/release\.yml@refs/tags/v.*$' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com \
  "$IMAGE"
```

**Do not put `0.2.1` in for `<VERSION>`.** It fails the command above:
`cosign sign` never completed against it (§6), so `charts/vpay:0.2.1` is
unsigned, and it is also mislabelled — pushed while `version:` still read
`0.2.1` against an `appVersion` of `0.2.2`. Nothing will ever repair it in
place; the republish guard above keys on the version being released, so it
stays published, unsigned and wrong until somebody deletes it.

**`0.3.0` is different: it is published and a signature exists for it** (run
`35492982589`, 2026-09-20). But nobody has actually run the command above
against it and had it succeed — it was attempted from an authoring machine
and could not reach sigstore's TUF CDN (`tuf-repo-cdn.sigstore.dev`, `dial
tcp: connect: connection refused`, twice). So `cosign download signature`
finding a signature on `0.3.0` is established; the command above actually
confirming the Fulcio certificate identity and the Rekor entry is not. Use
whichever version you have personally run this command against and watched
pass — that is still nobody's `0.3.0`, as of 2026-09-20.

**Installing and pinning** follows §4's shape, with the chart's own version in
place of an image's digest:

```bash
helm show values oci://ghcr.io/vaam-apps/charts/vpay --version "$VERSION"
helm template vpay oci://ghcr.io/vaam-apps/charts/vpay --version "$VERSION" -f your-values.yaml
helm upgrade --install vpay oci://ghcr.io/vaam-apps/charts/vpay --version "$VERSION" -f your-values.yaml
```

`helm search` will not find any of this — an OCI registry carries no
`index.yaml`, so there is nothing for `helm search repo` or `helm search hub`
to read. `deploy/helm/vpay/README.md`'s Install section covers this path
alongside the local-checkout one, including what to do between releases, when
there is no chart tag to point at.

**Two real runs exist: the first failed, the second succeeded.** `v0.2.2`
(run `35491807158`) pushed and then failed to sign; `v0.3.0` (run
`35492982589`) published and signed cleanly. See §6.
