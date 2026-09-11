# The dashboard — what slice 1 did NOT build, and what has been corrected since

_Split out of [docs/flows/dashboard.md](../dashboard.md) on 2026-09-11 by exp57, which broke a 865-line flow document into pages a person can read. **The text below is the original, unedited** — every dated measurement, every struck-through claim and every correction is here as it was written; only the relative links gained a `../` because the file moved one directory down._

## What slice 1 did NOT build

### ~~Nobody can sign in~~ — corrected 2026-09-07

**This was the headline of this section and it is no longer true.**
[ADR-0017](../../adr/0017-staff-authentication.md) took the decision this
paragraph said nobody had taken — how a human staff member proves who they
are — and `vpay_api::staff` serves the grant.
[dashboard-auth.md](../dashboard-auth.md) is the document that owns it; the short
form is: a vpay-owned `staff_members` table, argon2id with a deployment
pepper, mandatory RFC 6238 TOTP with a compare-and-swap replay guard,
server-side sessions with an absolute and an idle bound, and the
authorization-code grant with PKCE served for the dashboard client only.

`backends/tests/integration/tests/staff_sign_in.rs` (13 cases) drives it end
to end and **mints no token of its own**.

~~It is not built because building it requires a decision nobody has taken~~ —
and the paragraph that followed, about `authkestra-op` authenticating nobody,
is still accurate about `authkestra-op` and no longer a blocker: supplying the
`Identity` is exactly what `vpay_api::staff::oauth::authorize` does.

### ~~The `client_id` check is written for the grant we have~~ — resolved

Found by the 2026-09-06 review, recorded as finding F7 and deliberately not
changed then, because changing it meant choosing something reserved for the
maintainer. ADR-0017 decision 3 chose: **the credential is identified by
`aud`** — which the validator has already checked by the time
`require_dashboard_token` runs — **plus the merchant claim**, and `sub` names
the staff row and authorises nothing.

The second, smaller decision recorded beside it — that
`default_handle_authorization_code` mints `aud = <client_id>` with no
requested-audience path — is resolved in the same move, by changing the
_validator_ to expect what the grant produces rather than forking the handler.

~~**Consequence, stated plainly: the two routes above are a resource server
with no issuer.**~~ They have an issuer now. What was true and stays true is
why the tenancy boundary was built first: it has to be right _before_ a login
exists, not after.

### ~~There are still no pages~~ — corrected 2026-09-07 (exp28)

**This section said `frontends/apps/dashboard` was "still the scaffold, still
saying so on screen, still zero tests", and it is no longer true.** The pages
exist and a person can click them:

| Page              | What it is                                                                                                                                                                                                                                                           |
| ----------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `/login`          | Work email and argon2id password — leg one of ADR-0017's two factors                                                                                                                                                                                                 |
| `/login/totp`     | The six-digit code. A **first** sign-in renders the `otpauth://` QR and the base32 secret, and requires one valid code before the enrolment is committed                                                                                                             |
| `/login/password` | Replacing the one-time password `vpay-server staff add` printed. Not optional and not a nag: a session carrying `password_change_required` is refused by every authenticated route, `/oauth/authorize` included, so no `/dash/v1` token can exist until this is done |
| `/payments`       | The bound merchant's intents, newest first. Status and created-range filters, cursor paging                                                                                                                                                                          |
| `/payments/{id}`  | The intent, the charge, the refunds, the last error with its failure code, and the event timeline                                                                                                                                                                    |

`/` redirects to whichever of `/login` and `/payments` applies. It used to be
the whole app — a scaffold notice plus a legend of every status badge — and
that legend is gone with it: it was a reference for a payments list nobody had
written, and the list exists.

**The app is the OAuth client, and it runs the code leg server-side.** That is
[ADR-0017](../../adr/0017-staff-authentication.md) decision 4 rather than an
implementation choice, and it has one consequence a reader will go looking for
and not find: **there is no route at `redirect_uri`.** The app's own server
requests the code, follows the `302` and exchanges it, all inside one function
call, so `http://localhost:3000/dash/v1/callback` is a string the two OAuth
legs must spell identically and not a page. A browser never sees a code, a
verifier or a token.

That string is the demo stack's, and its port stopped being a literal on
2026-09-10 (issue #78): `justfile`'s `demo_dashboard_port` is written into the
app's `VPAY_DASHBOARD_REDIRECT_URI` and into the generated overlay's
`dashboard_client.redirect_uris` together, so `just demo_dashboard_port=13000
demo` spells `http://localhost:13000/dash/v1/callback` in both. Nothing about
the flow changes — the identity of the two strings is still the property that
matters, and it is still matched byte for byte.

The `/dash/v1` access token is not kept in the app either. The token endpoint
writes it to the `staff_sessions` row, and every render reads it back from
`GET /dash/v1/staff/session` — which is exactly what makes signing out a
revocation: delete the row and there is nowhere left to read it from.

**It is re-minted when it expires, and it has to be.** That token's TTL is
fifteen minutes (`vpay_api::op::ACCESS_TOKEN_TTL_SECS`) and a staff session's
is thirty minutes idle, twelve hours absolute — so a page that used the token
on the row and nothing else answered "the bearer token is invalid, expired"
for the rest of every sign-in past its first quarter of an hour. That is what
it did until the exp28 review measured it. `src/server/dash-read.ts` retries a
`401` once, after running the same authorization-code leg the first render
runs, which re-reads the staff row and re-checks the account, the merchant
binding and `password_change_required` — so re-minting is _more_ checking than
carrying one token for twelve hours, not less. A `403` is never retried.

**Every server action opens with an origin check, and it does not compare two
values the caller sets.** An export of a `'use server'` file is a `POST`
endpoint anything on the internet can reach — Next registers an action id for
it — and this app's session cookie is `SameSite=Lax` rather than `Strict`, so
a top-level form submission from another site carries it.

Next has its own check and it is not the one this deployment wants:
`app-render/action-handler` compares `Origin` against the host, and the host
it uses is **`x-forwarded-host` when present**, falling back to `Host`. Behind
a proxy that does not strip incoming forwarding headers — the default for
several — a caller who sends `X-Forwarded-Host: evil.example` and
`Origin: https://evil.example` makes the two agree and passes.

`server/csrf.ts` compares `Origin` against **`VPAY_DASHBOARD_PUBLIC_ORIGIN`**,
which no caller can influence, and `originIsAllowed`'s signature is the guard:
it takes the `Origin` and the `Host` and there is no third parameter for a
forwarding header to arrive through, so the decisive mutation is a signature
change rather than a one-character edit (issue #88 item 4). An absent `Origin`
is refused — the check must not be removable by removing a header.

**The variable is optional, and what a deployment gets without it is stated
rather than hidden.** With it unset the check compares the `Host` header —
still never `X-Forwarded-Host`, so strictly stronger than Next's own — which
is right for a dashboard a browser reaches directly and **wrong behind a
proxy that rewrites `Host`**, where every action is refused with one sentence
and a line in the container log naming the variable. It is optional because
making it required would have taken the sign-in down for every deployment not
yet reconfigured, this repository's own compose stacks included.

_Amended 2026-09-10 by the exp36 review (finding F7)._ **This check does not
replace Next's own — both run, and an action needs both to pass.** Measured
against a booted stack, firing the real `signIn` action id with chosen
headers:

| Configured origin        | `Origin`               | `Host`                    | `X-Forwarded-Host` | Result                                                                                                     |
| ------------------------ | ---------------------- | ------------------------- | ------------------ | ---------------------------------------------------------------------------------------------------------- |
| `http://localhost:13200` | same                   | honest                    | —                  | reaches the action                                                                                         |
| `http://localhost:13200` | `https://evil.example` | `evil.example`            | `evil.example`     | **refused by vpay** — and this is the whole of item 4: all three agree, which is exactly what Next accepts |
| `http://localhost:13200` | absent                 | honest                    | —                  | **refused by vpay**                                                                                        |
| _(unset)_                | `https://evil.example` | honest                    | `evil.example`     | **refused by vpay** — Next accepts this one too                                                            |
| `http://localhost:13200` | same                   | `vpay-dashboard.internal` | `localhost:13200`  | reaches the action                                                                                         |
| `http://localhost:13200` | same                   | `vpay-dashboard.internal` | —                  | **refused by NEXT**, `500`                                                                                 |

The last row is the operational consequence and nothing else in this
repository said it: **setting `VPAY_DASHBOARD_PUBLIC_ORIGIN` is necessary and
not sufficient behind a proxy that rewrites `Host`.** vpay's check passes, and
Next's own then aborts the action with

```
`x-forwarded-host` header with value `vpay-dashboard.internal` does not match
`origin` header with value `localhost:13200` from a forwarded Server Actions
request. Aborting the action.
```

— a `500`, a message naming neither this app's sentence nor the variable, and
nothing to point an operator at the cause. Such a proxy must **also** send
`X-Forwarded-Host` matching the public host, which every ordinary reverse
proxy does; the row above it is that deployment, and it works. If one ever
turns up that cannot, the lever is `serverActions.allowedOrigins` in
`next.config`, and it is deliberately not pulled here: widening Next's own
check is a security change that would want its own review, and no measured
deployment needs it.

_Amended 2026-09-10 by the exp36 review (finding F3)._ **It is set now** —
`compose.e2e.yml` gives the dashboard
`VPAY_DASHBOARD_PUBLIC_ORIGIN: http://localhost:${VPAY_DEMO_DASHBOARD_PORT}`,
keyed to the same variable as the publication and the registered redirect URI,
so the configured path is the one `just test-e2e` exercises rather than the
one nothing ever did. The chart carries `dashboard.publicOrigin` and a
`dashboard-public-origin` guard on its **shape**; no template reads it,
because this chart writes no dashboard workload, and the README says so where
the key is. It stays **optional** until that Deployment exists: a required
value on a workload nothing renders would fail a deployment for a setting
nothing reads. That is the remaining half of the follow-up, and
`docs/status.md` carries it.

**A vpay this app cannot reach is not a sign-out.** Every failure of the
session read used to send a browser to `/signed-out`, and `server/api.ts`
deliberately turns a rejected `fetch` into an `ApiFailure` with `status: 0`
rather than throwing — so "vpay is restarting", "the connection was reset" and
"vpay answered `503`" were indistinguishable from "your session is over". A
rolling restart therefore signed every staff member out of the dashboard, and
they could not tell that from having been signed out on purpose (issue #88
item 2).

`server/gate.ts`'s `refusalFor` is the whole of the fix and it is a pure
function with its own unit tests, because "a `503` signs everybody out" should
be a red test rather than something noticed during an incident. **`401` is the
only status that ends a session**, and the mapping is exact rather than
conservative: vpay answers `401` for _every_ session refusal by design —
absent, expired, idle, forged, disabled, at the wrong stage — so there is no
other status that could mean the session is over. Everything else renders the
message and its request id on the page, **with the cookie untouched**. A `403`
on the session route counts as an outage too: it would mean something in front
of vpay refused this app, which is a deployment problem and not a fact about
the person. The decisive mutation is widening `refusalFor` to `status >= 400`,
which turns four cases in `gate.test.ts` red.

> **`refusalFor` is about the SESSION READ, and only about it.** _Recorded
> 2026-09-10 by the exp36 review._ "`401` means the session is over" holds
> because `GET /staff/session` and `/oauth/authorize` have nothing else to
> refuse. On a route that also refuses a **credential** the same `401` means
> two things and a caller cannot tell them apart, which is the price of "every
> refusal is one answer" — so on such a route it must end nothing.
> `changePassword` learned that the hard way (finding F1) and no longer
> clears the cookie on one.
>
> ~~**`submitTotp` still does, and this review left it alone.**~~ **Closed
> 2026-09-10 (exp44).** A wrong six-digit code is a `401` there, so a mistyped
> code sent a person back to the email-and-password form rather than telling
> them — and took the sealed enrolment blob with it, so a first sign-in could
> not even be retried. The exp36 review recorded it (F6) and did not fix it,
> for a reason that was accurate: unlike `changePassword` the two lines could
> not simply be dropped, because `/login/totp` read no session on render —
> only the cookie's presence — so with the cookie kept a session that really
> was over would leave the person retyping codes at a form that could never
> accept one. Closing it meant giving that page the session read
> `PasswordPage` has, and `GET /staff/session` cannot serve it: that route is
> refused before the second factor, with the same `401` a dead session gets.
> So the page reads `GET /dash/v1/staff/session/stage` instead — an eighth
> staff route that answers the stage and nothing about the person — and
> `server/gate.ts::totpGateFor` is the decision, in the same shape and for the
> same reason as `refusalFor` beside it. `docs/flows/dashboard-auth.md`, "A
> mistyped code does not end a session either", carries the argument and the
> proof.

**There is one route that is not a page**, and it exists for a Next rule
rather than for a person: `GET /signed-out` clears the session cookie and
redirects to `/login`. A page may not write a cookie — `cookies().set` throws
outside a Server Action or a Route Handler — so a protected page that has
decided a session is unusable redirects here instead of clearing it itself.
Doing the clearing in the render is what made every such refusal a `500` until
the exp28 review. It forgets the cookie only for a top-level navigation
(`Sec-Fetch-Dest`), because a `GET` that clears a cookie is otherwise the
`<img src>` forced-logout the sign-out button is a POST to avoid.

**What the pages deliberately do not show:**

- **No "Rail" column on the list.** `GET /dash/v1/payment_intents` returns no
  charge at all, so the only rail-shaped value in that response is
  `payment_method_types` — the rails an intent _may_ be confirmed against. The
  column is headed **Methods**, because that is what it is; a "Rail" heading
  over it would be wrong for every intent that offers two and was taken by
  one. The detail page has a real `Rail`, from `charge.provider_code`.
- **No payer column on the list**, for that reason and a second one — see the
  next section.
- **No page count.** `/dash/v1` serves cursors and a `has_more`, and no count
  anywhere. "Page 3 of 12" would be either a `COUNT(*)` this API does not
  offer or a number this app made up. `Next` appears only when `has_more` said
  so — never because a page came back full, which would put a link onto an
  empty list and read as data having been lost.
- **A status this build cannot name renders as text, not as a coloured pill.**
  A green badge on an unfamiliar status is a claim.
- **The timeline says what is missing from it.** `events.type` is constrained
  to eight documented types (migration `0018`, extended by `0029`) and **five
  of them are written by nothing at all** (`../status.md`, "Events written by
  the worker"): only `payment_intent.succeeded`,
  `payment_intent.payment_failed` and `checkout.session.expired` are ever
  emitted, by settlement and by the housekeeping sweep. So a succeeded
  payment's timeline is one line — and a section headed "Timeline" with one
  line on it reads as everything that happened to that payment, which is the
  same failure as an empty table that means "the read was refused". The page
  therefore names the five missing types under the section, on screen rather
  than only here, and `payment-detail.test.tsx` pins the sentence.

**Configuration is read at container start and fails closed.** The API base
URL, the dashboard `client_id`, its `redirect_uri` and its scope come from the
environment through bracket notation, so `next build` cannot inline one
deployment's values into the image. A missing one is not defaulted: `/login`
renders the variable names an operator has to set, and offers no form.
Nobody types a password into a page that could not have used it.

The **merchant** is not configured in the app at all — it is read from
`GET /dash/v1/staff/session`, which answers from the session row. It is
rendered on every signed-in page beside the staff member's address, because an
operator looking at an empty payments list has to be able to tell "this
merchant has no payments" from "I am looking at the wrong merchant", and
nothing else on the page answers that.

**The nav rule is a gate, twice over.** `NAV_LINKS` is an exported constant
and `src/layout.test.tsx` resolves every entry against `app/**/page.tsx` on
disk _and_ checks the rendered markup for a dangling internal `href` — the
constant catches a link rendered only in a branch a test never exercises, and
the markup catches a link written straight into the JSX. Adding
`{ href: '/webhooks' }` fails both.

**Styling substrate** (exp26 Lane D, `docs/plans/2026-09-07-ui-revamp.md`
§4.2): `@base-ui/react` + Tailwind 4 + daisyUI 5 through `@vpay/ui`, theme
`bumblebee`. Every page and every component composes `@vpay/ui`; there is not
one `className` string anywhere under `app/` or `src/` outside the tests.
The four "recipes" that lane compiled are now the components the pages render
(`src/recipes/` → `src/components/`), and the sign-in recipe — one leg, no
password control, because the auth decision was open when it was written — is
replaced by the real two-leg forms.

### Two columns the list cannot show

- **The payer's phone is always blank.** `charges.payer_ref_masked` is never
  written — the confirm path stores `None`
  (`vpay_api::v1::payment_intents`'s `open_attempt`). The detail read
  renders the column rather than deriving a mask from the unmasked
  `payer_ref`, so the field appears the day the column is written and not
  before. `docs/status.md` carries the gap.

  **The page renders it as an em dash, `—`, and that dash is a real `null`
  rather than a hard-coded string**: it comes from the column, on a payment
  that _has_ a charge, and `dashboard.cy.ts` walks the list until it finds one
  before asserting it — a dash on an intent nobody confirmed would prove
  nothing. The row exists rather than being omitted so that the day the column
  is written the value appears, instead of a field having to be added then.
  What must not happen is this quietly becoming the _unmasked_ value because
  the masked one was empty, and `payment-detail.test.tsx` pins both directions.

  The **list** has no payer column at all, which is a second and stronger
  reason: `GET /dash/v1/payment_intents` does not return a charge, so there is
  no `payer_ref_masked` in that response to be null. A column there would be
  sourced from nothing.

- **There is no search by phone**, for the same reason: a filter over a
  column that is always `NULL` answers "no results" for every payer who ever
  paid, which reads as an answer.

### No writes, no other slices

ADR-0008's per-record write operations (re-poll a charge, replay a webhook,
issue a refund, annotate a charge) are not built and are not scoped —
`dashboard-auth.md`'s "Scope" section explains why the registration still
has exactly one scope. Slices 2–6 are untouched.
