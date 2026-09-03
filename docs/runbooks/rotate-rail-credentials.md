# Rotate a rail credential, and revoke a merchant client

Two different jobs on one page, because operators reach for the same word for
both and the mechanisms have nothing in common:

- **§1–§4 — a rail credential** (MTN, Orange): an environment variable from a
  Kubernetes Secret. Rotating it is a Secret edit plus a restart of **both**
  workloads.
- **§5 — a merchant's own credential**: vpay holds no merchant secret at all,
  so there is nothing to rotate. Revoking one needs the
  [ADR-0010](../adr/0010-merchant-auth-private-key-jwt.md) **dual-authority
  check** — YAML *and* the database. That check is the reason this page
  exists; [../roadmap.md](../roadmap.md) recorded that no runbook documented
  it.

**Nobody has done either on a deployment.** No cluster has run vpay, and no
credential in this repository has ever been a real one — every rail call ever
made went to a WireMock host ([../status.md](../status.md)).

---

## 1. What a rail credential is here

Rail credentials live in YAML as `${VAR}` placeholders, resolved from the
process environment at boot. From `config/application.yml`:

| Rail | `settings` (printed in full by `ProviderHost`'s `Debug`) | `credentials` (redacted) |
|---|---|---|
| `mtn_momo` | `target_environment`, `api_user` = `${MTN_API_USER}` | `subscription_key` = `${MTN_SUBSCRIPTION_KEY}`, `api_key` = `${MTN_API_KEY}` |
| `orange_money` | `env`, `lang` | `merchant_key` = `${ORANGE_MERCHANT_KEY}`, `client_id` = `${ORANGE_CLIENT_ID}`, `client_secret` = `${ORANGE_CLIENT_SECRET}` |

Six rail variables in this table, plus `MERCHANT_WEBHOOK_SECRET`
(`docs/flows/webhooks.md`, Step 5) for seven **on this branch, as of
2026-09-03**. The list is not fixed and is not owned by the chart: it is
whatever the `config/application.yml` baked into the image you are deploying
references, and it grows as features land. Read it off that revision at
upgrade time rather than trusting this table:

```bash
grep -o '${[A-Z_]*}' config/application.yml | sort -u
```

All of them are supplied by the single Secret the chart's
`rails.existingSecret` names, projected with `envFrom.secretRef` — so
`kubectl describe pod` shows the variable *names* and never the values. A key
the image needs and the Secret lacks is exit 78, not a missing feature.

**A rail named in configuration without its required keys is exit 78 at boot,
on both binaries**, not a failure at the first live charge. The rule is
`vpay_config::config::REQUIRED_RAIL_KEYS`, the one sanctioned place outside an
adapter that matches on a provider code ([ADR-0012](../adr/0012-rail-configuration-requirements-in-config.md)):
MTN requires `target_environment` + `api_user` and `subscription_key` +
`api_key`; Orange requires `merchant_key`, `client_id`, `client_secret`. An
unresolved `${VAR}` is a **named fatal error**, never an empty string.

`api_user` is a `settings` key deliberately — it is an identifier, not a
bearer secret — but it still comes from the same Secret, and omitting it is
still exit 78.

## 2. Rotating

Rotate at the rail first (MTN's or Orange's own portal issues the new
credential), then here.

**Never `--from-literal` for these.** A credential on a command line is in
your shell history, in `ps` output for every user on the box while the command
runs, and in whatever your terminal multiplexer logs. Write the values to a
file only you can read, use it, and delete it.

```bash
# Replace the whole Secret. Every key must be present: envFrom projects the
# Secret as a set, and a key you drop becomes an unresolved ${VAR}, i.e.
# exit 78 on both binaries at the next start. The exact list is read from
# `config/application.yml` at upgrade time — see §1.
umask 077
env_file="$(mktemp)"
trap 'shred -u "$env_file" 2>/dev/null || rm -f "$env_file"' EXIT
chmod 600 "$env_file"

# Edit, do not echo: one KEY=value per line, no quotes, no export.
"${EDITOR:-vi}" "$env_file"

kubectl create secret generic vpay-rails \
  --from-env-file="$env_file" \
  --dry-run=client -o yaml | kubectl apply -f -

# Explicitly, now — the trap is only a backstop for an interactive shell you
# might not close for hours.
shred -u "$env_file" 2>/dev/null || rm -f "$env_file"

# Restart BOTH. Environment variables are read once, at process start.
kubectl rollout restart deploy/<release>-server
kubectl rollout restart deploy/<release>-worker
kubectl rollout status  deploy/<release>-server --timeout=5m
kubectl rollout status  deploy/<release>-worker --timeout=5m
```

**Both, not just the server.** The worker reads the same configuration and
calls the same rails — it is the process that polls a submitted charge and
settles it. A server on the new credential and a worker on the old one is a
deployment where payments are submitted and never confirmed.

Restart the **server first**. The worker's `Recreate` strategy takes it fully
down and back up, and a window in which nothing polls is less bad than a
window in which nothing accepts.

## 3. Confirming it took, and the failure that looks like a rail outage

There is no endpoint that reports which credential a pod holds, by design —
`ProviderHost`'s hand-written `Debug` redacts `credentials`. What you can
check:

```bash
# Both pods are up and past their startup probe, i.e. neither exited 78.
kubectl get pods -l app.kubernetes.io/instance=<release>

# Names only, no values — this is what envFrom gives you.
kubectl describe pod <server-pod> | grep -A 10 'Environment Variables from'
```

Then watch `provider_requests` for the next real call:

```sql
SELECT provider_code, operation, status_code, error_kind, sent_at
FROM provider_requests
WHERE sent_at > now() - interval '15 minutes'
ORDER BY sent_at DESC;
```

**`error_kind = 'misconfigured'` is the signature of a bad credential.** It
means the adapter refused before or because of a bad credential, header or
`base_url` — *fix the deployment, not the mapping*. It is not
`provider_unavailable` (the rail is unreachable) and not `provider_error`
(the rail said something the adapter could not parse); see
[provider-error-rate.md](provider-error-rate.md).

A rotation that half-landed reads as a rail incident: authentication failures
across every call to one rail, starting exactly when you restarted. The
timing is the diagnosis.

## 4. Rolling back a rail credential

Put the previous value back in the Secret and restart both workloads again.
There is no state in the database tied to a rail credential, so there is
nothing else to undo — unlike the signing key, where a rollback is a crash
loop ([rotate-signing-key.md](rotate-signing-key.md) §3).

The rail's own side may not be as forgiving: if the old credential was
revoked at the provider when the new one was issued, rolling back locally
restores a credential the rail no longer honours. Check that before you
restart, not after.

## 5. Revoking a merchant client — the dual-authority check

**vpay stores no merchant secret, in any form, in any table**
([ADR-0010](../adr/0010-merchant-auth-private-key-jwt.md)). A merchant holds
its own private key and proves possession by signing a `private_key_jwt`
assertion; vpay holds only the **public** JWK, in YAML. So there is nothing
to rotate on vpay's side, and "rotating a merchant credential" means the
merchant generates a new keypair and sends a new public JWK — which is a pull
request, reviewed and deployed, because [ADR-0003](../adr/0003-yaml-configuration.md)
has no config hot reload.

Revoking is the operation that has to be fast, and it has **two authorities**:

| Authority | Where | What it decides |
|---|---|---|
| `merchant_clients` in YAML | `config/application.yml`, loaded at boot | **Identity.** Does this client exist; what is its public JWK, its `merchant_id`, its scopes, its audience |
| `disabled_clients` | the database (`client_id`, `disabled_at`, `reason`) | **Subtraction only.** Never grants access; only takes it away, with no deploy |

> A correct answer to "is this client allowed right now" needs **both**. YAML
> alone is not the answer, and neither is the table.

That is ADR-0010's own consequence paragraph, and it is why this section
exists.

### Revoke now (no deploy)

```sql
INSERT INTO disabled_clients (client_id, reason)
VALUES ('<client_id>', 'why, and who decided')
ON CONFLICT (client_id) DO UPDATE SET reason = EXCLUDED.reason;
```

It takes effect on the **next token request**, with no restart:
`vpay_api::op::clients::YamlClientStore::find_client` consults it, and
`find_client` is step 1 of every token request for every grant. A disabled
client is reported as "no such client", so the token endpoint cannot be used
to discover whether a merchant exists but is suspended.

**What it does not do: it acts on issuance only.** An access token already
issued stays valid for the rest of its TTL — 900 s
(`vpay_api::op::ACCESS_TOKEN_TTL_SECS`). Nothing in this repository shortens
that window, and there is no revocation endpoint. If 15 minutes of continued
access is unacceptable for the incident you are handling, revoking here is
not sufficient and you need a decision above this runbook. That gap is
[ADR-0009](../adr/0009-dashboard-oidc-provider.md)'s open revocation
question.

### Un-revoke

```sql
DELETE FROM disabled_clients WHERE client_id = '<client_id>';
```

A no-op on a client that was never disabled.

### Check both authorities

Neither on its own is an answer:

```bash
# 1. Does the client exist at all, and with which JWK? YAML is authoritative.
grep -n -A 20 'merchant_clients' config/application-<profile>.yml

# 2. Has it since been disabled? The database is authoritative for that.
```

```sql
SELECT client_id, disabled_at, reason FROM disabled_clients ORDER BY disabled_at DESC;
```

Answer the two questions in that order. A `client_id` in the table but not in
YAML is not "disabled" — it does not exist, and the row is a leftover from a
client that was removed from configuration.

**After a database restore, re-run query 2.** A restore to an instant before
a revocation silently re-enables the client, and nothing reports it —
[restore-from-backup.md](restore-from-backup.md) §5.

### Removing a client permanently

Delete it from `merchant_clients` in YAML and deploy. Until that deploy
completes on every replica, `disabled_clients` is what is holding the door;
do not remove the row until the deploy has rolled out. And note ADR-0010's
onboarding consequence in reverse: with no hot reload, old and new pods
disagree about the client list for the length of a rolling deploy.

## 6. What is unproven

- **No real rail credential has ever been used.** Every call this code has
  made went to a `wiremock/wiremock` host. `error_kind = 'misconfigured'` as
  the signature of a bad credential is read off the classification table, not
  off a real MTN or Orange rejection.
- **No cluster has run vpay**, so every `kubectl` command here is unexercised.
- **`disable_client` / `enable_client` are called by no shipping code.** The
  functions exist in `vpay_db` and are integration-tested against a real
  Postgres; an operator flips the row by hand, which is what §5 documents.
- The kill switch's enforcement is tested
  (`a_disabled_client_is_refused_with_invalid_client_and_401` and friends),
  and those tests have run against containers — but never against a
  deployment, and never against a real merchant.
