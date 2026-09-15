# Reproducing a payment against MTN's real sandbox (the `live` profile)

**Who this is for.** A developer (or reviewer) who wants to take a real payment
through the real MTN **sandbox** on their own machine, end to end — server,
worker, the payer checkout page, and the staff dashboard — and watch it settle.
This is the reproducible form of the work first done on 2026-09-15
([`../status.md`](../status.md)).

**What this page is.** A step-by-step runbook whose every command is the one
actually run. Follow it top to bottom and you end with a `succeeded`
`payment_intents` row that MTN's sandbox settled for you.

**What this page is not.**

- It is not a production deployment. The `live` profile runs with
  `deployment.livemode: false` against `http://localhost:8080` — prod-shaped,
  sandbox-only. No real money moves, no handset is prompted, no cluster runs.
- It is not evidence of MTN **production** or of Orange Money; see
  [`../status.md`](../status.md) for what the 2026-09-15 run actually proved.
- It is not the design of the profile system
  ([`../flows/configuration.md`](../flows/configuration.md), ADR-0003).

---

## Preconditions

- **Docker**, for vpay's Postgres (`vpay-pg` on host port **5433**; the
  unrelated `webank-services` Postgres owns 5432 on the original machine, but
  any port you choose works — see the server command below).
- **Rust toolchain** (the repo pins it in `rust-toolchain.toml`) and **Node +
  pnpm** (the repo pins Node in `.nvmrc`).
- **Real MTN sandbox credentials**: a **Collections** subscription key, an
  `api_user` UUID, and its API key. (The Disbursements key is not used —
  refunds are not implemented.)
- A browser.

---

## Step 0 — prerequisites on a fresh clone

```bash
cd <repo root>

# Build the server binary (one time; the working tree's committed code).
cargo build -p vpay-server

# Vendors @vaam-apps/vpay-stripe-js into examples/checkout-browser/dist
# (the payer page's dependency). gitignored, rebuilt on every run.
just build-checkout-browser

# Fetch the published @vaam-apps/ui used by the dashboard and checkout apps.
pnpm install --filter @vpay/dashboard

# Start vpay's Postgres if it is not already up. Migrations run at boot.
docker run -d --name vpay-pg -p 5433:5432 \
  -e POSTGRES_USER=vpay -e POSTGRES_PASSWORD=vpay -e POSTGRES_DB=vpay \
  postgres:16-alpine
```

## Step 1 — keys and credentials

Each developer generates their own throwaway keys; none are committed.

### The keys

```bash
# vpay's OP signs /v1 access tokens with this. Any throwaway key works locally.
cargo xtask gen-signing-key --out secrets            # -> secrets/oauth-signing-key.pem

# The `live-merchant` signing keypair. Generate a FRESH one: the private half
# must be yours, and its public half must be the one the live profile trusts.
cargo xtask gen-signing-key --out .e2e/live-merchant # -> .e2e/live-merchant/oauth-signing-key.pem
```

`cargo xtask gen-signing-key --out <dir>` writes `<dir>/oauth-signing-key.pem`
(the private half, mode 0600) and prints the matching `kid` + public JWK.
`config/application-live.yml` references the **public** half only — vpay never
holds the private half (ADR-0010) — through two `${VAR}` placeholders. Put
**your** key's `kid` and modulus `n` (both printed by `gen-signing-key`) into
`.env.live` as `LIVE_MERCHANT_JWK_KID` and `LIVE_MERCHANT_JWK_N`, replacing the
example values. `.e2e/` and `secrets/` are git-ignored.

### The credentials

```bash
cp .env.live.example .env.live
# edit .env.live: MTN_SUBSCRIPTION_KEY, MTN_API_USER, MTN_API_KEY,
# and (for the dashboard) VPAY_STAFF_PEPPER + VPAY_STAFF_TOTP_KEY.
# Generate the two staff secrets with:
#   openssl rand -base64 32 | tr '+/' '-_' | tr -d '=\n'
```

`.env.live` is loaded by `set -a; . ./.env.live; set +a` in every terminal
below. It is **git-ignored** — never commit the filled copy. The names must
match the `${VAR}` placeholders in `config/application-live.yml`; an unresolved
placeholder is a fatal boot error (exit 78), never an empty string.

## Step 2 — Terminal 1: the API server

```bash
cd <repo root>
set -a; . ./.env.live; set +a

./target/debug/vpay-server \
  --profile live --config config/application.yml \
  --database-url postgres://vpay:vpay@localhost:5433/vpay \
  --oauth-signing-key-file ./secrets/oauth-signing-key.pem \
  --bind 127.0.0.1:8080 --log-format text
```

Wait for `healthz 200`:

```bash
curl -s -o /dev/null -w '%{http_code}\n' http://localhost:8080/healthz
```

The observability listener (default `0.0.0.0:9090`) comes up with it.

## Step 3 — Terminal 2: the worker

The worker claims the poll job, runs the authenticated `query_status` against
MTN, and settles the charge — **this** is what moves money, not the callback.

```bash
cd <repo root>
set -a; . ./.env.live; set +a

./target/debug/vpay-server worker \
  --profile live --config config/application.yml \
  --database-url postgres://vpay:vpay@localhost:5433/vpay \
  --log-format text --observability-bind 127.0.0.1:9091
```

`--observability-bind 127.0.0.1:9091` matters: without it the worker tries to
bind the same default `:9090` the server already holds and exits with
`Address already in use`.

## Step 4 — Terminal 3: the payer checkout page

```bash
cd <repo root>
node examples/checkout-browser/serve.mjs    # serves http://localhost:4180
```

This is a plain HTML + JS payer page (no framework). It reads a publishable
key and a `client_secret` from its own URL, confirms an MTN MoMo push, and
polls until the worker settles — see [`./checkout.md`](checkout.md).

## Step 5 — Terminal 4: mint a payment (the "merchant server")

```bash
cd <repo root>
set -a; . ./.env.live; set +a

VPAY_MERCHANT_CLIENT_ID=live-merchant \
VPAY_MERCHANT_PRIVATE_KEY_PATH=.e2e/live-merchant/oauth-signing-key.pem \
CHECKOUT_PUBLISHABLE_KEY=pk_test_livesandboxmerchant01 \
MINT_CURRENCY=eur \
node examples/checkout-browser/mint.mjs
```

This mints an **EUR** `mtn_momo` PaymentIntent as `live-merchant` and prints a
ready-to-open URL. The tracked example script reads `MINT_CURRENCY` (the demo
default is XAF, which MTN's sandbox rejects):

```
http://localhost:4180/?pk=...&client_secret=...&api=http%3A%2F%2Flocalhost%3A8080
```

## Step 6 — process the payment in the browser

1. Open the URL printed in Step 5.
2. Enter the MTN-sandbox test MSISDN **`46733123454`** and submit.
3. The page shows "waiting…"; the worker's status query settles the charge;
   the page renders **`succeeded`** (~30–60 s).

**"The sandbox approves in the background" means exactly this:** MTN's sandbox
auto-settles that test MSISDN, so you never get a handset prompt. The worker's
`GET /collection/v1_0/requesttopay/{reference}` is the call that observes the
settlement and moves the row to `succeeded`.

Confirm it from the worker (Terminal 2), which logs:

```
the rail reported a charge as paid  …  payment_intent_id=pi_…
```

## Step 7 — the staff dashboard (read view)

```bash
cd <repo root>
set -a; . ./.env.live; set +a

# create a staff account once; prints a one-time password on stdout
./target/debug/vpay-server staff add \
  --profile live --config config/application.yml \
  --database-url postgres://vpay:vpay@localhost:5433/vpay \
  --merchant live-merchant --email you@example.test --name "You" --admin

# run the dashboard dev server
export VPAY_DASH_API=http://localhost:8080 \
       VPAY_DASHBOARD_CLIENT_ID=vpay-dashboard \
       VPAY_DASHBOARD_REDIRECT_URI=http://localhost:8080/dash/v1/callback \
       VPAY_DASHBOARD_SCOPE=dashboard:read \
       VPAY_DASHBOARD_PUBLIC_ORIGIN=http://localhost:3000
just dev-dashboard        # http://localhost:3000
```

Sign in with the email + the one-time password, then complete TOTP enrolment.
The dashboard shows the `live-merchant` tenant's payment intents — the one you
just paid is there. The dashboard is read-only (ADR-0008); it never processes
a payment, so this step is optional for reproducing the settlement.

---

## Gotchas (each cost real time on 2026-09-15)

1. **EUR, not XAF.** MTN's sandbox rejects XAF. The `live` profile settles
   `mtn_momo` in EUR (`config/application-live.yml`), and the mint invocation
   sets `MINT_CURRENCY=eur` (amount 5000 = 50.00 EUR). A confirm trying XAF is
   refused by the `currencies_agree` guard.
2. **The token mint must send a JSON body.** `POST /collection/token/` answers
   `411 Length Required` to a bodyless POST and a 200 "Request Rejected" HTML
   page to the grant sent form-encoded. The adapter sends
   `{"grant_type":"client_credentials"}` with `Content-Type: application/json`
   — committed since 2026-09-15, and the conformance stub now enforces it. Do
   not "simplify" it back to the Orange form-encoded spelling.
3. **Use a real sandbox test MSISDN, not the WireMock "steer" numbers.** The
   hex numbers in [`../flows/adapter-mtn-momo.md`](../flows/adapter-mtn-momo.md)
   (e.g. `237600000ce0`) are stub scenario keys for `wiremock/wiremock`, not
   real payers. `46733123454` is a stock MTN-sandbox auto-settling test number.
4. **Callback host is validated by MTN, but settlement does not depend on it.**
   `callback_url: http://localhost:8080/provider/mtn_momo/callback` in the live
   config must have a host MTN recognises for your `api_user`, or the
   `requesttopay` call answers `INVALID_CALLBACK_URL_HOST`. Settlement still
   happens through the worker's `query_status` — the callback is only a hint.
5. **Ports.** vpay's Postgres is on **5433** (not 5432, which an unrelated
   service may own); the worker needs a distinct `--observability-bind`.

---

## Teardown

```bash
# stop the four processes (Ctrl-C in each terminal, or kill by PID)
just demo-down     # NOT needed unless you also started the demo compose stack
docker rm -f vpay-pg   # if you want to drop the database too
```

---

## What is and is not proven by a successful run

**Proven** (all observed on 2026-09-15): merchant `private_key_jwt`
authentication; creating an EUR `mtn_momo` intent; confirming it through the
browser path; the worker's `query_status` settling it to `succeeded` against
`https://sandbox.momodeveloper.mtn.com`; the dashboard's staff sign-in against
a real database.

**Not proven by this run, unchanged by it** ([`../status.md`](../status.md)):
no MTN **production** call, no Orange Money call, no real payer, no webhook to
a merchant endpoint outside this repository, no refunds, no cluster. The
`live` profile is `livemode: false`.

---

## Related

- [`../status.md`](../status.md) — the machine-checked contract; the banner's
  tenth addendum records the 2026-09-15 first real rail call.
- [`../flows/adapter-mtn-momo.md`](../flows/adapter-mtn-momo.md) — the rail's
  real-environment table and "Not proven" list.
- [`./demo.md`](demo.md) — the WireMock demo stack (a **different** config from
  the `live` profile).
- [`./rotate-rail-credentials.md`](rotate-rail-credentials.md) — rotating an
  MTN credential.
- [`../flows/dashboard-auth.md`](../flows/dashboard-auth.md) — the staff
  dashboard's OAuth flow (ADR-0017).
