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
 * # And one leg that is about a token rather than about a person
 *
 * "replaces the access token before it expires" is issue #88 item 1, and it is
 * here rather than in a unit test because the thing it proves is a **sequence
 * of renders across an expiry**. It depends on this stack's short
 * `staff_auth.access_token_ttl_seconds` (`demo_staff_token_ttl`, twenty
 * seconds) — at the shipping 900 no browser run could reach the case at all,
 * which is why nothing ever had.
 */
import { totpDigits, waitForNextTotpStep } from "../support/dashboard";

/** The staff address `just demo-staff` created. */
const staffEmail = (): string => Cypress.expose("STAFF_EMAIL") as string;

/** A password this spec sets, replacing the printed one-time password. */
const NEW_PASSWORD = "a-long-enough-demo-password";

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
      expect(cookie?.value ?? "", "the session cookie after a wrong code").to.not.equal("");
    });

    // ---- leg 3: a code computed from what the screen showed --------------
    cy.get('[data-testid="totp-secret"]')
      .invoke("text")
      .then((secret) => {
        // Kept for leg 5's second sign-in, which needs a code from a LATER
        // step: the replay guard is a compare-and-swap that admits only a
        // strictly greater one.
        Cypress.env("dashboardTotpSecret", secret.trim());
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
    cy.get("#dashboard-current-password").type("not-the-printed-password", { log: false });
    cy.get("#dashboard-new-password").type(NEW_PASSWORD, { log: false });
    cy.get("#dashboard-confirm-password").type(NEW_PASSWORD, { log: false });
    cy.contains("button", "Set password").click();
    cy.location("pathname").should("eq", "/login/password");
    cy.get('[role="alert"]').should("be.visible");

    // Then the right one.
    cy.task<string>("staffPassword").then((printed) => {
      cy.get("#dashboard-current-password").clear().type(printed, { log: false });
    });
    cy.get("#dashboard-new-password").clear().type(NEW_PASSWORD, { log: false });
    cy.get("#dashboard-confirm-password").clear().type(NEW_PASSWORD, { log: false });
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
        expect(html, "the rendered page").to.not.match(/eyJ[A-Za-z0-9_-]{10,}\./);
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
      const ids = [...$rows].map((row) => row.getAttribute("data-payment-id") ?? "");
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
    // twenty (`demo_staff_token_ttl`), so the margin — 20 % of the TTL — falls
    // at sixteen seconds and one leg crosses both it and the expiry.
    //
    // **What makes this decisive.** The reactive re-mint in `dash-read.ts`
    // still exists, so a page renders correctly whether the token was replaced
    // early or replaced after a read failed on it. The two are told apart by
    // `access_token_expires_at`, read from vpay itself: the assertion is that
    // the expiry MOVED while the old one had not yet passed. Drop the margin
    // from `gateFor` and the second read answers the same expiry as the first.
    const ttlSeconds = 20;
    const marginSeconds = ttlSeconds * 0.2;

    cy.visit("/payments");
    cy.contains("h2", "Payments").should("be.visible");

    cy.getCookie("vpay_dash_session").then((cookie) => {
      const session = cookie?.value ?? "";
      expect(session, "a signed-in session").to.not.equal("");

      cy.task<string | null>("staffTokenExpiry", session).then((first) => {
        expect(first, "the session row must record when its token expires").to.be.a("string");
        const firstExpiry = Date.parse(String(first));

        // Past the margin, and deliberately NOT past the expiry.
        cy.wait((ttlSeconds - marginSeconds + 1) * 1000);
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
        cy.wait((ttlSeconds - (ttlSeconds - marginSeconds) + 2) * 1000);
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

    totpDigits(Cypress.env("dashboardTotpSecret") as string).then((code) => {
      cy.get("#dashboard-totp-code").type(code);
      cy.contains("button", "Continue").click();
    });

    // Straight to the payments list: the one-time password is behind them
    // now, so `/login/password` is not on the way any more.
    cy.location("pathname").should("eq", "/payments");
  });
});
