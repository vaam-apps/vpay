# The demo — §7. Two demos on one machine

_Moved out of [docs/runbooks/demo.md](../demo.md) on 2026-09-11 by exp57, which split a 1 426-line runbook into the procedure and its steps. **The text below is the original, unedited** — every dated measurement, every struck-through claim and every correction is here as it was written; only the links changed: a `../` for the new depth, and, where a `§` cross-reference pointed at a section that is now on another page, the page it moved to._

## 7. Two demos on one machine

**What is isolated, measured on 2026-09-04** by bringing a second stack up
beside the first (`just demo_project=vpay-demo-b demo_port=18088
demo_receiver_port=18089 demo-up`) and running both walkthroughs concurrently:

```console
$ docker ps --filter name=vpay-demo --format '{{.Names}}\t{{.Ports}}' | sort
vpay-demo-b-postgres-1          5432/tcp
vpay-demo-b-vpay-server-1       0.0.0.0:18088->8080/tcp
vpay-demo-b-vpay-worker-1
vpay-demo-b-wiremock-mtn-1      8080/tcp, 8443/tcp
vpay-demo-b-wiremock-orange-1   8080/tcp, 8443/tcp
vpay-demo-b-wiremock-webhook-1  0.0.0.0:18089->8080/tcp
vpay-demo-postgres-1            5432/tcp
vpay-demo-vpay-server-1         0.0.0.0:18080->8080/tcp
vpay-demo-vpay-worker-1
vpay-demo-wiremock-mtn-1        8080/tcp, 8443/tcp
vpay-demo-wiremock-orange-1     8080/tcp, 8443/tcp
vpay-demo-wiremock-webhook-1    0.0.0.0:18083->8080/tcp

$ docker network ls --filter name=vpay-demo --format '{{.Name}}'
vpay-demo-b_default
vpay-demo_default

$ docker volume ls --filter name=vpay-demo --format '{{.Name}}'
vpay-demo-b_pgdata
vpay-demo_pgdata
```

Two projects, two networks, two volumes, two databases with different rows in
them (4 payment intents in one, 6 in the other), and **only the two intended
host ports published per stack** — Postgres and both rail stubs publish nothing
(`ports: !reset []` in `compose.demo.yml`), which is what makes a second stack
possible at all: 5432, 8081 and 8082 are fixed literals in `compose.yml` and
two stacks would collide on them however the three variables were set.

`vpay-demo-b`'s walkthrough ran all six outcomes green while `vpay-demo` was
up, so the stacks genuinely do not interfere at the Compose layer.

### Step 9 re-measured this, with five published ports per stack

Step 9 published three more of them — the Orange stub (so a payer can follow a
redirect), vpay's checkout page and the demo shop — so the paragraph above
stopped being the whole story. Re-run on 2026-09-04 on the authoring host:

```console
$ just demo_port=18080 demo                                     # stack A, from nothing
$ just demo_project=vpay-demo-b demo_port=18081        demo_receiver_port=18083 demo_orange_port=18082        demo_checkout_port=13080 demo_shop_port=13001 demo-up    # stack B, beside it

$ docker ps --filter name=vpay-demo --format '{{.Names}}	{{.Ports}}' | sort
vpay-demo-b-postgres-1           5432/tcp
vpay-demo-b-vpay-checkout-1      0.0.0.0:13080->3000/tcp
vpay-demo-b-vpay-server-1        0.0.0.0:18081->8080/tcp
vpay-demo-b-vpay-shop-1          0.0.0.0:13001->3000/tcp
vpay-demo-b-vpay-worker-1
vpay-demo-b-wiremock-mtn-1       8080/tcp, 8443/tcp
vpay-demo-b-wiremock-orange-1    0.0.0.0:18082->8080/tcp, 8443/tcp
vpay-demo-b-wiremock-webhook-1   0.0.0.0:18083->8080/tcp, 8443/tcp
vpay-demo-postgres-1             5432/tcp
vpay-demo-vpay-checkout-1        0.0.0.0:3080->3000/tcp
vpay-demo-vpay-server-1          0.0.0.0:18080->8080/tcp
vpay-demo-vpay-shop-1            0.0.0.0:3001->3000/tcp
vpay-demo-vpay-worker-1
vpay-demo-wiremock-mtn-1         8080/tcp, 8443/tcp
vpay-demo-wiremock-orange-1      0.0.0.0:8082->8080/tcp, 8443/tcp
vpay-demo-wiremock-webhook-1     0.0.0.0:8083->8080/tcp, 8443/tcp
```

Sixteen containers, ten published ports, no collision.

**Both `docker ps` blocks above were captured before lane r2 bound these
publications to `127.0.0.1` later the same day**, which is why they read
`0.0.0.0:`. They are kept verbatim rather than hand-edited — this page's rule
is that pasted output is pasted output — and the ports and the absence of
collisions are what they were captured to show. A `docker ps` today prints
`127.0.0.1:` for every one of them.

**Neither block shows a `dashboard` container, and both are older than the two
changes that would put one in them.** exp28 (2026-09-07) added `dashboard` to
`demo_services`, so `just demo` starts it; issue #78 (2026-09-10) made its host
port `demo_dashboard_port`, so a second stack can publish it somewhere else.
Before #78 it was the one service that could not be moved — `compose.e2e.yml`
published it on a literal `3000` and the OP had `http://localhost:3000/…`
registered as the dashboard client's `redirect_uri`, matched byte for byte —
so two stacks could not both serve a dashboard and the second `demo-up` died
on the port bind. A stack B started today wants `demo_dashboard_port=13000`
alongside its other overrides.

**The Orange stub used to be the thing that made this impossible**, and it is
worth knowing why because the fix is a file nobody looks at. The stub's
`payment_url` comes from a mapping that templates a literal
`http://localhost:8082`: WireMock renders a response from the current request
alone, and vpay's submit arrives over the compose network as
`wiremock-orange:8080`, so the stub cannot learn what the host published it
on. Step 9's lane 2 therefore made `gen-demo-keys` _check_ the pair and refuse
any `demo_orange_port` but 8082 — correct, and it meant two demos collided on
that port with no way out but editing a committed file.

`gen-demo-keys` now writes a per-project **copy** of those mappings with the
port substituted, under `.e2e/<demo_project>/wiremock-orange/`, and
`compose.demo.yml` mounts the copy instead of the committed tree (Compose
merges `volumes:` by target path). The committed mapping is untouched and
stays the CI/e2e default. Measured, in the containers rather than in the
recipe's output:

```console
$ docker exec vpay-demo-wiremock-orange-1       grep -o 'localhost:[0-9]*/stub-hosted-page' /home/wiremock/mappings/webpayment.json | sort -u
localhost:8082/stub-hosted-page
$ docker exec vpay-demo-b-wiremock-orange-1       grep -o 'localhost:[0-9]*/stub-hosted-page' /home/wiremock/mappings/webpayment.json | sort -u
localhost:18082/stub-hosted-page
```

and end to end, from stack B's own walkthrough while stack A was up — its
Orange redirect and its checkout URL are on **its** ports, and the page a
payer would click answers:

```console
         url          http://localhost:18082/stub-hosted-page/pay-c01c39a3-…?return=…&cancel=…
      HOSTED — open this in a browser:
        http://localhost:13080/c/cs_svk2eds261453bxd8xe00yv5?key=pk_test_demomerchantsandbox01#…
✔ all five steps behaved as expected — 6 payments on 2 rails, …

$ curl -sS -o /dev/null -w '%{http_code}\n' 'http://localhost:18082/stub-hosted-page/pay-c01c39a3-…'
200
```

**What is NOT isolated, and it is a real limitation rather than a caveat.**
The three variables isolate everything Compose owns. They do not isolate
`.e2e/`, which holds **one** merchant key pair and **one** profile overlay for
the whole checkout:

```console
$ just demo_project=vpay-demo-b demo_port=18088 demo_receiver_port=18089 demo-up
gen-demo-keys: .e2e/application-demo.yml was generated for a different demo_port than 18088 — regenerating the pair
gen-demo-keys: wrote .e2e/demo-merchant/oauth-signing-key.pem (3072-bit RSA, mode 0600, host-only)
gen-demo-keys: wrote .e2e/application-demo.yml — client_id=demo-merchant kid=e-xZOcqEipVJG5wrXY7DE6WHUn8S-lkfDuShnxmv1Ss
```

Because the two stacks want different `demo_port`s, bringing the second one up
**regenerates the shared merchant key pair**. The first stack's server still
holds the _old_ public JWK in memory, so its walkthrough then fails at step 2:

```console
✘ step 2 (access token): the token endpoint refused this merchant with HTTP 401: {"error":"invalid_client","error_description":"Client authentication failed"}
```

So: **two demos brought up in sequence coexist and both serve; the older one's
`demo-walk` stops working from the moment the newer one's `demo-up` runs.**
Bring the second stack up _before_ you start walking the first, or accept that
only the most recently generated key pair authenticates.

The fix is to key the `.e2e/` artefacts on `demo_project` the way the Compose
project is keyed. It was **not** done in Step 8: `.e2e/demo-merchant/oauth-signing-key.pem`
is a literal in `.github/workflows/ci.yml` (twice), in `just stripe-compat`, in
`examples/merchant-stripe-node/index.mjs`, in `sdks/stripe-compat`, and as the
default of `examples/merchant-demo`'s `VPAY_PRIVATE_KEY_FILE`, and a mistake
there fails _silently_ as `invalid_client`. See `docs/plans/step8-notes/lane-a.md`.

**Step 9 did not fix it either, and it now has a second key pair in it.**
`.e2e/` after a demo:

```console
$ ls -d .e2e/*/
.e2e/demo-merchant/   .e2e/shop-merchant/   .e2e/vpay-demo/   .e2e/vpay-demo-b/
```

Only the last two are keyed on the project — those are the generated Orange
mappings. The overlay and **both** merchant key pairs are still shared, so the
sentence above holds unchanged: two demos brought up in sequence coexist and
both serve; the older one's `demo-walk` stops working from the moment the
newer one's `demo-up` runs, and now the older one's **shop** stops
authenticating too, for the same reason and with the same `invalid_client`.
