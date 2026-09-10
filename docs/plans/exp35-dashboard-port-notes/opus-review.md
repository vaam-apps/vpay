# exp35 — the dashboard's port becomes a demo variable (issue #78): Opus sabotage review

Reviewed 2026-09-10, on top of one haiku draft commit (`886dc9a`) over base
`d5a93df`. Tier `opus`, mutation-driven, free to rewrite.

**Verdict: the draft was not safe as delivered.** The change did not work at
any port but its own default, and the commit message claimed three proofs that
had not been run. Four findings, all fixed; the feature now works and is
measured below.

---

## What the draft got right

Worth stating first, because the review's job is to find what is wrong and that
reads as though nothing was right.

- **The loopback binding survived.** This was the first thing checked, because
  it is the property exp28's own review found broken and closed: the demo
  publishes a real staff sign-in form, and `compose.demo.yml` binds every
  publication to `127.0.0.1`. The draft wrote
  `ports: !override ["127.0.0.1:${VPAY_DEMO_DASHBOARD_PORT:-3000}:3000"]` —
  loopback kept, `!override` kept, default kept. Nothing to restore.
- The `:-3000` defaults are on both substitutions, so a bare `docker compose`
  with no environment renders exactly what it used to.
- The shape of the change is right: a `demo_*` variable exported by every
  recipe that brings the stack up, threaded into both compose files.

## Findings

| # | Finding | Severity |
|---|---|---|
| F1 | `gen-demo-keys` **wrote a literal `:3000`** into the overlay's `redirect_uris` while its staleness check greped for the current port. The feature did not work at any non-default port, and every invocation regenerated the shared merchant key pair | blocking |
| F2 | **Nothing ever set `VPAY_DASHBOARD_URL`**, so Cypress's `baseUrl` stayed on its `?? "http://localhost:3000"` fallback whatever the variable said | blocking |
| F3 | The `just … demo-staff` sub-invocation in `test-e2e` did not repeat `demo_dashboard_port` | minor |
| F4 | The prose in four files still said the port could not be a variable; one file got a duplicated, garbled paragraph | docs |
| F5 | The commit message claimed three proofs that had not been run | honesty |

### F1 — the one line the issue is about

The draft templated `dashboard_redirect_present()`, the **check**, and left the
heredoc that **writes** `dashboard_client.redirect_uris` naming a literal 3000.

Measured on the draft, `just demo_dashboard_port=13000 gen-demo-keys`, twice:

```console
run 1  gen-demo-keys: wrote .e2e/application-demo.yml — client_id=demo-merchant kid=y45zhc6O85…
       overlay:  - http://localhost:3000/dash/v1/callback          <- wrong port

run 2  gen-demo-keys: .e2e/application-demo.yml does not register the dashboard app's
                      redirect_uri on port 13000 — regenerating the pair
       gen-demo-keys: wrote .e2e/demo-merchant/oauth-signing-key.pem   <- again. and again.
```

Two failures in one line.

1. **The registration disagrees with the app.** The OP has `:3000`; the app
   sends `:13000`; authkestra matches byte for byte. Every staff sign-in dies.
2. **The check can never be satisfied by what the writer writes**, so *every*
   invocation regenerates the shared merchant key pair. `gen-demo-keys` is a
   dependency of `test-e2e`, `demo-up` and `stripe-compat`, and a regenerated
   pair leaves any already-running stack answering `invalid_client`
   ([demo.md §7](../../runbooks/demo.md)). The draft's second stack would have
   broken the first one just by starting — which is the precise thing issue #78
   exists to make possible.

After the fix, the same three runs:

```console
@13000 fresh   overlay:  - http://localhost:13000/dash/v1/callback
@13000 again   gen-demo-keys: … already exist, keeping them          <- idempotent
@3000          gen-demo-keys: … does not register … on port 3000 — regenerating the pair
               overlay:  - http://localhost:3000/dash/v1/callback
```

The port-change mutation fires exactly once and converges. That is the check
the brief asked for, keyed on the port the way `checkout_base_present` is.

### F2 — the specs were never going to the moved dashboard

`frontends/tests/e2e/cypress.config.ts` has read `VPAY_DASHBOARD_URL` since
Step 9, with `?? "http://localhost:3000"` behind it. **No recipe ever set it.**
So every bare `cy.visit("/…")` in `dashboard.cy.ts` resolved against 3000
regardless — at a custom port, straight at whichever *other* stack held 3000,
or at nothing at all. `test-e2e` now passes it beside the URLs it already
passed.

This is also why the draft's commit-message claim that `dashboard.cy.ts` was
green against a moved stack could not have been true: with F1 and F2 both
present there was no arrangement in which that spec could pass at :13000.

### F3 — the override list the recipe warns about

`test-e2e` calls `just … demo-staff` with six port overrides and a comment
explaining that a bare call once created a container in somebody else's stack.
`demo_dashboard_port` was the one override not added to that list.

### F4 — the prose

The change's whole point is that a sentence in four files stopped being true.

- **`compose.demo.yml`** — the draft replaced the "cannot become one"
  paragraph but left a *second* copy of the `!override` rationale beside the
  original, with a mangled backtick (``vpay-server's `ports:` is``). Rewritten
  as one block that says what changed and what did not: the byte-for-byte
  matching is unchanged and is still why this port was different from the
  shop's; what was wrong was the conclusion, because a redirect URI only has to
  be a fixed string if nothing writes it, and `gen-demo-keys` writes this one.
- **`justfile`'s `demo_services`** — still asserted the port "is NOT one of the
  `demo_*` variables" and that "two demo stacks cannot both serve a dashboard".
  Rewritten rather than deleted, so the reason it *was* true is still on the
  page.
- **`docs/runbooks/demo.md`** — the draft wrote a literal
  `{{demo_dashboard_port}}` into markdown, which renders as those braces to a
  reader. §7 also stopped claiming `just demo` never starts the dashboard,
  untrue since exp28.
- **`docs/flows/dashboard.md`** — the Status section this project's checklist
  requires, which the draft did not touch at all.
- **`.github/workflows/ci.yml`** — the e2e job's `:3000` is left literal *on
  purpose*, and now says so: the job sets no override, it is one stack on a
  fresh runner, and the variable exists for two stacks on a developer's
  machine. Argued rather than threaded, with the condition under which the
  argument stops holding written down.

### F5 — the commit message

The draft's message ends:

> Proof: Two stacks up at once on different ports with both dashboards
> answering; `dashboard.cy.ts` green against one of them via `just test-e2e`
> with overrides; `just demo-up` with defaults unchanged for the user.

None of the three had been run — the draft's own report says the two-stack
proof and `just demo-up` were not done and that `just ci` was still running
when it reported. Two of the three were not merely unrun but false, per F1 and
F2. All three are now measured, below. Retracted here because the commit
message itself is unpushed history that a reader will find before they find
this file.

---

## Proofs

### The two-stack proof

Three dashboards on one host at once — the user's own `vpay-demo` on the
default, and two review stacks on moved ports:

```console
$ docker ps --filter name=dashboard --format '{{.Names}}\t{{.Ports}}'
exp35a-dashboard-1     127.0.0.1:13000->3000/tcp
exp35b-dashboard-1     127.0.0.1:13001->3000/tcp
vpay-demo-dashboard-1  127.0.0.1:3000->3000/tcp

localhost:3000/   -> 307 (followed: 200)
localhost:13000/  -> 307 (followed: 200)
localhost:13001/  -> 307 (followed: 200)

exp35a-dashboard-1: VPAY_DASHBOARD_REDIRECT_URI=http://localhost:13000/dash/v1/callback
exp35b-dashboard-1: VPAY_DASHBOARD_REDIRECT_URI=http://localhost:13001/dash/v1/callback
```

Each stack also carried its own `demo_port`, `demo_receiver_port`,
`demo_orange_port`, `demo_checkout_port` and `demo_shop_port` (19080/19083/
19082/13080/13081 and 19180/19183/19182/13180/13181). No collision anywhere.

**The loopback binding still holds at a moved port** — the property exp28's
review closed, re-measured rather than assumed:

```console
$ curl -m 4 -o /dev/null -w '%{http_code}' http://10.10.0.227:13000/    # host's LAN address
000  (connection refused)
$ curl -m 4 -o /dev/null -w '%{http_code}' http://10.10.0.227:13001/
000  (connection refused)
```

### Cypress, against a moved stack, with another stack up beside it

`just demo_project=exp35a demo_port=19080 demo_receiver_port=19083
demo_orange_port=19082 demo_checkout_port=13080 demo_shop_port=13081
demo_dashboard_port=13000 test-e2e`, while `exp35b` held 13001 and the user's
`vpay-demo` held 3000 — **exit 0**:

| spec | tests | passing | failing | pending | skipped |
|---|---|---|---|---|---|
| `checkout.cy.ts` | 1 | 1 | 0 | 0 | 0 |
| `dashboard.cy.ts` | 8 | 8 | 0 | 0 | 0 |
| `shop-hosted.cy.ts` | 3 | 3 | 0 | 0 | 0 |
| `shop-embedded.cy.ts` (own run, `chromeWebSecurity: false`) | 4 | 4 | 0 | 0 | 0 |
| **total** | **16** | **16** | **0** | **0** | **0** |

The recipe's own probe reported `test-e2e: waiting for dashboard on
http://localhost:13000/`, and `dashboard.cy.ts`'s eight include *"signs a staff
member in through the real OP, enrolling their second factor on the way"* and
*"signs the same staff member back in with the password they set"* — i.e. the
full authorization-code leg against an OP whose registered `redirect_uri` names
:13000.

### The decisive mutation: does the sign-in actually depend on the port agreeing?

With stack A up and green, the overlay's redirect URI was edited to the port
the draft would have written — `:13000` → `:3000`, nothing else changed — and
`vpay-server` restarted:

```console
$ … VPAY_DASHBOARD_URL=http://localhost:13000 cypress run --spec cypress/e2e/dashboard.cy.ts
  ✓ sends an unauthenticated visitor to the sign-in form, and no further
  1 passing (2m)
  7 failing
  ✖  dashboard.cy.ts   01:59   8   1   7   -   -        exit 1
```

**8 tests, 1 passing, 7 failing.** The one that passes is the only one that
needs no OAuth. Every test that signs in fails. That is precisely what the
draft would have shipped for any `demo_dashboard_port` but 3000.

Restoring it by **regenerating** rather than by editing the file back also
re-proved F1's check:

```console
gen-demo-keys: .e2e/application-demo.yml does not register the dashboard app's
               redirect_uri on port 13000 — regenerating the pair
```

and the full recipe was then re-run from a torn-down stack — the gate re-running
the original failure, not just "does it build":

`just demo_project=exp35a … demo_dashboard_port=13000 test-e2e` → **exit 0,
16/16 again** (`checkout` 1/1, `dashboard` 8/8, `shop-hosted` 3/3,
`shop-embedded` 4/4).

### `just demo-up` with defaults is byte-for-byte unchanged

The user's `vpay-demo` held 127.0.0.1:3000 throughout this review, so binding a
scratch stack to the default port was not available without disturbing it. The
equivalent claim is made at the configuration layer instead, which is where the
change lives — the **rendered** three-file stack, no `VPAY_DEMO_*` set at all,
base `d5a93df` against this branch, build-context and bind-mount paths
normalised because the two were rendered from different directories:

```console
$ diff base.norm.yml branch.norm.yml && echo IDENTICAL
IDENTICAL
lines compared: 315
```

315 lines of rendered config, no difference — ports, environment, mounts, all
of it. And the substitution behaves at both ends:

```console
$ docker compose … config          # nothing set
  dashboard ports: [{host_ip: 127.0.0.1, target: 3000, published: '3000'}]
  redirect: http://localhost:3000/dash/v1/callback
$ VPAY_DEMO_DASHBOARD_PORT=13000 docker compose … config
  dashboard ports: [{host_ip: 127.0.0.1, target: 3000, published: '13000'}]
  redirect: http://localhost:13000/dash/v1/callback
```

**One** publication in each, not two, so the `!override` is doing its job; and
`host_ip: 127.0.0.1` in both, so the loopback property is not conditional on
the port being the default.

### `just ci`

See the commit that adds this file for the recipe-by-recipe numbers.

---

## What this review did NOT do

- **No browser-level confirmation of the `400` at `/authorize`.** The mutation
  proves the sign-in dies when the two strings disagree, and the mechanism is
  documented in three places in the repo, but the specific status code and
  error body were not captured from a server log — a direct probe of
  `/oauth/authorize` on `vpay-server` returned `unknown_route`, so that
  endpoint is not where a hand-written probe reaches it and chasing the right
  path was not worth the time once the spec-level evidence was in.
- **No second stack running Cypress concurrently.** `exp35b` was up and serving
  during stack A's e2e run, but only one suite executed at a time. Two
  concurrent e2e runs would also hit the shared-`.e2e/` limitation §7 already
  documents, which is issue #78's neighbour and not its subject.
- **`.e2e/` is still not keyed on `demo_project`** for the overlay or either
  merchant key pair. Bringing a second stack up still regenerates them and
  still leaves the older stack's `demo-walk` on `invalid_client`. That is the
  pre-existing limitation in §7, untouched here and not in #78's scope — but it
  does mean "two stacks coexist" holds for *serving*, including two dashboards,
  and not for two simultaneously-authenticating merchant walkthroughs.
- **`just demo-up` was never bound to the literal default port 3000** on this
  host, for the reason given above.
- **CI's e2e job was not run.** The workflow change is a comment.
- **Nothing was pushed.**
