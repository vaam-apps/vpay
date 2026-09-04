# Release: cut a tag, verify a signature, pin a digest

**Nobody has done this.** No tag has been pushed, `.github/workflows/release.yml`
has never run, no image exists at `ghcr.io/vaam-apps/vpay-*`, and nothing has
been signed. This runbook is written from the workflow file and the documented
behaviour of the tools it calls. Read [../status.md](../status.md) before you
trust a step here; the first real run is what turns it from a plan into a
procedure.

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

* No `release.yml` run exists. Not one image has been built by it, pushed,
  merged into a manifest list or signed.
* `aarch64-unknown-linux-musl` has never been compiled — not in CI, not
  locally. The arm64 half of every manifest list is unbuilt code paths in a
  workflow file.
* `cosign verify` has never been run against anything from this repository, so
  the regexp in §3 is derived from the workflow's `on:` block and Fulcio's
  documented identity format, not from a certificate anyone has read.
* `just release-dry-run` exercises the Dockerfiles and the chart. It exercises
  neither the registry, nor the attestations, nor the signature.
