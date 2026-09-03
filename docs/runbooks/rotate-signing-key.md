# Rotate the OAuth signing key

**Nobody has done this on a deployment.** No vpay has ever run outside a
developer machine and CI, so no signing key has ever been rotated in anger.
The mechanics below are read off the code and its tests; the `kubectl` steps
are not.

The one thing to take from this page before anything else:

> **Rotation is a restart. Rolling back to a retired key is a crash loop that
> exits 78, and rolling the Deployment back does not fix it.**

---

## 1. What rotation actually is

`vpay-server` loads one RSA private PEM from
`--oauth-signing-key-file` / `VPAY_OAUTH_SIGNING_KEY_FILE` at boot,
`vpay_api::op::keys::LoadedSigningKey::from_file` parses it into an
`authkestra_engine::TokenManager`, and **`TokenManager` holds that one key for
the life of the process**. Nothing re-reads the file. There is no rotation
endpoint, no SIGHUP, no timer. Rotating means restarting with a different
Secret, and that is the whole mechanism.

The `kid` is the **RFC 7638 thumbprint of the public JWK** — a function of the
key material itself, not of the filename or the pod. Two pods holding the
same PEM announce the same `kid`; a new key gets a new `kid` whether you meant
it to or not.

At boot, after loading the key, the server calls
`LoadedSigningKey::ensure_active_in_database` →
`vpay_db::ensure_active_signing_key`, which takes a Postgres advisory lock and
does read-decide-write in **one transaction**, so N replicas booting on the
same Secret rotate the record once between them. Three outcomes:

| The `kid` in the Secret | What happens |
|---|---|
| already the active row | `ActivationOutcome::AlreadyActive` — **no row is written at all** |
| new to this database | `Rotated { previous }` — the old row is retired with `expires_at = now() + 24 h`, the new one inserted, in one transaction |
| a row this database has **already retired** | `DbError::SigningKeyRetired` → `Category::Configuration` → **exit 78** |

`oauth_signing_keys` holds **public halves only** — migration `0010` dropped
`private_key_pem` deliberately. The private key exists only in the Secret.

**The overlap window is 24 hours** (`vpay_api::op::keys::ROTATION_OVERLAP`).
`/v1/oauth/jwks.json` publishes `WHERE active OR expires_at > now()`, so a
just-retired key keeps verifying tokens it already signed. Access tokens live
900 s (`vpay_api::op::ACCESS_TOKEN_TTL_SECS`), so 24 h dwarfs the longest
token that could still be in flight — the only property under test
(`the_rotation_overlap_dwarfs_the_access_token_ttl_it_has_to_cover`). Neither
number is configurable and neither is recorded in an ADR; whether 24 h is
*right* is an open maintainer question in [../roadmap.md](../roadmap.md).

## 2. Rotating

```bash
# 1. Generate a new key. 3072-bit RSA PKCS#8, mode 0600, refuses to
#    overwrite an existing file. It prints the `kid` and the public JWK —
#    write the kid down, it is what you check for in §2's confirmation and
#    in oauth_signing_keys.
#
#    ./newkey holds an unencrypted private key on your workstation from
#    here until step 4. Do not skip step 4, and do not generate it into a
#    directory something syncs or backs up.
umask 077
cargo xtask gen-signing-key --out ./newkey

# 2. Replace the Secret the chart's `signingKey.existingSecret` names.
#    The key inside it must keep the name `signingKey.key` (default
#    `oauth-signing-key.pem`) or the mount path changes and the server
#    exits 78 naming a file it cannot find.
kubectl create secret generic vpay-oauth-signing-key \
  --from-file=oauth-signing-key.pem=./newkey/oauth-signing-key.pem \
  --dry-run=client -o yaml | kubectl apply -f -

# 3. Restart. A Secret volume update does NOT restart anything, and even if
#    the file on disk changed, nothing re-reads it.
kubectl rollout restart deploy/<release>-server
kubectl rollout status  deploy/<release>-server --timeout=5m

# 4. Destroy the local copy, once §2's confirmation below has passed and NOT
#    before — if the rollout has to be rolled back you need this file. The
#    Secret is the only copy that should outlive this procedure.
shred -u ./newkey/oauth-signing-key.pem 2>/dev/null || rm -f ./newkey/oauth-signing-key.pem
rmdir ./newkey
```

Step 4 is deliberately after the confirmation in the next section: a rotation
that has not been confirmed is one you may have to undo, and the key file is
the only thing that can undo it.

Only the **server** mounts the signing key. `vpay-worker-bin` takes no
`--oauth-signing-key-file`, issues no token, and the chart deliberately does
not mount the Secret there. There is nothing to restart on the worker side.

### Confirming it took

```bash
# The log line ensure_active_signing_key writes on a real rotation:
#   "rotated the active signing key"  with kid, previous_kid, retire_previous_at
# On every replica after the first:
#   "signing key is already the active one; no rotation"
kubectl logs deploy/<release>-server | grep -i 'signing key'
```

```sql
-- Exactly one active row; the previous key retired with an expiry ~24h out.
SELECT kid, active, expires_at FROM oauth_signing_keys ORDER BY created_at;
```

```bash
# JWKS should carry BOTH kids during the overlap window.
curl -s https://<host>/v1/oauth/jwks.json | jq '.keys[].kid'
```

Two `kid`s is the correct state for the next 24 hours, not a symptom. One
`kid` immediately after a rotation means the old row's `expires_at` has
already passed, and tokens signed with it stop verifying.

## 3. Rolling back — read this before you type `rollout undo`

**A rollback to a retired `kid` crash-loops the server with exit 78, not 69.**

`vpay_db::ensure_active_signing_key` refuses to re-activate a retired row.
It does not silently resurrect the old key, because re-publishing a key that
was deliberately retired is a policy decision nobody has made
([../roadmap.md](../roadmap.md), "Open — signing-key rotation overlap
window"). It returns `DbError::SigningKeyRetired { kid, retired_at }`, which
classifies as `Category::Configuration`, which is exit **78**.

The number is the point. 78 (`EX_CONFIG`) tells a supervisor *fix the
deploy*; 69 (`EX_UNAVAILABLE`) tells it *wait for the database*. This failure
never resolves by waiting, so a 69 here would be an infinite crash loop
against a database that is perfectly healthy.

Pinned by two tests:

- `a_rollback_to_a_retired_signing_key_exits_78_and_a_dead_database_still_exits_69`
  — `backends/apps/vpay-server/src/main.rs:843`. A unit test over
  `exit_code_for`: a `DbError::SigningKeyRetired` wrapped in the same
  `.context(..)` `run` applies maps to 78, and — the control that makes it
  mean something — a `DbError::Connect(sqlx::Error::PoolTimedOut)` still maps
  to 69. It exercises the classification and the chain walk; it does **not**
  spawn a process, so it is not evidence about what a pod does.
- `ensure_active_signing_key_refuses_to_reactivate_a_retired_kid` —
  `backends/crates/vpay-db/tests/repositories.rs:558`, against a real
  Postgres. This is the one that proves the database layer refuses.

### What to do instead

**Roll forward.** Either:

- put the **current** key back in the Secret and restart (the fastest fix if
  the rollback was accidental — the current `kid` is still `active`, so this
  is the `AlreadyActive` path and writes nothing), or
- generate a **new** key and rotate to it (§2), which is a normal rotation.

Do **not** flip `active` by hand in `oauth_signing_keys` to resurrect a
retired key. Its `expires_at` is in the past or nearly so, the partial unique
index `one_active_signing_key` will fight you, and you would be making the
rotation-policy decision the code declined to make.

### If the image is also being rolled back

`kubectl rollout undo` changes the image, not the Secret. If a deploy rotated
the key *and* the image, undoing it leaves the new Secret with the old image
— which is fine — while undoing the Secret as well is the case above. And
independently: **a migration that has run is not undone by an older image.**
There are no down-migrations here. See [release.md](release.md) §5.

## 4. Related failures that look like this one

| Symptom | Cause | Where |
|---|---|---|
| exit 78, "loading the OAuth signing key from …" | Secret missing, wrong key name inside it, or an unreadable file mode | `signingKey.defaultMode` is `0440`, not `0400` — an `fsGroup`ed Secret volume is owned `root:<fsGroup>` and UID 65532 needs the group bit. Chart README |
| exit 78, not an RSA key / under 2048 bits | wrong key material | `LoadedSigningKey::from_file` refuses both, without echoing the PEM |
| exit 78 naming a `kid` and a retirement instant | **this runbook, §3** | |
| exit 69 | the database, genuinely | Not a key problem. Wait, or look at Postgres |
| Every `/v1` call 401s, nothing in the logs | issuer/audience mismatch, not a key problem | [../flows/merchant-auth.md](../flows/merchant-auth.md) |

## 5. What is unproven here

- **No rotation has ever been performed on a deployment.** The evidence is
  unit and integration tests against ephemeral containers.
- **The `kubectl` steps in §2 have never been run.** No cluster has ever run
  vpay ([../status.md](../status.md)).
- **The 24 h overlap has never elapsed anywhere.** Nothing has observed a
  retired key dropping out of `/jwks.json` at its `expires_at`.
- **The PEM is not zeroized.** `LoadedSigningKey::from_file` reads it into a
  `String` that is dropped normally, so key bytes may linger in freed heap —
  stated deliberately in `vpay_api::op::keys`'s module docs, not fixed.
