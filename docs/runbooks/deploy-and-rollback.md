# Deploy and roll back

**Nobody has done this.** No cluster has ever run vpay — not a real one, not
kind, not minikube ([../status.md](../status.md),
`deploy/helm/vpay/README.md`). Every command below is written from the chart
and the binaries' own shutdown code, and none of them has been run against a
Kubernetes API server.

**Pin an image built after 2026-09-03.** Both pods probe `/livez` on port
9090; `--observability-bind` and that route landed on 2026-09-03 (step-6
block A) and both binaries' `tests/cli.rs` prove they answer. Against an image
older than that there is nothing on 9090 and the kubelet restarts both pods in
a loop. ~~No image has been published at all yet — see §6 — so today the only
correct value for `images.*.digest` is one that does not exist.~~ **Corrected
2026-09-05: images have been published.** Release run `33929374661`
(2026-09-04, head `33d6c25`) pushed and signed all four; the one this chart
deploys is `ghcr.io/vaam-apps/vpay-server`
`sha256:5485db5e397edd8e672737e676756ca4e9eb56a23fb117a6bc762e0532b50537`,
built from a commit later than 2026-09-03 and so carrying the `/livez`
listener this section requires. That is a real value for
`images.server.digest` — but nobody has pulled it, so see §6 before you rely
on it.

**Changed 2026-09-07 (issue #77): there is one backend image, not two.** This
paragraph named a second, `ghcr.io/vaam-apps/vpay-worker`
`sha256:08667b03…`, and `images.worker` no longer exists — the worker
Deployment runs the `vpay-server` image with `args: ["worker"]`. A values file
that still sets `images.worker` fails `helm lint` rather than being ignored.
The `vpay-worker` package is frozen at that digest and nothing publishes to
it.

---

## 1. Before you upgrade

```bash
just verify                              # the self-checks
just helm-check                          # helm lint + template + 24 guards + kubeconform
helm template <release> deploy/helm/vpay -f my-values.yaml | less   # read it
```

Four things the tooling will not check for you:

1. **The digest is pinned.** Since issue #77 (2026-09-07) there is one
   backend image and both Deployments run it, so "pinned together" is now
   structural rather than something you have to check: one `images.server.digest`
   is what the server and the worker both resolve. It said "both digests are
   pinned, and pinned together" because `vpay-server` and `vpay-worker` share a
   schema and a migration set and running two versions against one database is
   not a supported configuration ([release.md](release.md) §4) — that hazard is
   gone by construction for these two, and remains for a rolling upgrade
   spanning a migration.
2. **The three Secrets exist in the namespace** — `database.existingSecret`,
   `signingKey.existingSecret`, `rails.existingSecret`. The chart creates
   none of them, and the guards catch an empty _value_, not a missing
   _Secret_.
3. **`config.profile` is the profile you meant.** A typo boots cleanly on the
   image's baked **sandbox** configuration — placeholder merchant keys,
   WireMock rail hosts — and reports itself healthy. Nothing catches this;
   `Config::load_with_env` merges the overlay only `if overlay_path.is_file()`.
   Check the rendered mount path against `VPAY_PROFILE` in the output of the
   `helm template` above.
4. **Upgrading from v0.15.0 or earlier deletes staff passwords and TOTP
   seeds, unless you export them first.** The release that carries migrations
   0056 (console keys) and 0058 (human-login tables, VPA-2) runs both, in
   order. 0056 deletes the `password` and `totp` rows of `credentials` and drops
   `staff_sessions`, `staff_sso_attempts` and `oauth_authorization_codes`;
   0058 creates those three tables again **empty** and drops
   `credentials_password_and_totp_are_retired`, but it cannot bring back a row
   a delete removed. So after the upgrade everybody
   is signed out and a half-finished SSO sign-in must be started over, and,
   **if you did not export**, every staff member who signed in with a
   password and a TOTP code must be enrolled again (a new one-time password
   and a new authenticator). **`oidc` credentials are kept**, so staff who
   sign in through an identity provider, and people vaam paired, resume
   without anything. **The passwords and seeds can be kept:** export them
   before the upgrade and put them back after it, as
   [below](#keeping-staff-passwords-across-0056-and-0058). That release also
   refuses a `staff_auth:` config block, so the block goes, but its
   `password_pepper` and `totp_encryption_key` **values are kept** (the export
   is useless without them). **That is true of the releases between VP-6a and
   VPA-3 only: a release that carries VPA-3 accepts the block again**, as one of
   the two blocks of the `human_login` method (step 3), and a deployment that
   wants staff to sign in with a password keeps it. A database that applied 0056 from a pre-release
   build has already lost the rows; the same section says how to take them from
   a backup.

   **A deployment that relies on human login must not upgrade to a release cut
   before VPA-3.** #383 removed the sign-in routes, and VPA-2 (migration 0058)
   restored only the persistence under them: on such a release no route reads or
   writes those tables, nobody signs in by password or TOTP whatever you restore,
   and the export below keeps the data safe without making login work. VPA-3
   restores the password, TOTP, session and authorization-code routes behind
   `auth.methods` (`human_login`); **staff SSO is still not served** (VPA-4), so
   a deployment that signs staff in through an identity provider waits for that
   release too.

### Two things this release changes on `/dash/v1`

From this release a signed `/dash/v1` request that carries **any** `Authorization`
header is a `400` naming `authorization`, so a proxy or gateway in front of the
management pod must not add one. And with `human_login` served and no `dashboard:`
block, the `console_keys` already paired still open `/dash/v1`; writing the
sign-in's blocks does not lock them out.

### Keeping staff passwords across 0056 and 0058

`staff_members` rows survive 0056, and 0058 drops the one CHECK that refused a
`password` or `totp` row, so the rows can be copied out of `credentials`
before the upgrade and copied back after it. That keeps the argon2id hashes
and the TOTP counters, and no staff member is enrolled again.
`a_credential_export_taken_before_0056_is_restored_after_0058` in
`backends/crates/vpay-db/tests/repositories.rs` runs these statements against
a real database and asserts every column of every row comes back; it also
reads this page and fails if the statements below drift from the ones it ran.

**Anything changed after the export is undone by the import.** The file is a
photograph of `credentials` at one moment, and step 6 puts that moment back.
See step 2.

**1. Check the database has not run 0056 yet.** It must return no row:

```sql
SELECT version FROM _sqlx_migrations WHERE version >= 56;
```

**2. Freeze credential changes, then keep the export unchanged until step 6
has succeeded.** The old release serves sign-ins until the new one replaces
it, and a sign-in advances the member's TOTP counter. A password reset, a
forced password change, a new authenticator or a sign-in made after the export
is **not** in the file, and step 6 reverts it:

- a reset password stops working and the old hash is back (with the old
  `must_change`);
- an authenticator enrolled after the export is gone, because 0056 deletes it
  and the file does not have it;
- the TOTP counter returns to the exported step, so a code already used since
  the export can be used again until its time window passes.

So stop the old release from serving sign-ins before you export: take its
server out of the route, or scale it down. Whichever you choose, check after
step 5 that `kubectl get deploy/<release>-server` shows the replicas your values
ask for, because a manual scale-down is not something `helm upgrade` is
guaranteed to undo. Make no credential change by any other door
(`vpay-server staff add` writes none, but a script or a SQL session can) until
step 6 has succeeded.

**3. Keep the pepper and the TOTP key, and decide where they live.** What the
new release does with a `staff_auth:` block depends on whether it carries VPA-3.

- **A release that carries VPA-3 accepts `staff_auth` and `dashboard_client`
  again**, as the two blocks of the `human_login` method of `auth.methods`
  ([ADR-0037](../adr/0037-authentication-methods-are-deployment-configuration.md)).
  To keep staff signing in with a password and a TOTP code, leave both blocks in
  the values you hand to step 5, **unchanged**, and the pepper and the TOTP key
  with them: the exported argon2id hashes and sealed TOTP seeds verify only under
  those two values. The method must be permitted (`auth.methods` absent, or
  naming `human_login`), and the blocks must be **both** there: one without the
  other, or a block with `human_login` left out of an explicit list, exits `78`
  at boot (`AuthMethodNotConfigured`, `AuthMethodDisabled`). A `staff_auth.sso`
  block is refused as an unknown key until VPA-4: delete it. To stop serving the
  sign-in instead, delete both blocks, as the next bullet.
- **A release cut between VP-6a and VPA-3 refuses a `staff_auth:` block** when it
  loads its configuration, before it runs a migration: the process exits `78`
  with `ConfigError::RetiredKey` and no pod becomes ready. So the block must be
  gone from the base file and every profile overlay in the values you hand to
  step 5 (not earlier: the old release reads its configuration again if a pod
  restarts), **but the block holds `staff_auth.password_pepper` and
  `staff_auth.totp_encryption_key`**: copy both into your secret store now,
  unchanged, and keep them until you upgrade to a release that carries VPA-3.
  Destroying or rotating either makes the file in step 4 unusable
  ([ADR-0017](../adr/0017-staff-authentication.md) says what losing each costs).

[dashboard-auth.md](../flows/dashboard-auth.md#upgrading-from-before-vp-6a)
step 4 says the same.

**4. Export, before any process of the new release starts.** Migrations run at
boot, in the first process of the new image to start, **in any mode** — `serve`,
`worker`, and a one-off `vpay-server staff add` alike; there is no separate
migrate job. So export before `helm upgrade`, not during it. Run `psql` as a
role that can read `credentials`, after `umask 077` so the file is private.
`\copy` is one line, and writes the file on the machine running `psql`; the
`SET` pins the date style so the timestamps are written in the one form that
step 6 reads back, whatever the role's or the server's default is:

```text
SET datestyle = ISO;
\copy (SELECT id, staff_member_id, kind, material, issuer, subject, counter, must_change, expires_at, created_at, updated_at FROM credentials WHERE kind IN ('password', 'totp') ORDER BY id) TO 'creds.csv' CSV HEADER
```

The eleven columns are every column of `credentials`, and the test fails if a
migration adds a twelfth without this page. The file holds argon2id digests and
sealed TOTP seeds: keep it off shared storage, and delete it once step 6 has
succeeded.

**5. Upgrade, and wait for 0058.** `helm upgrade` as in §2, with the
`staff_auth` block as step 3 decided. Confirm the migration ran before step 6:

```sql
SELECT max(version) FROM _sqlx_migrations;  -- 58 or higher
```

**6. Import.** After the migration, never before it: until 0058 drops
`credentials_password_and_totp_are_retired`, the database refuses the rows. Use
the same `SET` as step 4, in the same `psql` session:

```text
SET datestyle = ISO;
\copy credentials (id, staff_member_id, kind, material, issuer, subject, counter, must_change, expires_at, created_at, updated_at) FROM 'creds.csv' CSV HEADER
```

It is one `COPY`, so it lands whole or not at all. It fails, and inserts
nothing for anybody, in two cases, and the staging form below handles both:

- **The singleton-kind rule.** A staff member holds at most one `password` and
  one `totp`: `credentials_one_singleton_kind_per_staff_member` is a unique
  index on `(staff_member_id, kind)` for those kinds. If a member was given a
  new password or authenticator between step 5 and step 6, the import fails on
  that index.
- **A member who no longer exists.** A row whose `staff_member_id` is not in
  `staff_members` fails the foreign key.

Either delete the offending rows and run the import again, or load the file
into a staging table and let the database skip what cannot be restored. The
staging form restores a row only if its member still exists, and a row that
collides with one already in `credentials` (on its id or on the singleton
index) is skipped and the existing row is kept. The decision is **per row, not
per member**: a member can get the password back and not the authenticator, or
the reverse. The last statement lists every exported row that did not come back
as it was exported, whatever the reason:

```text
SET datestyle = ISO;
CREATE TEMP TABLE credentials_restore (LIKE credentials);
\copy credentials_restore (id, staff_member_id, kind, material, issuer, subject, counter, must_change, expires_at, created_at, updated_at) FROM 'creds.csv' CSV HEADER
INSERT INTO credentials (id, staff_member_id, kind, material, issuer, subject, counter, must_change, expires_at, created_at, updated_at) SELECT id, staff_member_id, kind, material, issuer, subject, counter, must_change, expires_at, created_at, updated_at FROM credentials_restore WHERE staff_member_id IN (SELECT id FROM staff_members) ON CONFLICT DO NOTHING;
SELECT r.id, r.staff_member_id, r.kind FROM credentials_restore r WHERE NOT EXISTS (SELECT 1 FROM credentials c WHERE to_jsonb(c) = to_jsonb(r)) ORDER BY r.id;
```

Each row the last statement lists is a credential that member must be given
again (a new one-time password or a new authenticator), or whose existing
credential you keep on purpose.

**If 0056 already ran,** the rows are gone from this database and only a backup
can supply them. Restore one taken before 0056 into a scratch instance
([restore-from-backup.md](restore-from-backup.md) §2), run the export of step 4
against it, and load it into the production database, after 0058, with the
**staging form**: it restores only the staff members who still exist there and
lists the rest. The pepper and the TOTP key of step 3 must be the ones in force
when the backup was taken.

## 2. Upgrading

```bash
helm upgrade --install <release> deploy/helm/vpay \
  -f my-values.yaml \
  --atomic \
  --timeout 10m
```

**`--atomic`.** It implies `--wait`, so helm blocks until every Deployment
reports ready, and if the upgrade fails or times out it **rolls the release
back automatically**. That is the behaviour you want for a payment gateway:
the alternative is a half-applied release with a failing Deployment and a
human deciding under pressure. Read §4 first, though — an automatic rollback
of the _manifests_ does not undo a migration, and it does not undo a signing
key rotation.

**`--timeout` must exceed a full rollout**, and a rollout includes both pods
draining. With the defaults that is `terminationGracePeriodSeconds` (35 s) per
pod plus start-up time, and start-up includes running migrations against a
cold database. 10m is a starting point, not a measurement.

Watch it:

```bash
kubectl rollout status deploy/<release>-server --timeout=10m
kubectl rollout status deploy/<release>-worker --timeout=10m
kubectl get pods -l app.kubernetes.io/instance=<release> -w
```

### What the two workloads do differently

|                     | server                                                                                                                      | worker                                                                                                                                                                  |
| ------------------- | --------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Strategy            | rolling (Deployment default)                                                                                                | **`Recreate`** — the old worker is gone before the new one starts. It serves no traffic, so there is nothing to keep available, and overlapping claim loops buy nothing |
| PodDisruptionBudget | `minAvailable: 1`, so a node drain cannot take both replicas                                                                | none                                                                                                                                                                    |
| Probes              | liveness `/livez` :9090, readiness `/healthz` :8080 (a real `SELECT 1`), startup `/livez` :9090 with `failureThreshold: 30` | liveness + startup `/livez` :9090; no readiness — nothing routes to it                                                                                                  |

The startup probe exists because migrations run at boot, before the listener
binds, and against a cold database that is the slowest part of a start. It is
what stops a slow migration being counted as a liveness failure.

## 3. The grace-period guard, and what draining actually does

The chart refuses to render if
`terminationGracePeriodSeconds < shutdownGraceSeconds + 5`. It is a Helm
`fail` in `templates/_validate.tpl` named `grace-period`, so `helm lint`,
`helm template`, `helm install` and `helm upgrade` all abort with:

```text
vpay chart guard "grace-period": terminationGracePeriodSeconds is 25 but
shutdownGraceSeconds is 25; the kubelet would SIGKILL the process while it is
still draining in-flight work. Set terminationGracePeriodSeconds to at least 30.
```

Defaults are 35 and 25. **Never raise `shutdownGraceSeconds` without raising
`terminationGracePeriodSeconds` with it** — the guard will stop you at render
time, which is the point.

Why the ordering matters, per binary — this is
[../status.md](../status.md)'s `--shutdown-grace-seconds` row, not a
description of intent:

- **`vpay-server`**: `serve_with_bounded_drain` races the axum drain against a
  `shutdown_grace_seconds` clock and **exits non-zero if the clock wins**,
  logging that in-flight work was cut off. ~~No test exercises that timeout
  path.~~ Corrected 2026-09-03 (Step 6 review pass), and §6 below says the
  same: `an_in_flight_request_that_outlasts_the_grace_period_is_exit_1_and_says_so`
  (`backends/apps/vpay-server/tests/cli.rs`) holds a real request open on a
  real socket — a client that promises `Content-Length: 200` on
  `POST /v1/oauth/token` and sends 29 bytes, so the `Form` extractor is
  genuinely pending inside the router — across a real SIGTERM, and asserts
  exit **1** plus the forced-cutoff WARN. It synchronises on the
  `Expect: 100-continue` interim response rather than a sleep, so "the head
  was parsed and routed before the signal arrived" is proven by the server
  rather than guessed at with a timer. What it does **not** prove is that a
  slow _rail_ drains the same way (the drain is a property of the connection,
  not of what the handler awaits), and nothing has measured the bound under
  real load.
- **`vpay-worker`**: this half is real and has a test. The grace clock starts
  **when the signal arrives**, not at boot. Tasks stop claiming at the top of
  an iteration, so a clean drain settles every claimed job and hands back no
  lease (`LoopReport::released == 0`). On timeout the remaining tasks are
  aborted, `jobs::release_all` returns every lease this worker still holds,
  and the binary exits **1**.
  `a_drain_that_runs_out_of_grace_releases_every_lease_it_still_holds`
  (`backends/tests/integration/tests/worker_e2e.rs`) drives exactly that path
  against a real Postgres and asserts zero rows still leased afterwards;
  delete the `release_all` call and it fails.

So **a non-zero exit during a rolling update means work was cut off, not that
shutdown failed.** The log line is the evidence. What a kubelet reports for a
container that exits 1 while terminating has never been observed here.

There is **no `preStop` hook**, deliberately. The race a sleep-based hook
covers — a process serving before it can handle a signal — is already closed:
`ShutdownSignals::install()` is the first statement in both `main`s.

## 4. Rolling back

### The manifests

```bash
helm history <release>
helm rollback <release> <revision> --wait --timeout 10m
```

Equivalently, and preferably for an image change: pin the **previous digests**
in your values file and `helm upgrade` forward to them. A rollback and a
forward-upgrade-to-old-digests produce the same pods; the second leaves your
values file telling the truth about what is running.

### The three things a rollback does not undo

**1. Migrations.** They run at boot and there are **no down-migrations in this
repository**. An older image against a newer schema is not a supported
configuration and nothing checks it. If a release contained a migration,
rolling the image back is a decision to run old code against a new schema —
read the migration first.

**2. A signing-key rotation.** If the release rotated the signing key,
rolling the **Secret** back to the retired key crash-loops the server with
**exit 78** (`DbError::SigningKeyRetired`), not 69. `helm rollback` does not
touch a Secret the chart does not own, so this only bites if somebody rolls
the Secret back by hand — but that is exactly what "undo the deploy" means to
most people. [rotate-signing-key.md](rotate-signing-key.md) §3.

**3. Anything a rail already did.** Rolling back does not un-submit a charge.
Charges left mid-flight are chased by the poll ladder once a worker is
running; see [unresolved-charges.md](unresolved-charges.md).

### If `--atomic` rolled back for you

Find out _why_ before re-running. The automatic rollback restored the
manifests; it did not restore a migration and it did not tell you what
failed.

```bash
kubectl get pods -l app.kubernetes.io/instance=<release>
kubectl logs deploy/<release>-server --previous | tail -50
kubectl get events --sort-by=.lastTimestamp | tail -30
```

| Exit code | Meaning                                                     | Usually                                                                                                                                                                                                                                        |
| --------- | ----------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **78**    | configuration — fix the deploy, restarting will not help    | a missing `${VAR}` from the rails Secret (**both** binaries), an unreadable or wrong signing key, a rail named without its required credentials ([ADR-0012](../adr/0012-rail-configuration-requirements-in-config.md)), or a **retired `kid`** |
| **69**    | the database is unavailable — transient, waiting is correct | `DATABASE_URL` wrong or Postgres unreachable. Note the sqlx acquire timeout makes this take a few seconds                                                                                                                                      |
| **1**     | the drain clock elapsed (§3), or an unclassified error      | in-flight work was cut off                                                                                                                                                                                                                     |

Exit 78 in a crash loop is never fixed by restarting. That distinction is the
whole reason the codes are split.

## 5. Changing configuration without changing the image

The overlay is mounted with `subPath`, so **the mounted file does not update
when the ConfigMap changes**. The chart closes that loop with a
`checksum/config-overlay` pod annotation: editing `config.overlay` in your
values changes the annotation, which makes `helm upgrade` a rolling restart
rather than a silent no-op. This is consistent with [ADR-0003](../adr/0003-yaml-configuration.md)
— configuration loads once, at boot, and there is no hot reload.

Editing the ConfigMap **directly** with `kubectl edit` changes nothing at all,
because nothing re-reads the file and nothing restarts. Go through helm.

### Staff who lost a password or an authenticator

There is no `staff remove`, `staff disable` or `staff reset-password` command
(gap, [status/backend.md](../status/backend.md)), and no SQL can set a password,
because a hash needs the deployment's pepper. Use `psql` as a role that can write
`staff_members` and `credentials`. The address is the one `staff add` stored, in
lower case. Confirm who is asking by another channel first: both recoveries let
whoever makes the request back in.

**Lost authenticator.** The member keeps their password and gets a new second
factor. Enrolment is the absence of a TOTP credential, so deleting it, and the
sessions that were issued under the old one, makes the next password sign-in
enrol a fresh authenticator:

```sql
BEGIN;
DELETE FROM credentials
 WHERE kind = 'totp'
   AND staff_member_id = (SELECT id FROM staff_members WHERE email = 'ada@example.test');
DELETE FROM staff_sessions
 WHERE staff_id = (SELECT id FROM staff_members WHERE email = 'ada@example.test');
COMMIT;
```

**Lost password.** Read the member's attributes, delete the member, and add them
again with the same ones. `staff add` refuses a second account for an address
while the row exists, and deleting only the password credential would leave a
member nobody can issue a password to:

```sql
SELECT id, merchant_id, display_name, is_admin FROM staff_members WHERE email = 'ada@example.test';
DELETE FROM staff_members WHERE email = 'ada@example.test';
```

```text
vpay-server staff add --merchant <merchant_id> --email ada@example.test --name '<display_name>' [--admin]
```

The last command prints the new one-time password alone on stdout. The delete
cascades to the member's sessions, authorization codes, credentials **and console
keys**, so a paired key has to be paired again, and the member's `stf_…` id
changes. Nothing audits either recovery: write down who asked and who ran it.

## 6. What is unproven

Everything operational on this page. Specifically:

- No cluster has run this chart. Nothing here is evidence about scheduling,
  admission, probe behaviour, `readOnlyRootFilesystem`, NetworkPolicy
  enforcement, or PDB behaviour during a drain.
- ~~**The liveness probes point at a listener no image has.**~~ Corrected
  2026-09-03: the listener landed the same day. See the note at the top for
  what is still true about older images.
- ~~**No image exists** at `ghcr.io/vaam-apps/vpay-server` or `-worker`;
  `release.yml` has never run, so every digest a values file could pin today
  would be invented~~ ([release.md](release.md)). ~~**Retired 2026-09-05:**
  `release.yml` has run 13 times on `master`, 12 green; the latest,
  `33929374661`, pushed and cosign-signed a manifest list for each of the four
  images.~~ **Corrected 2026-09-20:** that tally is stale — read
  [release.md](release.md)'s own header for the current run count, and note
  there are three images now, not four (`vpay-worker` was retired by issue
  #77 on 2026-09-07; this page's own header, above, already says so). A
  digest pinned from a green run is still measured, not invented. **What
  replaces this bullet:** nobody has ever _pulled_ one of those images, GHCR
  package visibility is unmeasured (the available token has no
  `read:packages` scope and anonymous pull is refused), and no `cosign verify`
  has been run — so a cluster that cannot authenticate to GHCR would fail at
  `ImagePullBackOff` and nothing here would have predicted it.
- ~~The server's drain timeout has no test.~~ Corrected 2026-09-03: it has
  one. `an_in_flight_request_that_outlasts_the_grace_period_is_exit_1_and_says_so`
  in `backends/apps/vpay-server/tests/cli.rs` holds a real request open on a
  real socket across a real SIGTERM and asserts exit 1 plus the forced-cutoff
  WARN. What it does not prove is that a slow _rail_ produces the same
  outcome; the drain is a property of the connection, not of what the handler
  awaits.
- **No Prometheus has scraped either process.** `/metrics` is served and its
  contents are asserted by both binaries' `tests/cli.rs`, but no alert rule
  has ever been evaluated against a real series.
- `--atomic`'s behaviour here is helm's documented behaviour, not something
  observed against this chart.
- 10m is a guessed timeout. Nothing has measured a rollout.
