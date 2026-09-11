/**
 * The dashboard, end to end, against the real stack from `compose.e2e.yml`:
 * real Next.js, real `vpay-server`, real Postgres, real argon2id, real
 * RFC 6238, real RS256, with WireMock standing in for the payment rails.
 *
 * **Nothing is stubbed inside the browser, and nothing about the sign-in is
 * stubbed anywhere.** The password is the one `vpay-server staff add` printed
 * into a file; the TOTP codes are computed from the secret THE ENROLMENT
 * SCREEN DISPLAYED, so what this proves is that an authenticator app enrolled
 * from that QR would work — not that a secret the test also knows verifies
 * against itself. Every refusal asserted below is vpay's own.
 *
 * # The five legs, and why there are five rather than three
 *
 * A freshly created staff member has to go through all of them, and skipping
 * any one of them is not something a real person could do:
 *
 * 1. `/login` — email and argon2id password.
 * 2. `/login/totp` — enrolment is **mandatory at first sign-in** (ADR-0017
 *    decision 1): a session never reaches `authenticated` while
 *    `staff_members.totp_secret` is `NULL`.
 * 3. the same screen — a mistyped code first, which must show the error and
 *    keep both the session and the sealed enrolment blob (the exp36 review's
 *    F6), and then one valid code, which is what commits the enrolment.
 * 4. `/login/password` — `staff add` sets `password_change_required`, and
 *    since 2026-09-10 the change also needs the password IN FORCE (issue #79
 *    item 3): the wrong one is refused in the browser here, and the printed
 *    one is what completes it. The other half of that item — every OTHER
 *    session of the same staff member is deleted — needs two browser sessions
 *    at once and is proven in
 *    `staff_sign_in.rs::changing_a_password_needs_the_current_one_and_ends_every_other_session`
 *    instead. This spec does not claim it.
 *    every authenticated route refuses such a session, `/oauth/authorize`
 *    included. So no `/dash/v1` token can exist until this is done.
 * 5. only then `/payments`, rendered from a token minted by the
 *    authorization-code grant with PKCE, whose exchange this app's own server
 *    performed.
 *
 * # Two legs that are about the DEMO rather than about the dashboard
 *
 * "takes a payment through the shop…" and "shows the demo staff member that
 * payment…" are exp51, and together they are the only thing in this
 * repository that fails if the demo dashboard is bound to a tenant nobody
 * clicks anything in. The stack registers two merchant clients on two tenants
 * and `/dash/v1` reads exactly one. Nothing about them is stubbed either: the
 * payment asserted on is one this spec made through a merchant's own website
 * with a phone number, not one it minted with a merchant credential. Two
 * tests and not one because a test's primary origin is its first `cy.visit`
 * — the first comment in the pair says what that cost when it was one.
 *
 * # And one leg that is about a token rather than about a person
 *
 * "replaces the access token before it expires" is issue #88 item 1, and it is
 * here rather than in a unit test because the thing it proves is a **sequence
 * of renders across an expiry**. It depends on this stack's short
 * `staff_auth.access_token_ttl_seconds` (`demo_staff_token_ttl`, thirty
 * seconds) — at the shipping 900 no browser run could reach the case at all,
 * which is why nothing ever had.
 */
import {
  carriedForward,
  carryForward,
  totpDigits,
  waitForNextTotpStep,
} from "../support/dashboard";
import {
  MTN,
  buyOnVpaysPage,
  checkoutOrigin,
  orderIdFromUrl,
  readOrder,
  shopUrl,
} from "../support/shop";

/** The staff address `just demo-staff` created. */
const staffEmail = (): string => Cypress.expose("STAFF_EMAIL") as string;

/** A password this spec sets, replacing the printed one-time password. */
const NEW_PASSWORD = "a-long-enough-demo-password";

/**
 * The two values a test here leaves for a LATER test, by name.
 *
 * They live in Node (`carryForward`/`carriedForward`, `../support/dashboard`)
 * rather than in `Cypress.env`, and that is not a preference: the shop leg
 * below moves this spec's primary origin off the dashboard's port, and a
 * runtime `Cypress.env` write does not survive it. Both of these were read
 * back as `undefined` — measured on CI run 34555068739 and locally — while
 * the payment they were about sat in Postgres in the right tenant.
 * `cypress/tasks/dashboardTasks.ts` has the mechanism.
 */
const TOTP_SECRET = "dashboardTotpSecret";
const SHOP_PAYMENT_INTENT_ID = "shopPaymentIntentId";

/**
 * # Why this spec turns test isolation off
 *
 * Cypress clears cookies before every test by default, and this spec is a
 * **sequence**: sign in once, look around while signed in, sign out, sign back
 * in. With isolation on, the four tests after the sign-in all start signed out
 * — measured, and every one of them failed on a page that had correctly
 * redirected to `/login`.
 *
 * The alternative is a `beforeEach` that signs in again, and it is worse than
 * it looks: vpay's TOTP replay guard is a compare-and-swap on
 * `staff_members.last_totp_step` admitting only a strictly greater step, so
 * six sign-ins need six *different* 30-second windows — a spec that spends
 * three minutes waiting for clocks in order to test a payments list.
 *
 * The order below is therefore load-bearing, and each test says what it
 * assumes about the one before it.
 */
describe("the dashboard", { testIsolation: false }, () => {
  it("sends an unauthenticated visitor to the sign-in form, and no further", () => {
    // Runs first, in a fresh browser: nothing has signed in yet.
    cy.clearCookies();
    cy.visit("/payments");
    cy.location("pathname").should("eq", "/login");
    cy.contains("h2", "Sign in").should("be.visible");

    // And it does not loop: /login renders a form rather than redirecting
    // again. Every refusal on this path is one 401, and the answer to all of
    // them is this page.
    cy.get("form").should("exist");
    cy.location("pathname").should("eq", "/login");
  });

  it("signs a staff member in through the real OP, enrolling their second factor on the way", () => {
    cy.visit("/login");
    // Evidence, committed under docs/plans/exp28-dashboard-pages-notes/. Named
    // rather than automatic so the four screens a reviewer needs are the four
    // that exist, and taken before the fields are typed into so no credential
    // is ever in an image.
    cy.screenshot("01-login", { capture: "viewport" });

    // ---- leg 1: the password -------------------------------------------
    cy.task<string>("staffPassword").then((password) => {
      cy.get("#dashboard-signin-email").type(staffEmail());
      cy.get("#dashboard-signin-password").type(password, { log: false });
      cy.contains("button", "Sign in").click();
    });

    // ---- leg 2: enrolment ----------------------------------------------
    cy.location("pathname").should("eq", "/login/totp");
    // The QR and the typed key are both here, and both are the SAME secret:
    // the page reads the key out of the URI the QR encodes.
    cy.get('[data-testid="totp-qr"]')
      .should("be.visible")
      .and("have.attr", "src")
      .and("match", /^data:image\/png;base64,/);
    // The QR and the secret ARE in this image, and that is deliberate: it is
    // the screen under review, the secret belongs to a staff member this stack
    // creates fresh on every run, and the stack is torn down with `down -v`
    // moments later. It authenticates nothing that will exist tomorrow.
    cy.screenshot("02-enrolment", { capture: "viewport" });

    // ---- leg 3a: a MISTYPED code must not sign anybody out ---------------
    //
    // The exp36 review's F6, in a browser. `POST /dash/v1/staff/totp` answers
    // the same 401 for a wrong six-digit code as for a session it refuses, and
    // `submitTotp` read every one of them as "the session is over": it cleared
    // the session cookie AND the sealed enrolment blob, so a typo sent the
    // person back to the email-and-password form and a first sign-in could not
    // even be retried — the retry would have carried no secret to commit.
    //
    // Three assertions, and the QR is the one that would be missed: the page
    // has to still be the ENROLMENT page, not just still be at this path.
    cy.get("#dashboard-totp-code").type("000000");
    cy.contains("button", /finish enrolment/i).click();
    cy.location("pathname").should("eq", "/login/totp");
    cy.get('[role="alert"]').should("be.visible");
    cy.get('[data-testid="totp-qr"]').should("be.visible");
    cy.getCookie("vpay_dash_session").should((cookie) => {
      expect(
        cookie?.value ?? "",
        "the session cookie after a wrong code",
      ).to.not.equal("");
    });

    // ---- leg 3: a code computed from what the screen showed --------------
    cy.get('[data-testid="totp-secret"]')
      .invoke("text")
      .then((secret) => {
        // Kept for the last test's second sign-in, which needs a code from a
        // LATER step: the replay guard is a compare-and-swap that admits only
        // a strictly greater one. In Node, not in `Cypress.env` — see
        // TOTP_SECRET above.
        carryForward(TOTP_SECRET, secret.trim());
        return totpDigits(secret.trim());
      })
      .then((code) => {
        cy.get("#dashboard-totp-code").clear().type(code);
        cy.contains("button", /finish enrolment/i).click();
      });

    // ---- leg 4: the printed password must be replaced --------------------
    //
    // And it must be PRESENTED to be replaced (issue #79 item 3, 2026-09-10).
    // On a first sign-in the password in force is the one `staff add`
    // printed, so that is what goes in the current-password field.
    cy.location("pathname").should("eq", "/login/password");

    // The wrong current password first, in the browser, because this is the
    // case the field exists for: without the check a stolen session cookie
    // was an account takeover in one request. vpay answers the same 401 it
    // answers for every credential refusal, and the page stays put.
    cy.get("#dashboard-current-password").type("not-the-printed-password", {
      log: false,
    });
    cy.get("#dashboard-new-password").type(NEW_PASSWORD, { log: false });
    cy.get("#dashboard-confirm-password").type(NEW_PASSWORD, { log: false });
    cy.contains("button", "Set password").click();
    cy.location("pathname").should("eq", "/login/password");
    cy.get('[role="alert"]').should("be.visible");

    // Then the right one.
    cy.task<string>("staffPassword").then((printed) => {
      cy.get("#dashboard-current-password")
        .clear()
        .type(printed, { log: false });
    });
    cy.get("#dashboard-new-password")
      .clear()
      .type(NEW_PASSWORD, { log: false });
    cy.get("#dashboard-confirm-password")
      .clear()
      .type(NEW_PASSWORD, { log: false });
    cy.contains("button", "Set password").click();

    // ---- leg 5: signed in, on a token the code exchange minted -----------
    cy.location("pathname").should("eq", "/payments");
    cy.contains(staffEmail()).should("be.visible");
  });

  it("keeps the session token httpOnly and the access token out of the browser entirely", () => {
    // Assumes the sign-in above; testIsolation is off for that reason.
    // ADR-0008 and ADR-0017 decision 4, asserted in the browser rather than
    // in a unit test: the session cookie is httpOnly, and the `/dash/v1`
    // bearer token is never in the document at all — it lives in the
    // `staff_sessions` row and is read back server-side on every render.
    cy.getCookie("vpay_dash_session").should((cookie) => {
      expect(cookie, "the session cookie").to.not.equal(null);
      expect(cookie?.httpOnly, "httpOnly").to.equal(true);
      expect(cookie?.sameSite, "sameSite").to.equal("lax");
      // Asserted HERE and not only in `cookies.test.ts`, because this is the
      // attribute the unit test cannot observe: `Secure` is a property of
      // what a BROWSER accepted off the wire, and the constant it is read
      // from could be right while the cookie that reached Chrome was not.
      expect(cookie?.secure, "secure").to.equal(true);
    });

    cy.visit("/payments");
    // No script on this origin can read it, which is what httpOnly means.
    cy.document().then((doc) => {
      expect(doc.cookie, "document.cookie").to.not.contain("vpay_dash_session");
    });
    // And no JWT anywhere in the rendered page.
    cy.get("body")
      .invoke("html")
      .should((html: string) => {
        expect(html, "the rendered page").to.not.match(
          /eyJ[A-Za-z0-9_-]{10,}\./,
        );
      });
  });

  it("shows the sign-in form — not an error page — for a cookie vpay refuses", () => {
    // THE REFUSAL PATH, in a browser, and the one no other case here reaches.
    //
    // Every other test either carries no cookie (`clearCookies`, or after a
    // sign-out that cleared it) or holds a good one. A cookie vpay REFUSES —
    // forged, expired, idle, signed out from another browser, or belonging to
    // an account an operator has just disabled — is one answer, `401`, and
    // `docs/flows/dashboard-auth.md` says the app's reply to all of them is
    // to forget the cookie and show the form.
    //
    // It was not. Measured against the real stack on 2026-09-07: the page
    // tried to clear the cookie during its own render, which Next refuses
    // ("Cookies can only be modified in a Server Action or Route Handler"),
    // so `/payments` answered **500** — and, the cookie still being there,
    // answered 500 on every request after it. `app/signed-out/route.ts` is
    // the fix and this case is what would have caught it.
    //
    // Runs while signed in, and puts the real session back afterwards, so the
    // sequence this spec depends on is undisturbed.
    cy.getCookie("vpay_dash_session").then((good) => {
      const real = good?.value ?? "";
      expect(real, "a session to restore afterwards").to.not.equal("");

      cy.setCookie("vpay_dash_session", "not-a-session-vpay-ever-minted");
      cy.visit("/payments");

      cy.location("pathname").should("eq", "/login");
      cy.contains("h2", "Sign in").should("be.visible");
      // And the dead cookie is gone from this browser, which is what stops
      // the next request being refused all over again.
      cy.getCookie("vpay_dash_session").should((cookie) => {
        expect(cookie?.value ?? "", "the refused cookie").to.equal("");
      });

      cy.setCookie("vpay_dash_session", real, {
        httpOnly: true,
        secure: true,
        sameSite: "lax",
        path: "/",
      });
      cy.visit("/payments");
      cy.contains("h2", "Payments").should("be.visible");
    });
  });

  it("lists this merchant's payments and opens one of them", () => {
    // Still signed in from the sign-in test.
    // A payment of this spec's own, so the assertion does not depend on
    // whichever other spec ran first. `checkout.cy.ts` creates payments for
    // the same tenant and they show up in this list too — this one is here
    // because its id is known.
    cy.task<{ id: string }>("mintCheckoutPaymentIntent").then((minted) => {
      cy.visit("/payments");

      cy.contains("h2", "Payments").should("be.visible");
      cy.get("table tbody tr").should("have.length.at.least", 1);
      cy.screenshot("03-payments", { capture: "viewport" });

      // The row for the payment just created, opened by its own link.
      cy.get(`[data-payment-id="${minted.id}"]`)
        .should("exist")
        .find("a")
        .click();

      cy.location("pathname").should("eq", `/payments/${minted.id}`);
      cy.get('[data-testid="detail-id"]').should("have.text", minted.id);

      // An intent nobody has confirmed has no charge, and the page says that
      // rather than rendering an empty charge block.
      cy.contains("No charge").should("be.visible");

      // And an EMPTY timeline, which this spec asserted the opposite of
      // until it was run. `payment_intent.created` is one of five of the
      // eight documented event types that **nothing writes** (docs/status.md,
      // "Events written by the worker"): only `.succeeded`,
      // `.payment_failed` and `checkout.session.expired` are ever emitted, by
      // settlement and by the housekeeping sweep. So a just-minted intent has
      // no events at all, and the page says "No events yet." rather than
      // rendering a blank section.
      //
      // The populated case is the next test, on a payment that has a charge.
      cy.contains("No events yet.").should("be.visible");
      // `fullPage`, unlike the other three: the detail page is longer than a
      // viewport and the sections a reviewer most needs to see — the charge,
      // the timeline, and the line naming the five event types nothing writes
      // — are all below the fold. A committed screenshot that cannot show the
      // part of the page under discussion is evidence of nothing.
      cy.screenshot("04-payment-detail", { capture: "fullPage" });
    });
  });

  it("takes a payment through the shop, the way the person who reported this did", () => {
    // THE CASE THE MAINTAINER HIT BY HAND, on 2026-09-11, in two halves: this
    // one makes the payment, the next one goes looking for it. They are two
    // tests rather than one because a Cypress test's PRIMARY ORIGIN is fixed
    // by its first `cy.visit` — here the shop's — and every later command on
    // another origin then has to be inside `cy.origin()`. Measured: written
    // as one test, the dashboard half failed with "the command was expected
    // to run against origin http://localhost:3001 but the application is at
    // http://localhost:3000". `testIsolation: false` is what carries the
    // dashboard SESSION into the next test.
    //
    // It does not carry the id, and the first version of this pair said it
    // did. Cookies survive the split; a `Cypress.env` write does not, because
    // the `Cypress` object belongs to the spec bridge of the primary origin
    // this test just moved. The id goes through Node instead
    // (`carryForward`), and the same mistake had also been silently breaking
    // the LAST test in this file, whose TOTP secret was stored before this
    // test runs. Both were `undefined`; neither had been run.
    //
    // Deliberately NOT `cy.task('mintCheckoutPaymentIntent')`. That task
    // holds a merchant private key, and a payment minted with it would prove
    // only that a tenant can read its own rows — which
    // `dashboard_read_surface.rs` already proves. What is under test across
    // these two is the arrangement of the DEMO, so the payment has to arrive
    // the way a person's does: through the shop, on vpay's page, with a
    // phone number, and settled by `vpay-worker` polling a rail.
    buyOnVpaysPage();

    cy.origin(
      checkoutOrigin(),
      { args: { msisdn: MTN.succeeds } },
      ({ msisdn }) => {
        cy.get('[data-screen="select_rail"]', { timeout: 60_000 }).should(
          "be.visible",
        );
        cy.get('button[data-rail="mtn_momo"]').click();
        cy.get('[data-screen="collect_msisdn"]').should("be.visible");
        cy.get("#vpay-msisdn").type(msisdn);
        cy.get('button[type="submit"]').click();
        // `vpay-worker` polling MTN's stub is what moves this; nothing here
        // pushes the status forward.
        cy.get('[data-outcome="succeeded"]', { timeout: 120_000 }).should(
          "be.visible",
        );
        cy.get('[data-outcome="succeeded"] button').click();
      },
    );

    // Back on the shop's return page, which reads the shop's own database —
    // written by the shop's webhook handler after it verified vpay's
    // signature, and by nothing else.
    cy.url({ timeout: 60_000 }).should("include", "/return");
    cy.get('[data-testid="paid-message"]', { timeout: 120_000 }).should(
      "be.visible",
    );

    // The id is read out of the SHOP's own `orders.get`, not out of vpay and
    // not out of anything this spec minted: it is what the merchant stored,
    // so a dashboard that shows it is showing the merchant's own payment and
    // not merely something with the right shape.
    orderIdFromUrl()
      .then((orderId) => readOrder(orderId))
      .then((order) => {
        const intentId = order.paymentIntentId ?? "";
        expect(intentId, "the shop's own paymentIntentId").to.match(/^pi_/);
        // In Node. This test's primary origin is the SHOP's port, the next
        // one's is the dashboard's, and a `Cypress.env` write does not cross
        // that — it was `undefined` one test later, every run. See
        // SHOP_PAYMENT_INTENT_ID at the top of this file.
        carryForward(SHOP_PAYMENT_INTENT_ID, intentId);
      });
  });

  it("shows the demo staff member that payment, in the list and by id", () => {
    // The other half, and the guard on `demo_dashboard_merchant`.
    //
    // A `/dash/v1` request reads exactly one tenant's rows — the one
    // `dashboard_client.merchant_id` names — and the demo stack registers TWO
    // merchant clients on two tenants: `demo-merchant` (`just demo-walk`) and
    // `shop-merchant` (`examples/shop`). The binding named the first, so a
    // payment made through the shop belonged to a tenant this dashboard does
    // not show, and the detail read answered the uniform cross-tenant 404 it
    // answers for an id that exists nowhere. Neither answer was wrong; the
    // binding named the surface nobody clicks in.
    //
    // **Point `demo_dashboard_merchant` back at `demo-merchant-tenant` and
    // both assertions below fail**: the row is absent from the list, and
    // `/payments/{id}` is Next's `notFound()`, which `cy.visit` refuses as a
    // 404. That mutation is this test's whole reason for existing.
    //
    // Still signed in from the sign-in test; the shop and vpay's page set no
    // cookie this spec's dashboard session cares about.
    carriedForward(SHOP_PAYMENT_INTENT_ID).then((intentId) => {
      expect(
        intentId,
        "the payment the previous test made through the shop",
      ).to.match(/^pi_/);

      cy.visit("/payments");
      cy.contains("h2", "Payments").should("be.visible");
      cy.get(`[data-payment-id="${intentId}"]`)
        .should("exist")
        .find("a")
        .click();

      // Not a 404: the by-id read is the second half of what was reported,
      // and it is answered by the same tenant predicate the list is.
      cy.location("pathname").should("eq", `/payments/${intentId}`);
      cy.get('[data-testid="detail-id"]').should("have.text", intentId);
      // And it settled, so this is a payment with a charge behind it rather
      // than an intent nobody ever confirmed.
      cy.get('[data-testid="detail-rail"]').should("have.text", "mtn_momo");
      cy.screenshot("05-shop-payment-on-the-dashboard", {
        capture: "fullPage",
      });

      // THE BY-ID READ ON ITS OWN, not reached through the list.
      //
      // "when I was trying to find them by payment id, I was getting a 404"
      // is half of what was reported, and until this line the only route to
      // it was a row in the list — so a regression that broke the by-id read
      // while leaving the list intact would have failed at
      // `[data-payment-id]` and been read as a list problem, and one that
      // broke both would have been reported as the list alone. A direct
      // visit is the reported symptom itself: `dash::payment_intents::
      // retrieve` answers the uniform cross-tenant 404, the app maps it to
      // Next's `notFound()`, and `cy.visit` fails on a 404 status.
      cy.visit(`/payments/${intentId}`);
      cy.get('[data-testid="detail-id"]').should("have.text", intentId);
    });
  });

  it("renders the masked payer as a dash, because nothing writes that column", () => {
    // `charges.payer_ref_masked` is NULL on every row this deployment has
    // ever written (docs/status.md). The value is rendered FROM the column,
    // so this assertion is about a real null and not about a hard-coded
    // dash: it holds on a payment that HAS a charge.
    //
    // `checkout.cy.ts` runs before this spec (Cypress orders specs
    // alphabetically) and confirms an intent, so the list carries at least
    // one charged payment. The assertion is written so that it is skipped
    // honestly if there is none, rather than passing on an intent with no
    // charge at all — where a dash would prove nothing.
    cy.visit("/payments");
    cy.get("table tbody tr").then(($rows) => {
      const ids = [...$rows].map(
        (row) => row.getAttribute("data-payment-id") ?? "",
      );
      // Walk the list until one has a charge.
      const check = (index: number): void => {
        if (index >= ids.length) {
          throw new Error(
            "dashboard.cy.ts: no payment in this merchant's list has a charge, so the " +
              "masked-payer assertion has nothing to assert against. `checkout.cy.ts` " +
              "is what creates one; if it did not run, this failure is that.",
          );
        }
        cy.visit(`/payments/${ids[index]}`);
        cy.get("body").then(($body) => {
          if ($body.find('[data-testid="detail-payer"]').length === 0) {
            check(index + 1);
            return;
          }
          cy.get('[data-testid="detail-payer"]').should("have.text", "—");
          cy.get('[data-testid="detail-rail"]').should("not.have.text", "");
          // The populated timeline, on the one kind of payment that has one:
          // settlement is what writes an event, so a charged payment has at
          // least the type that terminated it.
          cy.get("main").should("contain.text", "payment_intent.");
        });
      };
      check(0);
    });
  });

  // ------------------------------------------------------------------ BFF ---
  //
  // THE CASES `bff.test.ts` CANNOT HAVE, and the reason this block exists.
  // Every case in that suite is a synthetic `Request` against a stubbed
  // `fetch`, so the surface's whole CSRF control — "the browser sent
  // `Sec-Fetch-Site: same-origin`" — is a header the unit suite writes itself
  // and then reads back. These run in Chrome against the real stack, where
  // the header is the browser's and nothing here can spell it.
  //
  // `cy.intercept` is used as a **spy and never as a stub**: no response is
  // faked and no request is rewritten — every one of them reaches the
  // dashboard and vpay. `cypress/support/e2e.ts` says request stubbing does
  // not live in these specs and that still holds. What an interception buys
  // is the one thing `cy.window().fetch` cannot report: the headers the
  // browser put on the wire, and the status the server answered when the page
  // itself is not allowed to read it.
  //
  // Still signed in from the sign-in test; `testIsolation` is off.

  it("serves the signed-in staff member's own browser fetch, on the browser's own headers", () => {
    // THE CASE THAT ANSWERS THE OPEN QUESTION. `docs/status.md` has said of
    // this surface since it was built that "a real browser's `Sec-Fetch-Site`
    // and cookie arrive as this code expects" is NOT proven. This is what
    // proves it — or, if Chrome disagrees with `bff.ts`'s reading of the
    // Fetch spec, what says so.
    cy.intercept("GET", "**/api/dash/payment_intents*").as("bffList");

    cy.visit("/payments");
    cy.contains("h2", "Payments").should("be.visible");

    cy.window()
      .then((win) =>
        win
          .fetch("/api/dash/payment_intents", { credentials: "same-origin" })
          .then((response) =>
            response.text().then((body) => ({ status: response.status, body })),
          ),
      )
      .then((answer) => {
        expect(answer.status, "the BFF's own answer to its own page").to.equal(
          200,
        );
        const page = JSON.parse(answer.body) as Record<string, unknown>;
        expect(page["object"]).to.equal("list");
        expect(page["data"], "the rows").to.be.an("array");
        expect(page, "the envelope this surface promises").to.have.property(
          "has_more",
        );
        expect(page).to.have.property("cursor");
        // The token check in a browser rather than against a serialised
        // `NextResponse`: a `/dash/v1` bearer is a JWT, and there is none in
        // what reached this page.
        expect(
          answer.body,
          "the body a browser actually received",
        ).to.not.match(/eyJ[A-Za-z0-9_-]{10,}\./);
      });

    cy.wait("@bffList").then((interception) => {
      const headers = interception.request.headers;
      // WHAT THE BROWSER ACTUALLY SENT. Each line is an assumption written
      // down in `src/server/bff.ts` and, until this spec, believed rather
      // than seen. Measured under Chrome 152 and under Cypress's bundled
      // Electron 138, identical in both.
      expect(
        headers["sec-fetch-site"],
        "the header apiIsSameOrigin requires, as the BROWSER set it",
      ).to.equal("same-origin");
      expect(
        headers["origin"] ?? null,
        "and NO Origin at all — `apiIsSameOrigin`'s whole reason for reading " +
          "Sec-Fetch-Site is that a browser omits Origin on a same-origin " +
          "GET. If this is ever not null, that sentence has stopped being " +
          "true and the rule can be tightened",
      ).to.equal(null);
      expect(headers["sec-fetch-mode"]).to.equal("cors");
      expect(headers["sec-fetch-dest"]).to.equal("empty");
      expect(
        String(headers["cookie"] ?? ""),
        "the httpOnly session cookie, which no script on the page can read " +
          "and the browser attaches anyway",
      ).to.contain("vpay_dash_session");
      expect(interception.response?.statusCode).to.equal(200);
      // Nothing cached it, which matters more here than on a page: this is a
      // merchant's payment list on a shared origin.
      expect(interception.response?.headers["cache-control"]).to.equal(
        "no-store",
      );
    });
  });

  it("refuses the same browser fetch the moment the session cookie is gone", () => {
    // The cookie half, with the page already loaded so the fetch is the only
    // thing that changes. Restores the session afterwards, like the
    // refused-cookie test above, because the sequence below depends on it.
    cy.intercept("GET", "**/api/dash/payment_intents*").as("bffNoSession");

    cy.visit("/payments");
    cy.contains("h2", "Payments").should("be.visible");

    cy.getCookie("vpay_dash_session").then((good) => {
      const real = good?.value ?? "";
      expect(real, "a session to restore afterwards").to.not.equal("");

      cy.clearCookie("vpay_dash_session");
      cy.window()
        .then((win) =>
          win
            .fetch("/api/dash/payment_intents", { credentials: "same-origin" })
            .then((response) =>
              response
                .text()
                .then((body) => ({ status: response.status, body })),
            ),
        )
        .then((answer) => {
          expect(answer.status, "no session, no read").to.equal(401);
          // The sentence, not just the status: a `401` that said something
          // about vpay's internals would be the finding the exp55 review
          // fixed, coming back.
          expect(answer.body).to.contain("carried no staff session");
        });

      cy.wait("@bffNoSession").then((interception) => {
        expect(
          String(interception.request.headers["cookie"] ?? ""),
          "the browser must have sent no session cookie",
        ).to.not.contain("vpay_dash_session");
        expect(interception.response?.statusCode).to.equal(401);
      });

      cy.setCookie("vpay_dash_session", real, {
        httpOnly: true,
        secure: true,
        sameSite: "lax",
        path: "/",
      });
      cy.visit("/payments");
      cy.contains("h2", "Payments").should("be.visible");
    });
  });

  it("refuses a cross-origin fetch from another site's page, cookie and all", () => {
    // THE CSRF CONTROL, FIRING IN A BROWSER — a genuine cross-origin request
    // rather than a same-origin one with a forged header, which would prove
    // nothing this spec could not also have made up.
    //
    // The shop is a different ORIGIN (`localhost:3001` against the
    // dashboard's `localhost:3000`) and the same SITE, because a site is
    // scheme plus registrable domain and **a port is not part of it**. That
    // is not a weakness of the test, it is the sharper half of it: a
    // same-site request is one a `SameSite=Lax` cookie is still attached to,
    // so what arrives at the dashboard below CARRIES THE SESSION and is
    // refused on the origin rule alone.
    //
    // What this case does NOT isolate: which half of `apiIsSameOrigin`
    // refused it. A cross-origin `fetch` carries an `Origin`, so
    // `originIsAllowed` refuses before `Sec-Fetch-Site` is consulted. The
    // next case is the one with no `Origin` on it.
    cy.intercept("GET", "**/api/dash/payment_intents*").as("bffCrossOrigin");

    const target = `${Cypress.config("baseUrl")}/api/dash/payment_intents`;
    cy.visit(shopUrl());

    cy.window()
      .then((win) =>
        win
          .fetch(target, { credentials: "include" })
          .then((response) => `read it: ${response.status}`)
          .catch(() => "the browser would not let this page read the answer"),
      )
      .then((outcome) => {
        // The BFF sends no `Access-Control-Allow-Origin`, so the page cannot
        // see the answer at all — a second, independent refusal, and worth
        // asserting rather than working around. The SERVER's answer is read
        // below, off the wire, where a page's own permissions do not apply.
        expect(outcome).to.equal(
          "the browser would not let this page read the answer",
        );
      });

    cy.wait("@bffCrossOrigin").then((interception) => {
      const headers = interception.request.headers;
      expect(
        headers["origin"],
        "the attacking page's origin, as the browser set it",
      ).to.equal(shopUrl());
      expect(
        headers["sec-fetch-site"],
        "a port is not part of a site, so Chrome calls this same-site",
      ).to.equal("same-site");
      expect(
        String(headers["cookie"] ?? ""),
        "and SameSite=Lax attaches the session to a SAME-SITE request — which " +
          "is exactly why this surface cannot rest on the cookie policy alone",
      ).to.contain("vpay_dash_session");
      expect(
        interception.response?.statusCode,
        "403 and not 401: the origin rule refused it with a live session in hand",
      ).to.equal(403);
    });
  });

  it("refuses a cross-origin FRAME of the same URL, which carries no Origin at all", () => {
    // THE CASE THAT ISOLATES `Sec-Fetch-Site`, and the only one here that can.
    //
    // `bff.ts` requires that header because **a browser sends no `Origin` on
    // a same-origin `GET`** and script cannot add one, so `Origin` alone
    // cannot answer the question on this method. The corollary is the hole
    // the header closes: a cross-origin request that is a NAVIGATION rather
    // than a `fetch` carries no `Origin` either, so `originIsAllowed` has
    // nothing to refuse and passes. An `<iframe src="…">` is such a
    // navigation, and it is a thing any page on the internet can do.
    //
    // If this answered anything but `403`, the surface would have a hole the
    // unit suite — which writes the header it then reads — could not see.
    cy.intercept("GET", "**/api/dash/payment_intents*").as("bffFramed");

    const target = `${Cypress.config("baseUrl")}/api/dash/payment_intents`;
    cy.visit(shopUrl());

    cy.window().then((win) => {
      const frame = win.document.createElement("iframe");
      frame.src = target;
      win.document.body.appendChild(frame);
    });

    cy.wait("@bffFramed").then((interception) => {
      const headers = interception.request.headers;
      // MEASURED, Chrome 152 and Electron 138 alike. Every line of this is
      // the attack, written out as the browser sent it.
      expect(
        headers["sec-fetch-mode"],
        "a frame load is a navigation, not a fetch",
      ).to.equal("navigate");
      expect(headers["sec-fetch-dest"]).to.equal("iframe");
      expect(
        headers["origin"] ?? null,
        "and a navigation carries NO Origin — so `originIsAllowed` has " +
          "nothing to refuse and passes this request through",
      ).to.equal(null);
      expect(
        headers["sec-fetch-site"],
        "leaving this as the only thing that says where it came from",
      ).to.equal("same-site");
      expect(
        String(headers["cookie"] ?? ""),
        "AND THE SESSION TRAVELS WITH IT. `SameSite=Lax` does not stop a " +
          "same-site request, so a bare <iframe> on any page sharing this " +
          "registrable domain reaches this surface holding a signed-in " +
          "staff member's cookie. This is the request Sec-Fetch-Site exists " +
          "for, and it is not hypothetical",
      ).to.contain("vpay_dash_session");
      expect(
        interception.response?.statusCode,
        "403 — and Sec-Fetch-Site is the ONLY check that can have produced " +
          "it, because there was no Origin and there was a good cookie",
      ).to.equal(403);
    });
  });

  it("answers a browser's OPTIONS with 405 and no Allow header", () => {
    // The exp56 middleware, in a browser rather than against Next's own
    // helper. `middleware.test.ts` measures that Next would auto-implement
    // `OPTIONS` as a `204` carrying `Allow: GET, HEAD, OPTIONS`, answered
    // before the origin check and before the cookie is read; this is the
    // same URL asked the same question by Chrome, through the real Next
    // server, with the middleware in front of it.
    cy.intercept("OPTIONS", "**/api/dash/payment_intents*").as("bffOptions");

    cy.visit("/payments");
    cy.contains("h2", "Payments").should("be.visible");

    cy.window()
      .then((win) =>
        win
          .fetch("/api/dash/payment_intents", {
            method: "OPTIONS",
            credentials: "same-origin",
          })
          .then((response) => ({
            status: response.status,
            allow: response.headers.get("allow"),
          })),
      )
      .then((answer) => {
        expect(answer.status, "not Next's 204").to.equal(405);
        expect(
          answer.allow,
          "and no method list, which is the half a 405 could still have leaked",
        ).to.equal(null);
      });

    cy.wait("@bffOptions").then((interception) => {
      // Measured, and worth writing down beside the list case above: this
      // request DOES carry an `Origin`, on the same origin, because a
      // browser attaches one to every request whose method is not `GET` or
      // `HEAD`. That asymmetry is precisely why `Sec-Fetch-Site` has to be
      // read for the reads — `Origin` is present exactly on the methods this
      // surface does not serve.
      expect(interception.request.headers["origin"]).to.equal(
        Cypress.config("baseUrl"),
      );
      expect(interception.response?.statusCode).to.equal(405);
      expect(interception.response?.headers["allow"] ?? null).to.equal(null);
    });
  });

  it("replaces the access token before it expires, so a long session never sees a 401", () => {
    // ISSUE #88 ITEM 1, and the case the exp28 review's finding F4 could not
    // have. The `/dash/v1` token's TTL is
    // `staff_auth.access_token_ttl_seconds`; a session's bounds are twelve
    // hours and thirty minutes idle. Until 2026-09-10 `requireStaff` ran the
    // authorization-code leg only when the row carried NO token, so once one
    // was written it was used until sign-out and every render past its expiry
    // was an error box.
    //
    // At the shipping 900 s no browser run could reach that. This stack sets
    // thirty (`demo_staff_token_ttl`), so the margin — 20 % of the TTL — falls
    // at twenty-four seconds and one leg crosses both it and the expiry.
    //
    // **What makes this decisive.** The reactive re-mint in `dash-read.ts`
    // still exists, so a page renders correctly whether the token was replaced
    // early or replaced after a read failed on it. The two are told apart by
    // `access_token_expires_at`, read from vpay itself: the assertion is that
    // the expiry MOVED while the old one had not yet passed. Drop the margin
    // from `gateFor` and the re-mint happens AT the expiry instead of before
    // it — measured, and the second assertion below is the one that failed.
    //
    // **The waits are computed from the expiry vpay reported, not counted from
    // here**, and that is not tidiness. A fixed `cy.wait` pays for the visit
    // and the task that precede it out of the same window: at a twenty-second
    // TTL that window is four seconds wide and the first version of this leg
    // landed 300 ms inside it, which is a flake waiting for a slower machine.
    // Targeting an instant makes the slack a stated number — 4.5 seconds.
    const ttlSeconds = 30;
    /** Where in the token's life the second render should land: 85 % gone. */
    const renderAtFraction = 0.85;

    cy.visit("/payments");
    cy.contains("h2", "Payments").should("be.visible");

    cy.getCookie("vpay_dash_session").then((cookie) => {
      const session = cookie?.value ?? "";
      expect(session, "a signed-in session").to.not.equal("");

      cy.task<string | null>("staffTokenExpiry", session).then((first) => {
        expect(
          first,
          "the session row must record when its token expires",
        ).to.be.a("string");
        const firstExpiry = Date.parse(String(first));

        // Past the 80 % margin and comfortably short of the expiry.
        const target = firstExpiry - ttlSeconds * (1 - renderAtFraction) * 1000;
        cy.wrap(null).then(() => {
          cy.wait(Math.max(target - Date.now(), 0));
        });
        cy.visit("/payments");
        cy.contains("h2", "Payments").should("be.visible");

        cy.task<string | null>("staffTokenExpiry", session).then((second) => {
          const secondExpiry = Date.parse(String(second));
          expect(
            secondExpiry,
            "the render past the margin must have minted a NEW token; the same expiry back " +
              "means nothing re-minted and the fifteen minutes are still there",
          ).to.be.greaterThan(firstExpiry);
          expect(
            Date.now(),
            "and it must have done so BEFORE the old token expired — that is the whole " +
              "difference between this and the reactive retry in dash-read.ts",
          ).to.be.lessThan(firstExpiry);
        });

        // And now past the ORIGINAL token's expiry entirely: the render that
        // used to be an error box. No sign-in happens in between.
        cy.wrap(null).then(() => {
          cy.wait(Math.max(firstExpiry + 2000 - Date.now(), 0));
        });
        cy.visit("/payments");
        cy.location("pathname").should("eq", "/payments");
        cy.contains("h2", "Payments").should("be.visible");
        cy.contains(staffEmail()).should("be.visible");
        cy.get("table").should("exist");
      });
    });
  });

  it("refuses everything again after signing out", () => {
    // The last test that runs while still signed in.
    cy.visit("/payments");
    cy.contains("button", "Sign out").click();

    // Back at the form, and the cookie is gone from this browser.
    cy.location("pathname").should("eq", "/login");
    cy.getCookie("vpay_dash_session").should((cookie) => {
      expect(cookie?.value ?? "", "the session cookie value").to.equal("");
    });

    // And the row is gone from vpay, which is the part that matters: signing
    // out DELETES `staff_sessions`, taking the access token with it. A
    // cleared cookie alone would leave a live token obtainable by anyone who
    // still had the session string.
    cy.visit("/payments");
    cy.location("pathname").should("eq", "/login");
  });

  it("signs the same staff member back in with the password they set", () => {
    // Proves leg 4 actually changed the credential — a password change that
    // silently did nothing would leave every assertion above green — and
    // that enrolment is not repeated: this sign-in gets the plain code
    // screen, with no QR.
    waitForNextTotpStep();

    cy.visit("/login");
    cy.get("#dashboard-signin-email").type(staffEmail());
    cy.get("#dashboard-signin-password").type(NEW_PASSWORD, { log: false });
    cy.contains("button", "Sign in").click();

    cy.location("pathname").should("eq", "/login/totp");
    cy.get('[data-testid="totp-qr"]').should("not.exist");
    cy.contains("h2", "Enter your code").should("be.visible");

    // The secret the enrolment screen showed, back from Node: the shop leg
    // above moved this spec's primary origin, and `Cypress.env` did not
    // survive it — this read was `undefined` and `cy.task('totpCode')` died
    // on it, every run, until 2026-09-11.
    carriedForward(TOTP_SECRET)
      .then((secret) => totpDigits(secret))
      .then((code) => {
        cy.get("#dashboard-totp-code").type(code);
        cy.contains("button", "Continue").click();
      });

    // Straight to the payments list: the one-time password is behind them
    // now, so `/login/password` is not on the way any more.
    cy.location("pathname").should("eq", "/payments");
  });
});
