# Testing the `notchpay` rail against Notch Pay's real sandbox

**Who this is for.** The maintainer, or whoever they ask (Valantine Suh ran
the 2026-09-15 MTN sandbox tests the same way), with a Notch Pay
**test-mode** account. You run two things on your own machine:

- **Part A**, RFC-0012 delivery step 1: a probe that asks Notch Pay's sandbox
  the § 10 sandbox questions and writes a redacted report.
- **Part B**, delivery step 5: vpay itself, with server, worker and Postgres,
  taking one sandbox payment that succeeds, one that fails and one that
  collects less than was asked, through the `notchpay` adapter.

You paste the outputs back to a Claude session. It records them on dated
pages under `docs/status/verification/` and corrects the adapter wherever
Notch Pay disagrees with its specification.

**What this page is not.** It is not evidence on its own: the evidence is on
the dated pages it points to. Part A ran on 2026-10-21 from a cloud session
([verification](../status/verification/2026-10-21-notchpay-sandbox-run.md)).
Part B ran on 2026-10-21 from the maintainer's machine
([verification](../status/verification/2026-10-21-notchpay-step5.md)), and
its B3b on 2026-10-22
([verification](../status/verification/2026-10-22-notchpay-b3b.md)).
_(This said "It is not evidence. Nothing has called Notch Pay until someone
runs it and the outputs are recorded" until 2026-10-21;
[the first attempt](../status/verification/2026-10-20-notchpay-sandbox.md) had
made no request.)_ It is not a production test: every key here is a test key,
and `deployment.livemode` stays `false`. It is not a test of vpay's webhook
signature check, which [RFC-0014](../rfc/0014-verifying-rail-callbacks.md)
built on 2026-10-23 and the `notchpay-sandbox` profile does not configure.
_(This said the check was "designed … and not built" until 2026-10-23.)_

Written 2026-10-20. Before either part had been run against Notch Pay, what
was run, in a container that cannot reach Notch Pay, was this:

- the probe's three phases, against the repository's WireMock stub
  (`backends/tests/conformance/wiremock/notchpay`) on loopback, with fixture
  keys;
- its key refusal (no request, no report);
- `verify-signature` against HMACs computed independently with `openssl`;
- B1 and B2 below, booted against a local Postgres with fixture keys, up to
  `healthz` and `livez`. Neither process calls the rail until a charge
  exists, and B3 was not run.

_(This page said "Nothing on this page has been run yet against Notch Pay"
and "The first real run will find something this page gets wrong; fix the
page in the same change that records the run" until 2026-10-21.)_ **Part B's
run on 2026-10-21 found four things this page got wrong**, corrected in the
same change that records it: B1 left the server's observability listener on
`0.0.0.0:9090`, and `/livez` is on that port, not on 8080; B3's loop never set
`VPAY_AMOUNT`, so every payment carried the example's default 5 000 XAF,
against ground rule 4; B3's timing text omitted the worker's immediate first
read and said "about two minutes" where both outcomes arrived on the second
read, about 15 s after create; and B3 expected `…000001` to reach
`requires_payment_method`, which it does not (see B3). B3's query also gained
the `provider_txn_id` and `notchpay_reference` columns.

---

## Ground rules

1. **Test keys only.** In Notch Pay's dashboard, check that the account is in
   **Test** mode before you copy the keys. The probe and vpay's boot both
   refuse a key that contains neither `test.` nor `test_`. That check is a
   backstop. It does not replace looking at the dashboard.
2. **Keys never leave your machine.** Keep them in an env file **outside the
   repository**. Never paste a key, the hash key or an endpoint's signing
   secret into a chat, an issue or a commit. The probe's report is redacted,
   but read it before you paste it.
3. **Real egress.** vpay trusts only its vendored root certificates and
   ignores proxy variables. Behind a TLS-intercepting proxy, every call fails
   at the handshake; that is what stopped the 2026-10-20 attempt. Check first:

   ```bash
   echo | openssl s_client -connect api.notchpay.co:443 -servername api.notchpay.co 2>/dev/null \
     | openssl x509 -noout -issuer
   ```

   The issuer must be a public CA, not your company's or a gateway's.
   The one exception is a Claude Code cloud session, below.

4. **Vary the numbers.** Notch Pay's fraud system is active in the sandbox
   and watches for many payments to one number. The probe varies amounts.
   Do not loop it.

## The env file

```bash
mkdir -p ~/.config/vpay && touch ~/.config/vpay/notchpay-sandbox.env
chmod 600 ~/.config/vpay/notchpay-sandbox.env
$EDITOR ~/.config/vpay/notchpay-sandbox.env
```

```text
NOTCHPAY_TEST_PUBLIC_KEY=...      # dashboard: Settings > API keys, test mode
NOTCHPAY_TEST_PRIVATE_KEY=...     # sent as X-Grant on every call
NOTCHPAY_TEST_HASH_KEY=...        # Part A item 7 only; this profile gives vpay no signing secret
NOTCHPAY_SANDBOX_MERCHANT_JWK_KID=...   # Part B, from gen-signing-key below
NOTCHPAY_SANDBOX_MERCHANT_JWK_N=...     # Part B, from gen-signing-key below
```

Load it in every terminal with `set -a; . ~/.config/vpay/notchpay-sandbox.env; set +a`.

---

## Part A — the sandbox probe (step 1)

`backends/crates/vpay-notchpay/examples/sandbox_probe.rs` is a cargo
example. Its own unit tests run in `just ci`; the probe itself never does.
It builds every request body from `vpay-notchpay`'s own
wire types and decodes every answer with them. Each read is also made
through `vpay_notchpay::Client`. It writes one JSON report, appending a run
per phase. Run it from the repository root:

```bash
cargo build -p vpay-notchpay --example sandbox_probe
PROBE=./target/debug/examples/sandbox_probe
REPORT=~/notchpay-sandbox-report.json      # outside the repository
```

### Cloud sessions only: `--ca-bundle`

A Claude Code cloud session's egress gateway re-terminates TLS with its own
CA, so the probe's vendored roots refuse `api.notchpay.co` there (ground
rule 3). In such a session, and only there, add
`--ca-bundle /root/.ccr/ca-bundle.crt` to every phase that calls Notch Pay:
`read`, `payments`, `webhooks`, `search`, `refunds`, `two-step`, `channels`,
`webhook-receive` and `webhook-update`. The probe then trusts that file's PEM certificates **in
addition to** the vendored roots, for its own client only. Redirects stay
refused and proxy variables stay ignored, exactly as in
`vpay_provider::http`. A missing, unreadable or certificate-less file stops
the probe before any request. The flag is off unless given.

This is a cloud-only aid for step 1. It is not in `vpay-server`,
`vpay-worker`, `vpay_provider::http` or any shipping configuration, and must
not be added to them: ADR-0003 forbids an environment branch in shipping
code. Step 5 therefore cannot run from such a session; it stays on a machine
whose egress presents Notch Pay's own certificate.

### A1. Read-only first

```bash
$PROBE read --out "$REPORT"
```

This changes nothing on the account. It reads a payment reference that cannot
exist and lists the account's webhook endpoints. Its last line says whether
any field names a mode. **If anything says live, stop.**

### A2. Payments (items 2, 3, 4, 5)

```bash
$PROBE payments --out "$REPORT"
```

It initializes and processes six payments on `cm.mobile`, the channel the
adapter sends a charge confirmed by the rail code (since 2026-10-23 a charge
routed by method type is sent `cm.mtn` or `cm.orange`), with Notch Pay's
documented outcome numbers. Notch Pay's
[testing page](https://developer.notchpay.co/get-started/testing) lists them
under `cm.mtn` and `cm.orange`, so whether they work on `cm.mobile` is item 3.

| Number          | Documented outcome          |
| --------------- | --------------------------- |
| `+237670000000` | MTN: success                |
| `+237670000001` | MTN: insufficient funds     |
| `+237670000002` | MTN: failure (other reason) |
| `+237670000003` | MTN: timeout (no response)  |
| `+237670000004` | MTN: canceled by user       |
| `+237690000000` | Orange: success             |

For each one it processes by vpay's own reference first, and by Notch Pay's
`trx.…` reference only if that is refused (item 2). It then reads every 5 s
until a terminal status or 180 s (`NOTCHPAY_PROBE_POLL_SECONDS` changes it),
recording each status change (item 5). Finally it re-initializes the first
reference (item 4). It takes about ten minutes, mostly the timeout number.

### A3. Webhook endpoints (items 13, 14, 15, 16, 17's event name)

```bash
$PROBE webhooks --out "$REPORT" --webhook-url https://vpay-sandbox-probe.example/provider/notchpay/callback
```

It lists the account's endpoints, then creates three at that URL:

- one with the five payment events;
- the same again, to see whether a duplicate URL is refused;
- one asking for `payment.processing`.

Then it lists again and **deletes every endpoint it created**. Any it could
not delete are named on its last line; delete those in the dashboard. A
signing secret in an answer is recorded as its shape only (`hsk_test_…`).
The URL need not be reachable for these items. Use one that is obviously
vpay's probe.

### A4. Receiving a webhook (items 7 and 17)

These need Notch Pay to POST to you, so they need a **public URL**. Which
tunnel to use is your call. A third-party tunnel or capture service sees your
test account's payment data, which is why a Claude session will not choose
one for you. If you have none, skip A4. Items 7 and 17 then stay open, by
name.

With a tunnel forwarding to `localhost:8099`, run a receiver that keeps the
raw body and the header **names** (`receive.py` below writes outside the
repository):

```python
# ~/receive.py — saves each POST's raw body and header names; prints nothing secret
import http.server, json, pathlib, time
out = pathlib.Path.home() / "notchpay-webhooks"; out.mkdir(exist_ok=True)
class H(http.server.BaseHTTPRequestHandler):
    def do_POST(self):
        body = self.rfile.read(int(self.headers.get("Content-Length") or 0))
        stem = out / str(time.time_ns())
        stem.with_suffix(".body").write_bytes(body)
        sig = next((v for k, v in self.headers.items() if k.lower() == "x-notch-signature"), "")
        stem.with_suffix(".sig").write_text(sig)
        stem.with_suffix(".headers.json").write_text(json.dumps(list(self.headers.keys())))
        self.send_response(200); self.end_headers()
http.server.HTTPServer(("127.0.0.1", 8099), H).serve_forever()
```

1. In the dashboard (Settings > Webhooks), add an endpoint at your tunnel's
   URL for every payment event, `payment.processing` included if offered.
2. Run `python3 ~/receive.py`, then repeat A2 (`$PROBE payments --out "$REPORT"`).
3. For one delivery, check what the signature covers:

   ```bash
   f=$(ls ~/notchpay-webhooks/*.body | head -1); s="${f%.body}.sig"
   $PROBE verify-signature --body "$f" --signature "$(cat "$s")"
   ```

   It prints whether HMAC-SHA256 under the hash key matches the raw body or
   its `JSON.stringify` form. It never prints the key.

4. Delete the endpoint in the dashboard.

What to paste back from A4:

- the `verify-signature` output;
- the event `type` of each delivery (`grep -ho '"type":"[^"]*"' ~/notchpay-webhooks/*.body | sort | uniq -c`);
- the contents of one `.headers.json` file, which holds header names only.

Do not paste the bodies or the signatures.

### A5. Search by reference (item 19)

```bash
$PROBE search --out "$REPORT" --reference <our reference> --reference <trx.… reference>
```

Read-only. For each value it calls `GET /payments?search=` and then reads
the same value directly. Take the references from `payments`' report. On
2026-10-21 `search` found Notch Pay's `trx.…` and not ours
([verification](../status/verification/2026-10-21-notchpay-sandbox-run.md)).

### A6. Refunds (items 6 and 12)

```bash
$PROBE refunds --out "$REPORT"
```

This one **moves sandbox money**. It initializes and processes one 200 XAF
payment on `+237670000000` and reads it until `complete` (at most 120 s).
Then it sends `POST /refunds` for 50 XAF four times:

- the guide's body (`payment`, `amount`, `reason`, `metadata.vpay_refund`);
- the identical body again, which is the replay item 6 asks about;
- the same body with a `reference` of ours, which the guide does not
  document;
- that `reference` again.

After 15 s it reads `GET /payments/{trx}/refunds`, each refund id that came
back at `GET /refunds/{id}`, and the payment. Its last notes say how many
distinct refund ids the four answers held. If the payment ends with more
than 50 XAF refunded, or with more than one refund for an identical body,
**a replay pays twice**. Write that down before anything else.

On 2026-10-22 every `POST /refunds` answered `403` "Forbidden", with no
message, and nothing was refunded
([verification](../status/verification/2026-10-22-notchpay-refunds.md)).
Rerun it once the dashboard or Notch Pay says refunds are allowed for the
account.

### A7. The two-step submit, the per-network channels, and a received webhook (2026-10-23)

ADR-0032 (initialize and process as two port calls, with a commit between
them) and RFC-0012 step 8 (`cm.mtn` and `cm.orange` from the payer's method
type) were built on 2026-10-23 against a stub. These phases measure Notch
Pay's side of both. vpay itself still cannot run from a cloud session (see
"Cloud sessions only"), so they are probe phases, not Part B.

```bash
# Initialize two payments and process neither. Re-send one initialize (the
# 409), read the never-processed one at 0, 60, 300, 900, 1800 and 3600 s,
# process the other at 300 s, and process the first at the end. About an hour.
NOTCHPAY_PROBE_EXPIRY_CHECKS=0,60,300,900,1800,3600 \
NOTCHPAY_PROBE_LATE_PROCESS_SECONDS=300 \
$PROBE two-step --out "$REPORT.two-step"

# Every documented MTN and Orange outcome number on cm.mtn and cm.orange,
# then each network's success number on the other network's channel.
$PROBE channels --out "$REPORT.channels"

# One endpoint at a URL you can read, one success, one failure, a wait, then
# DELETE. Then, if the endpoint stored no URL, the documented PUT by id.
NOTCHPAY_PROBE_DELIVERY_WAIT=900 $PROBE webhook-receive --out "$REPORT.webhook" --webhook-url "$INBOX_URL"
$PROBE webhook-update --out "$REPORT.webhook-update" --reference ep_test.… --webhook-url "$INBOX_URL"
```

Give each phase its own `--out` when two run at once: the report is read,
extended and written back when a phase ends, so two concurrent phases on one
file keep only the last.

The 2026-10-23 run used a webhook.site inbox as `$INBOX_URL`, on the
maintainer's go-ahead in that session's chat ("A third-party capture
service"). Create it answering `500`, so retries would show: `POST
https://webhook.site/token` with `{"default_status":500}`. Read it with `GET
https://webhook.site/token/<uuid>/requests`, and delete it with `DELETE
https://webhook.site/token/<uuid>` when you are done. The token reads the
inbox, so keep it out of the repository and out of the report. If a request
arrives, save its raw body to a file outside the repository and run
`$PROBE verify-signature --body <file> --signature <x-notch-signature value>`.

**Two things that run did not expect, and what to do about them:**

- **`POST /webhooks` stored `"url": null`** and nothing reached the inbox in
  21 minutes. `GET`, `PUT` and `DELETE /webhooks/{id}` all answered `404`,
  so the endpoint could not be read, repaired or deleted through the API.
  **Delete it in the dashboard**, by the id the probe printed.
- **Every payment processed after that endpoint existed stayed
  `processing`**, while every one before it reached a final status within
  seconds. Delete the endpoint before you run `payments`, `channels` or
  `refunds`, and run `read` first to confirm the list is empty.

### What to paste back from Part A

The whole of `$REPORT`, after reading it. It holds request and response
bodies with the sandbox test numbers, Notch Pay's ids and references, and no
key. Before you paste, check:

```bash
for v in NOTCHPAY_TEST_PUBLIC_KEY NOTCHPAY_TEST_PRIVATE_KEY NOTCHPAY_TEST_HASH_KEY; do
  grep -qF -- "$(printenv $v)" "$REPORT" && echo "$v FOUND - do not paste" || echo "$v absent"
done
```

---

## Part B — the first test-mode payment through vpay (step 5)

`config/application-notchpay-sandbox.yml` is the profile. It has
`livemode: false`, one rail (`notchpay` at `https://api.notchpay.co`, channel
`cm.mobile`, XAF) and one merchant, `notchpay-sandbox-merchant`. Its keys are
`${VAR}`s from the env file above.

### B0. Build, key, database

```bash
cargo build -p vpay-server
cargo xtask gen-signing-key --out secrets                                # the OP's signing key
cargo xtask gen-signing-key --out .e2e/notchpay-sandbox-merchant         # the merchant's keypair
# put the printed kid and n into the env file as NOTCHPAY_SANDBOX_MERCHANT_JWK_KID / _N
pnpm install --filter "@vpay-examples/merchant-node..." && pnpm --filter @vaam-apps/vpay-sdk build
docker run -d --name vpay-pg-notchpay -p 127.0.0.1:5434:5432 \
  -e POSTGRES_USER=vpay -e POSTGRES_PASSWORD=vpay -e POSTGRES_DB=vpay postgres:16-alpine
```

`secrets/` and `.e2e/` are git-ignored. `127.0.0.1:` keeps Postgres, whose
password is `vpay`, on loopback. Without it Docker publishes the port on
every interface. _(This said `-p 5434:5432` until 2026-10-22; B3b's run
bound loopback instead
([verification](../status/verification/2026-10-22-notchpay-b3b.md)).)_ A
merchant keypair from an earlier run can be reused instead of generated, if
its public half is the env file's `_KID` and `_N`: B3b's run did that.

### B1. Terminal 1: the server

```bash
set -a; . ~/.config/vpay/notchpay-sandbox.env; set +a
./target/debug/vpay-server \
  --profile notchpay-sandbox --config config/application.yml \
  --database-url postgres://vpay:vpay@localhost:5434/vpay \
  --oauth-signing-key-file ./secrets/oauth-signing-key.pem \
  --bind 127.0.0.1:8080 --observability-bind 127.0.0.1:9090 \
  --log-format text 2>&1 | tee ~/notchpay-server.log
```

`--observability-bind` keeps `/livez` and `/metrics` on loopback. Without it
the server's observability listener bound `0.0.0.0:9090`, every interface, on
the maintainer's laptop on 2026-10-21. `/livez` is on that port and not on
8080, where only `/healthz` answers (`/livez` there is a `404`).

A **WARN about the public key's exposure is expected** at every boot
(ADR-0031 condition 4). A boot error naming `credentials.public_key` and
"lacks" means the key has neither `test.` nor `test_`. Stop and check which
key you copied.

### B2. Terminal 2: the worker

```bash
set -a; . ~/.config/vpay/notchpay-sandbox.env; set +a
./target/debug/vpay-server worker \
  --profile notchpay-sandbox --config config/application.yml \
  --database-url postgres://vpay:vpay@localhost:5434/vpay \
  --log-format text --observability-bind 127.0.0.1:9091 2>&1 | tee ~/notchpay-worker.log
```

The worker's authenticated read is what moves the charge. Notch Pay cannot
reach `localhost`, and no webhook is needed.

**Since 2026-10-22 the worker also lists the account's webhook endpoints**,
at start and hourly: RFC-0013's `ensure_callbacks` job. The profile sets
`callbacks.register: check`, so that is one `GET /webhooks` and never a
create; the worker logs `no callback endpoint is registered at vpay's
callback URL` at `warn`, which is expected here. Creating one is B4.

### B3. Terminal 3: one success, one failure, one short collection

```bash
for pair in 237670000000:210 237670000002:220 237670000001:230; do
  n=${pair%%:*}; amt=${pair##*:}
  VPAY_BASE_URL=http://localhost:8080 VPAY_CLIENT_ID=notchpay-sandbox-merchant \
  VPAY_PRIVATE_KEY_FILE=.e2e/notchpay-sandbox-merchant/oauth-signing-key.pem \
  VPAY_RAIL=notchpay VPAY_PAYER_MSISDN=$n VPAY_AMOUNT=$amt VPAY_ORDER="np-$n-$(date +%s)" \
    node examples/merchant-node/index.mjs
done
```

Set `VPAY_AMOUNT`: the example's default is 5 000 XAF for every payment, and
ground rule 4 says to vary the amounts. Each payment prints `created pi_…`
and `confirmed processing`. The worker reads the charge once right after
submit, then again after waits of 10, 20, 30, 45, 60 and 90 s, and every
120 s after that (the waits are between reads, not from submit). On
2026-10-21 both outcomes arrived on the second read, about 15 s after create.
Wait a few minutes, then read the state from the database:

```bash
psql postgres://vpay:vpay@localhost:5434/vpay -c \
 "select pi.id, pi.status, c.state, c.failure_code, c.provider_txn_id is not null as has_txn,
         c.provider_txn_id, c.provider_ref_extra->>'notchpay_reference' as notchpay_reference, c.updated_at
    from payment_intents pi join charges c on c.payment_intent_id = pi.id order by c.created_at"
psql postgres://vpay:vpay@localhost:5434/vpay -c \
 "select object_id, type, created_at from events order by seq"
```

Expected if the mapping is right:

- `…000000` reaches `succeeded`.
- `…000002` reaches `requires_payment_method`, charge `failed`,
  `failure_code` `provider_error` (Notch Pay's read is `failed` and names no
  reason).
- `…000001`, the "insufficient funds" number, reaches neither. Notch Pay's
  sandbox reads it `complete` with `amounts.total` short of `amount` (step 1
  and step 5, 2026-10-21). Since 2026-10-23 the adapter refuses that as
  `NeedsReconciliation` (vaam-apps/vpay#313), and the worker escalates the
  charge to `unresolved` on the first read that says so: one WARN naming the
  cause ("…needs an operator; escalating it to unresolved now"), then an ERROR
  with `alert = true`, then the hourly poll. The intent stays `processing`.
  **Any `succeeded` here is a defect, and so is a charge still `pending`
  after its second read.** _(Until 2026-10-23 this said the adapter refused it
  as `Malformed`, the worker retried it on the ladder with a WARN, and the
  charge became `unresolved` only at the 24 h horizon.)_

Whatever you see is the answer. Do not adjust anything to match the
expectation.

If `…000000` or `…000002` never leaves `processing`, the read may not be
reaching a terminal status. Leave it running for 15 minutes and paste what
you have.

_(B3 said `…000001` "reaches `requires_payment_method` with a `failure_code`"
until 2026-10-21, and ran only two payments, `…000000` and `…000001`.)_

### B3b. Timeout, canceled, and Orange

Step 5 ran only `…000000`, `…000001` and `…000002` through vpay. The other
three outcome numbers have been answered by the probe (step 1, 2026-10-21)
and never through vpay. Same stack, same terminal:

```bash
for pair in 237670000003:240 237670000004:250 237690000000:260; do
  n=${pair%%:*}; amt=${pair##*:}
  VPAY_BASE_URL=http://localhost:8080 VPAY_CLIENT_ID=notchpay-sandbox-merchant \
  VPAY_PRIVATE_KEY_FILE=.e2e/notchpay-sandbox-merchant/oauth-signing-key.pem \
  VPAY_RAIL=notchpay VPAY_PAYER_MSISDN=$n VPAY_AMOUNT=$amt VPAY_ORDER="np-$n-$(date +%s)" \
    node examples/merchant-node/index.mjs
done
```

Then the same two `psql` queries as B3. Expected, from step 1's reads and
the adapter's mapping:

- `…000003` (MTN timeout): Notch Pay reads `expired`, so the charge is
  `failed`, with `failure_code` `payer_timeout` and the intent
  `requires_payment_method`.
- `…000004` (MTN canceled by user): Notch Pay reads `canceled`, so the
  charge is `failed`, with `failure_code` `payer_declined` and the intent
  `requires_payment_method`.
- `+237690000000` (Orange success): Notch Pay reads `complete` with
  `amounts.total` equal to `amount`, so the intent is `succeeded`, with one
  balanced ledger transaction.

Step 1 saw `expired` and `canceled` by its final read, about 180 s after
process. It did not time how much sooner they came. So give these numbers
up to 15 minutes before calling one stuck. ~~A2's table, taken from Notch
Pay's testing page, has no Orange failure number, so B3b cannot test
Orange's failure path.~~ **Corrected 2026-10-23:** the testing page gives the
five suffixes `…000000` to `…000004` "for each operator", Orange's
`+23769…` included. A2's table took Orange's success number only. The
probe's `channels` phase (A7) ran Orange's other four on `cm.orange` on
2026-10-23, and they read as MTN's do. vpay itself has not run them, so a
B3c with `+237690000002`, `…000003` and `…000004` would be the first.

On 2026-10-22 B3b ran from the maintainer's machine, and every expectation
above held. `…000003` failed `payer_timeout`, `…000004` failed
`payer_declined`, and `+237690000000` succeeded with a 260 XAF ledger
capture. Each was terminal on the worker's second read, 11 to 12 s after
create, not minutes. vpay sent 3 submits and 6 reads
([verification](../status/verification/2026-10-22-notchpay-b3b.md)).

### B4. Registering vpay's webhook endpoint (RFC-0013 step 5)

Not run yet. It is RFC-0013's delivery step 5: the first time vpay creates a
webhook endpoint on a real Notch Pay account. It makes one `GET /webhooks`
and **one `POST /webhooks`**, which the account keeps until someone deletes
it in the dashboard: Notch Pay's `DELETE` answered `404` on 2026-10-21.

**Since 2026-10-23 no deployment does this unless told to**: `ensure` is no
longer Notch Pay's default, which is `off` (RFC-0013's amendment). The
probe's endpoint, created through the same `POST /webhooks` that day, was
stored with `"url": null` and delivered nothing in 21 minutes, and four
payments processed after it stayed `processing`
([verification](../status/verification/2026-10-23-notchpay-sandbox-two-step-and-channels.md)).
Run B4 only to measure that again, on the maintainer's approval.

1. Point the callback URL at an `https://` address. The profile's default is
   `http://localhost:8080/...`, which Notch Pay has never been shown; the
   2026-10-21 run created endpoints at an `https://….invalid` URL, which it
   accepted. A4's tunnel URL works too, and then A4's receiver sees the
   events. In the profile, under the `notchpay` rail:

   ```yaml
   callback_url: https://vpay-sandbox-b4.invalid/provider/notchpay/callback
   callbacks:
     register: ensure
   ```

2. Check, then register, then check again:

   ```bash
   set -a; . ~/.config/vpay/notchpay-sandbox.env; set +a
   V="./target/debug/vpay-server --profile notchpay-sandbox --config config/application.yml \
      --database-url postgres://vpay:vpay@localhost:5434/vpay --log-format text"
   $V callbacks ensure --check   # expect: notchpay ensure missing -   (and exit 1)
   $V callbacks ensure           # expect: notchpay ensure created ep_test.…
   $V callbacks ensure           # expect: notchpay ensure present ep_test.…   (no second create)
   ```

3. In the dashboard (Settings > Webhooks), one endpoint should carry the
   description `vpay:vpay-notchpay-sandbox:notchpay:default url:` followed
   by 16 hex digits, and the five `payment.*` events. Note whether the
   dashboard shows its URL: the API answers `"url": null`.

What to paste back from B4: the three output lines, the dashboard's
description and URL for that endpoint, and
`grep -E "callback endpoint|ensure_callbacks" ~/notchpay-worker.log` if the
worker was running. Put the profile back (`register: check`, no
`callback_url`) afterwards, and delete the endpoint in the dashboard.

### What to paste back from Part B

- both `psql` outputs;
- `grep -E "notchpay|charge|poll" ~/notchpay-worker.log | tail -100`;
- the server's boot lines up to `healthz`.

Check them first. vpay never logs a key, and this verifies it:

```bash
for v in NOTCHPAY_TEST_PUBLIC_KEY NOTCHPAY_TEST_PRIVATE_KEY; do
  grep -qF -- "$(printenv $v)" ~/notchpay-*.log && echo "$v FOUND - do not paste" || echo "$v absent"
done
```

## Teardown

Ctrl-C both terminals, then `docker rm -f vpay-pg-notchpay`. Delete any
webhook endpoint left in the dashboard, B4's included. Delete `~/notchpay-webhooks/` once
the results are recorded.

## What a run proves, and what it does not

**Proves**, once recorded on a dated page: what Notch Pay's sandbox answers
to the calls vpay's client and adapter make, and whether `notchpay` charges
reach a terminal status through the worker's read. Only then may
[`../status/providers.md`](../status/providers.md)'s Collect and Status read
cells say "proven; sandbox" with the date.

**Does not prove**: anything about Notch Pay's production; a live key's
prefix (§ 10 item 8, a written question); refunds (`notchpay::refund` stays
a token); vpay's webhook signature check (RFC-0014, built 2026-10-23, and
not configured by this profile, so it answers `NoSecret` here).

## Related

- [RFC-0012](../rfc/0012-notchpay-adapter.md) § 10 — the questions, and § "Delivery".
- [`../flows/adapter-notchpay.md`](../flows/adapter-notchpay.md) — the adapter and its "To confirm" list.
- [`./live-sandbox-test.md`](live-sandbox-test.md) — the MTN equivalent this page is modelled on.
