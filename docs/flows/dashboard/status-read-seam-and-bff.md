# The dashboard — Status: the `/dash/v1` read seam, the BFF, and its security review

_Split out of [docs/flows/dashboard.md](../dashboard.md) on 2026-09-11 by exp57, which broke a 865-line flow document into pages a person can read. **The text below is the original, unedited** — every dated measurement, every struck-through claim and every correction is here as it was written; only the relative links gained a `../` because the file moved one directory down._

**The two `/dash/v1` reads are one module now, 2026-09-11 (exp55 Lane 1).**
`/payments` and `/payments/{id}` each assembled their own path, called
`readDash`, and unpacked the envelope themselves. `frontends/apps/dashboard/src/dash/provider.ts`
is those three decisions extracted once, named `getList` / `getOne` the way a
data provider names them, so that the eventual framework is a change of caller
rather than a rewrite of the read path. **No framework is installed and none
is imported**; the module's shapes are this app's own, and nothing under
`src/server/` — `readDash`'s single-`401` re-mint included — was touched.

One rule moved inside `src/payments-query.ts` while this happened, and it is
the one worth naming: the direction-sensitive reading of `has_more` (§ "the
paging links"). The pages want two hrefs and the seam wants two cursors, so
`pageCursors` is the single copy of the rule and `pagerHrefs` is its caller.
Deriving it twice would be two answers to "which end of the list is this", and
the symptom of a disagreement is a paging link onto an empty page — which
reads as data having been lost. Deleting the inversion fails
`src/dash/provider.test.ts`'s `a backward page reports NEWER rows, not older
ones` and two of `payments-query.test.ts`'s existing cases. Nothing a staff
member sees changed.

**The dashboard has a browser-reachable read surface of its own now, and
nothing uses it — 2026-09-11 (exp55 Lane 2).** Two `GET` route handlers,
`/api/dash/payment_intents` and `/api/dash/payment_intents/{id}`, authenticate
on the same httpOnly session cookie and proxy to `/dash/v1` with the token
read out of the `staff_sessions` row on that request. Until they existed
**nothing in this app was reachable from a browser except a page and four
Server Actions**, and that sentence was a property of the design rather than
an accident of it — which is why this is recorded here and not folded into
the lane that will consume it.

**Whether the app may have such a surface at all is the maintainer's
decision.** It is RD5 in
[the Refine plan](../../plans/exp55-refine-seam-bff-notes/refine-plan.md) §8. No
page, no component and no Cypress spec calls these handlers; deleting them
breaks nothing else in the app.

What they do about being reachable, each written as a mutation that was run
and reverted:

1. **A request with no session cookie reaches vpay not at all.** The cookie is
   read before anything is sent, and the test asserts the stubbed `fetch` was
   never called — not that the status was `401`. An endpoint that answers
   `401` _after_ asking vpay about the absent session is a surface anyone can
   use to make this server open a connection to vpay, once per request.
2. **A request this dashboard did not issue is refused by `csrf.ts`'s own
   rule.** `originIsAllowed`, the same function the four Server Actions are
   fronted by, with no second copy of it — plus `Sec-Fetch-Site: same-origin`,
   because **a browser sends no `Origin` at all on a same-origin `GET`** and
   script cannot add one, so `Origin` alone could not answer the question on
   this method. A request with no fetch metadata at all is refused, which is
   the opposite of what `/signed-out` does with the same absence and
   deliberately so: that route serves a person arriving on a page, this one
   serves this app's own script.
3. **The bearer token is in no response header and no response body**,
   asserted against the serialised response while the stubbed vpay echoes it
   into a header, into the list envelope's `url` and into an extra top-level
   field. The header half is structural rather than disciplined: `api.ts`'s
   `getJson` answers an `ApiResult`, so the handler never holds the upstream
   `Response` and there is nothing in scope to copy a header off.

**And no caller-supplied merchant id, audience or scope is forwarded.** The
upstream query string is built by `apiQueryString` from the five parameters
`queryFrom` reads, so `?merchant_id=…&audience=…&scope=…` is not denied — it
is never looked at, which is the property that also holds for the next
parameter somebody invents. The tenant stays `MerchantScope::for_dashboard`,
read server-side from the YAML dashboard binding, exactly as § "Merchant scope
on every query" above requires.

Two gaps are named rather than papered over. The handler's token gate is a
second transcription of `requireStaff`'s branch table — the decisions are
reused and only the acting differs, because a `307` to `/login` answered to a
`fetch` is HTML with a `200` by the time the caller sees it — and two copies
can drift. And a session that still owes a password change gets a `403` where
a page would send it to `/login/password`: an endpoint has nowhere to send
anybody, and no client exists yet to route the person.

**The security review of that surface, 2026-09-11 (exp55).** The three checks
above were re-run as mutations by a reviewer rather than taken on the
implementer's word, and each went red on the case it names. What the review
found on top of them, and what it did about it:

- **A refusal with no response behind it named vpay's internal address.**
  `api.ts`'s `unreachable` writes its message out of the thrown error, so the
  `502` carried `connect ECONNREFUSED 10.42.3.17:8080
(vpay-server.vpay-prod.svc.cluster.local)` to the browser, and a `200` that
  was not JSON carried the first bytes of whatever answered instead. On a page
  that reaches a staff member who is already signed in; here it is a
  scriptable endpoint reached by **any** cookie value, because the read that
  fails is the session read. Fixed: a `status: 0` now answers this surface's
  own sentence, and `api.ts` is untouched.
- **A `200` whose body was not the expected document answered `500`.**
  `getJson` casts the parsed body to `T` unchecked and `pageCursors` indexes
  `rows[0]`; five of six malformed shapes threw a `TypeError` out of the
  handler, which Next answers as its own error, and the sixth answered `200`
  with rows that were an object. The page render had the same exposure through
  the same seam. Fixed in `src/dash/provider.ts`: such a body is a `502`
  refusal, never an empty list — an empty list is a claim about the merchant's
  payments.
- **Parameter smuggling and the `[id]` segment were attacked and held.**
  Duplicated parameters, `status[]`, `__proto__`/`constructor`/`prototype`,
  percent-encoded and full-width spellings, a 200 kB value, and an id that is
  a URL, a link-local address, `../staff/session`, `//evil.example/x` or
  carries `?`, `#`, `%2F` or a CRLF: none reached the upstream URL or headers
  unescaped, and `Object.prototype` was untouched. The cases are now in
  `bff.test.ts` so a later change cannot quietly make one of them reach it.
- **Two claims in the code were corrected rather than the code changed.** The
  test file named the list case as the one pinning the projection; it is the
  detail case — `getList` has already rebuilt its shape field by field, so the
  list handler's projection renames three keys and drops nothing. And "absent
  `Sec-Fetch-Site` is a pre-2020 browser" understated the availability cost:
  **Safari has sent it only since 16.4 (March 2023)**, so this surface refuses
  every older WebKit with a `403`. That costs nothing while nothing calls
  these handlers, and is a decision for whatever eventually does.

**The first of those two corrections was made in the wrong place, and exp56
finished it.** The review rewrote the test file's _header_ and left the marker
inside it — `// THE THIRD DECISIVE CASE`, sitting on the list case — which is
the comment nearest the code and the one a reader would actually follow. They
would have deleted `project`, watched the case they were pointed at stay
green, and concluded the projection was dead weight. Both now name
`puts it in no part of the detail read either`.

Re-running that mutation also measured something the review's sentence rounds
off: it fails **two** cases, and only one of them is about a credential. The
other is `answers the page and its two cursors, and nothing vpay sent besides`,
which fails on the envelope's key names — `data, hasMore, cursor` where the
wire promises `object, data, has_more, cursor`. That is evidence about the
contract this surface publishes, not about the token staying in, and it is
named because two red cases read as more corroboration than one when they are
two different claims.

~~**What the review could not check, and what would:** that a real browser's
`Sec-Fetch-Site`, `Origin` and cookie arrive as this code assumes. Every case
here is a synthetic `Request` against a stubbed `fetch`. Only a Cypress spec
against `compose.e2e.yml` can answer it, and there is none.~~

**Answered 2026-09-11 (exp56), and the browser agreed with the code.** Five
cases in `frontends/tests/e2e/cypress/e2e/dashboard.cy.ts` drive this surface
from a real browser against the real stack, signed in through the real OP.
`cy.intercept` is used there as a **spy and never as a stub** — nothing is
faked, every request reaches the dashboard and vpay — because it is the only
way to read the headers the browser put on the wire and the status the server
answered when the calling page is not allowed to see it.

What Chrome 152 actually sent, identical under Cypress's bundled Electron 138:

| the request                         | `Origin`                | `Sec-Fetch-Site` | cookie | answer |
| ----------------------------------- | ----------------------- | ---------------- | ------ | ------ |
| `fetch` from `/payments`, signed in | **absent**              | `same-origin`    | sent   | `200`  |
| the same, cookie cleared            | absent                  | `same-origin`    | none   | `401`  |
| `fetch` from the shop (`:3101`)     | `http://localhost:3101` | `same-site`      | sent   | `403`  |
| `<iframe src>` from the shop        | **absent**              | `same-site`      | sent   | `403`  |
| `OPTIONS` from `/payments`          | `http://localhost:3100` | `same-origin`    | sent   | `405`  |

Three things in that table were assumptions in `bff.ts` and are measurements
now. A browser really does send **no `Origin` at all** on a same-origin `GET`,
which is the sentence the whole `Sec-Fetch-Site` rule rests on. It sends one
on `OPTIONS` from the very same page — `Origin` is attached to every method
that is not `GET` or `HEAD`, so it is present exactly on the methods this
surface does not serve. And the httpOnly cookie is attached by the browser on
all five, including the two the page's own script could not have added it to.

**The frame case is the decisive one and it is the only thing in this
repository that pins `Sec-Fetch-Site`.** An `<iframe src="…">` is a
_navigation_, so it carries no `Origin` for `originIsAllowed` to refuse — and
`SameSite=Lax` does not withhold the session from a **same-site** request,
which `localhost:3101 → localhost:3100` is, because a port is not part of a
site. So the request arrives with a signed-in staff member's cookie and
nothing but `Sec-Fetch-Site` saying where it came from.

Measured by mutation: delete the `Sec-Fetch-Site` comparison from
`apiIsSameOrigin`, leaving the `Origin` check, rebuild the image and re-run.
`refuses a cross-origin FRAME of the same URL, which carries no Origin at all`
fails with **`200` where `403` was expected** — a merchant's payment list
served into an arbitrary same-site page. Every other case stays green,
`refuses a cross-origin fetch from another site's page, cookie and all`
included, because a cross-origin **`fetch`** does carry an `Origin` and is
refused a step earlier. One case red, fifteen green, and it is the right one.

**What the browser run still does not cover:** Safari, and therefore the
absent-`Sec-Fetch-Site` refusal that the review measured as this rule's
availability cost (WebKit has sent the header only since 16.4, March 2023).
`cypress run` uses Electron by default here and this evidence was gathered in
Chrome; neither is WebKit, and no case exercises a client that sends no fetch
metadata at all.

**The `OPTIONS` oracle is closed, 2026-09-11 (exp56).** The review measured
that Next auto-implements `OPTIONS` as a `204` carrying
`Allow: GET, HEAD, OPTIONS`, answered from a closure built at route-compile
time — so **before** the origin check and before the cookie was read. It was
the one answer this surface could give without passing its own gate, and it
was recorded rather than fixed. `frontends/apps/dashboard/middleware.ts` now
matches `/api/dash/:path*` and answers `405` with **no `Allow` header** to
every method that is not `GET` or `HEAD`. `HEAD` is deliberately let through:
Next binds it to the `GET` handler itself, so it takes the identical gate, and
`/dash/v1` serves it too.

A middleware rather than an `OPTIONS` export in each `route.ts` because
`:path*` covers the third route nobody has written yet, which an export would
have to be remembered for. The decisive case is
`answers OPTIONS 405 with no Allow header, before the route module can` —
make `middleware` return `NextResponse.next()` unconditionally and it goes red
on the status and on the header, along with two more of the seven in
`middleware.test.ts`. The case beside it imports Next's **own**
`autoImplementMethods` and runs the real route module through it, so the `204`
and the `Allow` string are measured against the installed Next rather than
quoted from the review. That mattered: **the review named Next 16.3.4, which
is `examples/shop`'s pin and not this app's.** The dashboard resolves 15.5.25,
whose `AUTOMATIC_ROUTE_METHODS` and `Allow` assembly are the same, so the
finding held against a version it was not measured on.

**What it closed and what it did not, measured against a real `next start`.**
The middleware runs on the matched subtree before routing, so `OPTIONS
/api/dash/nope` — a path no route file serves — answered `404` before and
answers the same `405` as the real routes now. For `OPTIONS`, route existence
is genuinely unanswerable. **For `GET` it is not:** an unauthenticated `GET`
to a route that exists reaches `bff.ts`'s gate where a path nothing serves
gets Next's `404`. `OPTIONS` was the loudest discriminator and the only one
reachable without passing a check, never the only one — making the `GET` pair
uniform would mean answering `404` to an honest signed-out client, and the
route names are in this repository anyway. `answers alike for a path in the
subtree that no route file serves` pins the half that is closed.

**The one thing a reader must not conclude from this document:** that the
dashboard is finished. ~~Two `GET` routes exist that nobody can authenticate
to.~~ _Corrected 2026-09-07._ A staff member can sign in and read this
merchant's payments. What the dashboard still cannot do is anything at all to
them: there is no write path, no other slice, and no audit log — because
there is nothing yet to audit.

**Issue #88 item 3 — the end-to-end paging test — landed 2026-09-11 (block
2J), then a review fixed a vacuousness bug in it the same day.** The
seventeenth case in `dashboard.cy.ts` pages forward through the list and back,
tying together the API's cursors, `pageCursors` in `payments-query.ts`, and
the BFF proxy. As first written it clicked "Next" **only if a Next link
already existed**, falling back to a "stable on revisit" assertion otherwise
— and measured, by the time that test runs in `e2e:default`'s spec order,
this tenant carries exactly three payments against a `PAGE_SIZE` of 25 (one
from `checkout.cy.ts`, which runs first alphabetically, two from earlier
tests in this same file; `shop-hosted.cy.ts` and `shop-embedded.cy.ts` both
run later). The forward-paging branch was therefore never the branch that
ran — a single-page result every time, silently taking the untested arm. The
issue comment reporting this block ("`ea9c442`... syntax verified... test
will run in CI") did not say that; it is corrected here.

**Fixed, same day, in the review:** the test now mints 30 extra unconfirmed
PaymentIntents on the tenant through a new `mintPaymentIntentsForPaging`
Cypress task (`cypress/tasks/checkoutTasks.ts`) before visiting `/payments`,
and the "Next" link assertion has no conditional around it — a missing link
is now a real failure. **What is still unverified:** this fix is checked by
`tsc --noEmit` and `eslint --max-warnings 0` only. It has not been run
against `compose.e2e.yml`, and neither has the decisive mutation the brief
asked for — breaking the backward-page `has_more` inversion in `pageCursors`
and confirming this Cypress case (rather than only the existing unit case in
`src/dash/provider.test.ts`) goes red. Bringing up the full compose stack to
do either was judged too heavy for the shared host this review ran on
(concurrent builds had already OOM-killed a run earlier the same day); both
remain open for whoever next runs `just test-e2e` for real.
