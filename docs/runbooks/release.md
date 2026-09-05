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

| Image | Index digest pushed to `:edge` and `:sha-33d6c25…` | Rekor tlog index |
|---|---|---|
| `ghcr.io/vaam-apps/vpay-server` | `sha256:5485db5e397edd8e672737e676756ca4e9eb56a23fb117a6bc762e0532b50537` | 2717616118 |
| `ghcr.io/vaam-apps/vpay-worker` | `sha256:08667b03bae210802d04d59dba92820be9bccb4052f8337c74f0ea0a80d68a78` | 2717617767 |
| `ghcr.io/vaam-apps/vpay-dashboard` | `sha256:ba6d6712dc143598c66c34300dffa3e38cdd5a21de98dfc9b43a13103b21a7a7` | 2717616040 |
| `ghcr.io/vaam-apps/vpay-checkout` | `sha256:5214e408be6062123b51374d99988ef20e28081fa96e7bcb0eb4ac2b5b12e51e` | 2717615975 |

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

**What is still true, and it is the fourth clause:** **no `v*` tag has been
pushed.** Every run above took the `type=raw,value=edge` branch, so §2's semver
table is still unexercised, and §3's `cosign verify` has never been run by
anyone — see "What is unproven" (§6). This runbook's *tag-cutting* half is
therefore still written from the workflow file rather than from a procedure
anyone has followed. Read [../status.md](../status.md) before you trust a step
here.

---

## 1. What a release is

A `v*` tag on `master`. Pushing it runs
[`release.yml`](../../.github/workflows/release.yml), which builds three images
on two architectures, merges each pair into a manifest list, applies the tags,
and signs each manifest-list digest with cosign.

| Trigger | Tags produced (per image) |
|---|---|
| `git push origin v1.2.3` | `1.2.3`, `1.2`, `sha-<40 hex>` |
| a merge to `master` | `edge`, `sha-<40 hex>` |

There is deliberately no `latest`. A real deployment pins a digest (§4).

The three images are `ghcr.io/vaam-apps/vpay-server`, `-worker` and
`-dashboard` (step-6 decision (1)). The chart deploys the first two;
`vpay-dashboard` is published and **not** templated — see
`deploy/helm/vpay/README.md` for why.

## 2. Cutting a tag

Before you tag, the thing worth checking is the one CI cannot: that `master` is
green *and* that the chart's `appVersion` and the tag agree, because
`values.yaml`'s `images.*.tag` defaults to `.Chart.AppVersion`.

```bash
just verify              # the two self-checks
just ci                  # everything CI runs, in CI's order
just release-dry-run     # the three images for THIS host's arch, then helm-check
gh run list --branch master --limit 3

grep -n '^appVersion:' deploy/helm/vpay/Chart.yaml   # must match the tag, sans `v`

git tag -a v1.2.3 -m 'vpay 1.2.3'
git push origin v1.2.3
gh run watch "$(gh run list --workflow release --limit 1 --json databaseId --jq '.[0].databaseId')"
```

`just release-dry-run` builds for the host architecture only. The other
architecture is built by a native `ubuntu-24.04-arm` runner and by nothing
else (step-6 decision (8), [ADR-0014](../adr/0014-builder-host-musl-triple.md));
there is no local rehearsal for it that is not QEMU.

### If the run fails

| Symptom | Almost certainly |
|---|---|
| `failed to load manifest for workspace member` in the backend build | a new `[workspace] members` entry that `backends/Dockerfile` does not `COPY`. Add the `COPY`; see the Dockerfile's header |
| `denied: permission_denied` on push | `packages: write` missing, or the package's visibility/permissions in GHCR do not let this repository push |
| the run is green but nobody can pull the image | **the first push creates the GHCR package as private.** `GITHUB_TOKEN` can create and push it; making it public is a one-time change in the package's settings, done by a human, and no workflow here does it |
| `imagetools create` says a digest is not found | one architecture's `build` job failed; `fail-fast: false` means the other still uploaded its digest |
| cosign asks for a key or fails on the OIDC token | `id-token: write` missing at the job or workflow level |

A failed run publishes nothing usable: the per-architecture manifests are
pushed by digest and untagged, so no tag moves until `imagetools create`
succeeds. **Re-run the workflow; do not re-cut the tag.** A tag that has been
pushed once is public history.

## 3. Verifying a signature

Signing is keyless (step-6 decision (3)): there is no public key. The
certificate binds the image to *this workflow file at this ref*, so
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

* **Only the manifest-list (index) digest is signed**, not the per-architecture
  child manifests. Verifying a tag or the index digest is covered; pinning a
  child manifest's own digest is not.
* Renaming or moving `release.yml` changes the certificate identity and breaks
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
  worker: { digest: "sha256:<64 hex>" }
```

A digest wins over a tag in this chart, and the `image-digest-format` template
guard rejects anything that is not `sha256:` + 64 hex at template time rather
than at image-pull time in the cluster. Verify the render before you install:

```bash
helm template vpay deploy/helm/vpay -f your-values.yaml | grep -n 'image:'
```

**The two workloads are pinned independently and must be pinned together.**
`vpay-server` and `vpay-worker` share a database schema and a migration set;
running two versions against one database is not a supported configuration.

## 5. Rolling back

Roll back by pinning the previous digest and upgrading — not by moving a tag.

```bash
helm upgrade vpay deploy/helm/vpay -f your-values.yaml   # with the older digests
```

Two rollbacks are **not** safe and are documented where they bite:

* **A migration that has run is not rolled back by an older image.** There is no
  down-migration path in this repository.
* **Rolling back to a retired signing-key `kid` crash-loops the server with
  exit 78** (`DbError::SigningKeyRetired`), not 69. Roll forward. See
  [../flows/deployment.md](../flows/deployment.md) §7.

## 6. What is unproven

Everything above. Specifically:

* ~~No `release.yml` run exists. Not one image has been built by it, pushed,
  merged into a manifest list or signed.~~ **Retired 2026-09-05: 13 runs
  exist, 12 green, the latest `33929374661` — see the correction at the top of
  this page for the digests and tlog indices.**
* ~~`aarch64-unknown-linux-musl` has never been compiled — not in CI, not
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
* **No `v*` tag has ever been pushed.** Every run took the
  `type=raw,value=edge` branch, so the semver tag path (`{{version}}`,
  `{{major}}.{{minor}}`) in §1's table has never produced a tag.
* **No image from any run has been pulled or executed anywhere**, and GHCR
  package visibility is unmeasured (the token lacks `read:packages`;
  anonymous pull is refused). A green push is not a reachable image.
* `cosign verify` has never been run against anything from this repository, so
  the regexp in §3 is derived from the workflow's `on:` block and Fulcio's
  documented identity format, not from a certificate anyone has read.
* `just release-dry-run` exercises the Dockerfiles and the chart. It exercises
  neither the registry, nor the attestations, nor the signature.
* **No GitHub Actions cache-hit rate has ever been read for either
  Dockerfile**, before or after the 2026-09-05 cargo-chef change (§7). What
  §7 reports was measured on an authoring host with a local
  `docker-container` builder. `type=gha` behaves differently — it is a
  network-backed store with an eviction policy and a per-repository budget —
  and nobody has looked at a `build-push-action` log to see which layers it
  actually restored.

## 7. What a release run recompiles (the build cache)

Added 2026-09-05, when `backends/Dockerfile` gained
[cargo-chef](https://github.com/LukeMathWalker/cargo-chef). Read this before
changing that file, because the *order* of its instructions is now part of
its behaviour.

The Rust image is built in four stages:

| Stage | What it does | When it re-runs |
|---|---|---|
| `chef` | `rust:1.95.0-alpine3.22`, `apk add musl-dev pkgconfig`, `cargo install cargo-chef --locked --version 0.1.78` | the base image tag or the cargo-chef pin changes |
| `planner` | copies the workspace, runs `cargo chef prepare` → `recipe.json` (manifests + `Cargo.lock`, **no source**) | every build; it compiles nothing and takes ~0.1 s |
| `builder` (cook) | `cargo chef cook --profile dist --target <host triple> -p vpay-server -p vpay-worker-bin` — compiles the ~317-package dependency graph into `target/` | `recipe.json` changes (a manifest or the lockfile moved), or `.cargo/config.toml` changes |
| `builder` (build) | `ARG VPAY_GIT_SHA`, copy the real source, `cargo build`, `cp` to `/out` | any source edit, or a different `VPAY_GIT_SHA` |

Three properties this shape depends on, each of which a plausible-looking
edit destroys silently — the build stays *correct*, it just stops caching:

1. **The cook's flags must match the build's.** Same `--profile dist`, same
   `--target` (read from `rustc -vV`, never hardcoded — see the Dockerfile's
   header and [ADR-0014](../adr/0014-builder-host-musl-triple.md)), same
   `-p` selection, and `.cargo/` copied in first so `+crt-static` applies. A
   cook under different rustflags writes fingerprints the real build rejects.
2. **`ARG VPAY_GIT_SHA` must stay below the cook.** `release.yml` passes a
   different `github.sha` on every push; an `ARG`/`ENV` pair above the cook
   invalidates the dependency layer on every release build.
3. **`planner` and `builder` must copy the same directories.** The recipe has
   to describe the workspace the next stage compiles.

Measured on the authoring host on 2026-09-05, `linux/amd64`, on a dedicated
`docker-container` buildx builder pruned before the cold run — see
[../plans/exp8-notes/opus.md](../plans/exp8-notes/opus.md) for the logs:

| Build | Before (one-stage) | After (cargo-chef) |
|---|---|---|
| cold, empty builder cache | 254 s | 238 s |
| one comment line added to `vpay-server/src/main.rs` | 260 s | **125 s** (cook `CACHED`) |
| `--build-arg VPAY_GIT_SHA` changed, nothing else | — | **116 s** (cook `CACHED`) |
| the same, with rule 2 violated (`ARG` moved above the cook) | — | 251 s (cook re-ran) |

The runtime images are unchanged: `vpay-server` is 15.9 MB before and after,
two layers both times, and `docker export` of it lists exactly `config/` and
`/vpay-server`. cargo-chef is in the builder only.

**The saving is bounded by `[profile.dist]`, and the number above is the
honest one.** `dist` inherits `release`: `lto = "fat"`, `codegen-units = 1`.
A fat-LTO link re-consumes every dependency's LLVM IR, so the final
`cargo build` costs about two minutes however much of the graph is already
compiled. Halving an incremental build is what cargo-chef buys here — not the
near-instant rebuild it buys a project without fat LTO.
