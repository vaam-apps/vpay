/**
 * The staff dashboard's sign-in, end to end, against the real stack from
 * `compose.e2e.yml` + `compose.demo.yml`: real Next.js, real `vpay-server`,
 * real Postgres, real argon2id, real RFC 6238, real RS256 — with WireMock
 * standing in for the payment rails and nothing standing in for the sign-in.
 * This is VPA-5c: VPA-5a built the sign-in and VPA-5b the single sign-on
 * button, each proved by jsdom and a fake `fetch`, and neither had ever met a
 * running vpay.
 *
 * **Nothing about the sign-in is stubbed.** The password is the one
 * `vpay-server staff add` printed into a file; the TOTP codes are computed from
 * the secret THE ENROLMENT SCREEN DISPLAYED, so what this proves is that an
 * authenticator app enrolled from that QR would work — not that a secret the
 * test also knows verifies against itself. Every refusal asserted below is
 * vpay's own.
 *
 * # What it needs, and what it does not
 *
 * The stack must serve `human_login`, which the `demo` overlay does since
 * VPA-5c (`just gen-demo-keys`), and `just demo-staff` must have created the
 * member and kept their one-time password (`just test-e2e` and CI's
 * `e2e (compose)` job both do). Against a stack that serves no `human_login`
 * the first case fails, by design: "paired key only" is exactly what it asserts
 * is NOT there.
 *
 * **The browser reaches the dashboard on `localhost`, and has to.** The session
 * cookies are `__Host-` prefixed and `Secure`, and a browser stores a `Secure`
 * cookie from `http://` only on a secure context. `localhost` is one by rule;
 * `http://dashboard:3000` (the compose network's name) or any other host is not,
 * and there the sign-in would succeed, redirect and arrive at `/login` again
 * with no cookie. Cypress runs on the host and the dashboard is published on
 * `localhost:3000`, so `baseUrl` is the only origin this spec uses
 * (`docs/flows/dashboard-auth/human-login.md`, "The cookie needs a secure
 * origin"). **That a Cypress-driven Chromium stores the cookie is the premise of
 * this whole file and was written down before it was observed**; the first
 * `e2e (compose)` run is the first time it is tested.
 *
 * # The cases, and where each one is
 *
 * | VPA-5c case | Test                                                            |
 * | ----------- | --------------------------------------------------------------- |
 * | discovery   | "serves the sign-in, and does not say 'paired key only'"        |
 * | 3           | "refuses a wrong password with one sentence, …"                 |
 * | 18          | "signs in with the one-time password and enrols …"              |
 * | 19          | "makes the person replace the one-time password …"              |
 * | 1           | "signs the same person back in …" (a plain sign-in, no enrolment)|
 * | 4           | "revokes the session on sign-out …"                             |
 *
 * **Not here: single sign-on.** It needs an OpenID Connect provider, and this
 * stack has none (no Keycloak, no fake issuer — vpay's own SSO suites run theirs
 * in a testcontainer, out of this stack's reach). The overlay writes no
 * `staff_auth.sso` and the dashboard container gets no
 * `VPAY_DASHBOARD_SSO_ISSUER`, so `/login` draws no button. Building a provider
 * to fill that gap was ruled out of this ticket; the gap is named in
 * `docs/flows/dashboard-auth/sso.md`, "Not built".
 *
 * # Why this spec turns test isolation off
 *
 * Cypress clears cookies before every test by default, and this spec is a
 * **sequence**: sign in once (enrolling and replacing the printed password on
 * the way), look at the account, sign out, sign back in. With isolation on, the
 * tests after the first sign-in all start signed out. The alternative is a
 * `beforeEach` that signs in again, and it is worse than it looks: vpay's TOTP
 * replay guard is a compare-and-swap on `staff_members.last_totp_step`
 * admitting only a strictly greater step, so every sign-in needs its own
 * 30-second window.
 *
 * The order below is therefore load-bearing, and each test says what it assumes
 * about the one before it. **A failure early in the sequence fails the tests
 * after it**, for the reason that they have nobody to be signed in as; read the
 * first red one.
 */
import {
  carriedForward,
  carryForward,
  expectNoWcagViolations,
  totpDigits,
  waitForNextTotpStep,
} from "../support/dashboard";

/** The staff address `just demo-staff` created. */
const staffEmail = (): string => Cypress.expose("STAFF_EMAIL") as string;

/** vpay as the host reaches it (`cypress.config.ts`). */
const vpay = (): string => Cypress.expose("VPAY_BASE_URL") as string;

/**
 * The session cookie, by the name `src/server/cookies.ts` sets it under: the
 * `__Host-` prefix is a browser-enforced promise (`Secure`, `Path=/`, no
 * `Domain`), so a cookie that is here under this name was accepted on those
 * terms.
 */
const SESSION_COOKIE = "__Host-vpay_dash_session";

/**
 * What this spec sets as the member's own password, replacing the printed one
 * — generated per run, so no credential is written down in this file and the
 * value means nothing the moment the stack is torn down (`down -v`). Twelve
 * characters is vpay's minimum; this is 41.
 */
const NEW_PASSWORD = `e2e-${crypto.randomUUID()}`;

/** A password that is nobody's, for the refusals. */
const WRONG_PASSWORD = `wrong-${crypto.randomUUID()}`;

/** The base32 secret the enrolment screen showed, carried to the last test. */
const TOTP_SECRET = "dashboardTotpSecret";

/** An address nobody was ever given an account at. */
const NOBODY = `nobody-${crypto.randomUUID().slice(0, 8)}@example.test`;

/**
 * Answers the sign-in's two questions, one screen each, and submits.
 *
 * Two screens and a Continue between them, not a form: `OneQuestionSteps`
 * collects one piece of information per screen. The password is typed with
 * `log: false` so it is not in the command log or a screenshot.
 */
function signIn(email: string, password: string): void {
  cy.get("#dashboard-step-email").clear().type(email);
  cy.contains("button", "Continue").click();
  cy.get("#dashboard-step-password").clear().type(password, { log: false });
  cy.contains("button", "Sign in").click();
}

/**
 * Answers the password change's three questions and submits. Safe to call
 * again after a refusal: the stepper returns to the question the refusal
 * names, keeps the other answers, and every field is cleared before it is
 * typed into.
 */
function changePassword(current: string, next: string): void {
  cy.get("#dashboard-step-current_password")
    .clear()
    .type(current, { log: false });
  cy.contains("button", "Continue").click();
  cy.get("#dashboard-step-new_password").clear().type(next, { log: false });
  cy.contains("button", "Continue").click();
  cy.get("#dashboard-step-confirm_password").clear().type(next, { log: false });
  cy.contains("button", "Set password").click();
}

/**
 * The sentence in the alert, without the request id vpay attaches to it —
 * which differs on every request and so would make two identical refusals
 * compare unequal.
 */
function refusalSentence(): Cypress.Chainable<string> {
  return cy
    .get('[role="alert"]')
    .should("be.visible")
    .invoke("text")
    .then((text) => text.replace(/Request\s+\S+\s*$/, "").trim());
}

describe(
  "the dashboard's staff sign-in (human_login)",
  { testIsolation: false },
  () => {
    it("serves the sign-in, and does not say 'paired key only'", () => {
      // Runs first, in a fresh browser: nothing has signed in yet.
      cy.clearCookies();

      // What vpay itself says it serves, asked directly. The page's reading of
      // this document decides which `/login` it draws, so asserting the document
      // first means a failure below names the right side.
      cy.request({
        url: `${vpay()}/.well-known/vpay-auth`,
        log: false,
      }).then((response) => {
        expect(response.status, "discovery status").to.equal(200);
        expect(
          (response.body as { methods?: string[] }).methods,
          "the methods this deployment serves",
        ).to.include("human_login");
      });

      cy.visit("/login");
      // The first question, and not the paired-key notice, and not the
      // "no sign-in is configured" one that names the variables to set.
      cy.get("#dashboard-step-email").should("be.visible");
      cy.contains("This console signs in with a paired key").should(
        "not.exist",
      );
      cy.contains("No sign-in is configured").should("not.exist");
      cy.contains("Sign-in is not reachable").should("not.exist");
      expectNoWcagViolations("/login, the email question");

      // A signed-out visitor is sent here from the account page and from the
      // code screen, and not around in a circle.
      cy.visit("/");
      cy.location("pathname").should("eq", "/login");
      cy.visit("/login/totp");
      cy.location("pathname").should("eq", "/login");
    });

    it("refuses a wrong password with one sentence, the same for an address nobody has (case 3)", () => {
      // Account enumeration is the property: an address with no account and a
      // real address with a wrong password must be indistinguishable on the
      // screen. vpay answers both with `StaffSignInRefused`, one 401, one
      // sentence; this is that, through the dashboard, in a browser.
      cy.visit("/login");
      signIn(NOBODY, WRONG_PASSWORD);
      cy.location("pathname").should("eq", "/login");
      refusalSentence().then((forNobody) => {
        expect(
          forNobody,
          "a refusal that says something",
        ).to.have.length.greaterThan(10);
        expect(forNobody, "the sentence echoes no address").to.not.contain(
          NOBODY,
        );
        cy.getCookie(SESSION_COOKIE).should("not.exist");

        // The stepper put us back on the password question. Back to the email,
        // then a REAL address with the same wrong password.
        cy.contains("button", "Back").click();
        signIn(staffEmail(), WRONG_PASSWORD);
        cy.location("pathname").should("eq", "/login");
        refusalSentence().then((forTheMember) => {
          expect(
            forTheMember,
            "the sentence for a real address with a wrong password",
          ).to.equal(forNobody);
          expect(forTheMember).to.not.contain(staffEmail());
        });
      });
      // And the refusal stored nothing: no session, and no half-session that
      // `/login/totp` would accept.
      cy.getCookie(SESSION_COOKIE).should("not.exist");
      cy.visit("/login/totp");
      cy.location("pathname").should("eq", "/login");
    });

    it("signs in with the one-time password and enrols an authenticator (case 18)", () => {
      cy.visit("/login");
      cy.task<string>("staffPassword").then((password) => {
        signIn(staffEmail(), password);
      });

      // A first sign-in is owed an enrolment: the session is `pending_totp` and
      // does not reach `authenticated` while the member has no secret.
      cy.location("pathname").should("eq", "/login/totp");
      cy.get('[data-testid="totp-qr"]')
        .should("be.visible")
        .and("have.attr", "src")
        .and("match", /^data:image\/png;base64,/);
      expectNoWcagViolations("/login/totp, the enrolment");

      // A MISTYPED code must not sign anybody out, and must keep the enrolment.
      // `POST /dash/v1/staff/totp` answers the same 401 for a wrong code as for a
      // session it refuses; reading every one as "the session is over" cleared
      // the cookie and the sealed secret, so a typo sent the person back to the
      // password and a first sign-in could not even be retried (the exp36
      // review's F6). Three assertions, and the QR is the one that would be
      // missed: the page has to still be the ENROLMENT page, not merely still be
      // at this path.
      cy.get("#dashboard-step-code").type("000000");
      cy.contains("button", /finish enrolment/i).click();
      cy.location("pathname").should("eq", "/login/totp");
      cy.get('[role="alert"]').should("be.visible");
      cy.get('[data-testid="totp-qr"]').should("be.visible");
      cy.getCookie(SESSION_COOKIE).should("exist");

      // A code computed from what the screen showed. The typed key is in a
      // bottom sheet (every choice is one, and there is no dropdown); it is read
      // from the sheet and the sheet is closed before the code goes in.
      cy.contains("button", "Cannot scan? Enter the key instead").click();
      cy.get('[data-testid="totp-secret"]')
        .should("be.visible")
        .invoke("text")
        .then((shown) => {
          // "Spaces do not matter", the sheet says, and an authenticator ignores
          // them; the RFC 4648 decoder here does not.
          const secret = shown.replace(/\s+/g, "");
          // In Node, not in `Cypress.env` — see `dashboardTasks.ts`.
          carryForward(TOTP_SECRET, secret);
          return totpDigits(secret);
        })
        .then((code) => {
          cy.contains("button", "Done").click();
          cy.get("#dashboard-step-code").clear().type(code);
          cy.contains("button", /finish enrolment/i).click();
        });
    });

    it("makes the person replace the one-time password before anything else (case 19)", () => {
      // `staff add` sets `must_change`, and `/oauth/authorize` refuses a session
      // that still carries it, so until this is done no `/dash/v1` token can
      // exist. Assumes the enrolment above completed: the code was accepted and
      // the gate sent the session here.
      cy.location("pathname", { timeout: 20_000 }).should(
        "eq",
        "/login/password",
      );
      cy.contains("Replace it before going any further").should("be.visible");
      expectNoWcagViolations("/login/password");

      // The wrong CURRENT password first, in the browser, because the field
      // exists for this case: without it a stolen session cookie was an account
      // takeover in one request. vpay refuses with its usual 401, the page stays
      // and the session survives (a refusal here must not be read as the end of
      // the session, `actions.ts`'s `changePassword`).
      changePassword(WRONG_PASSWORD, NEW_PASSWORD);
      cy.location("pathname").should("eq", "/login/password");
      cy.get('[role="alert"]').should("be.visible");
      cy.getCookie(SESSION_COOKIE).should("exist");

      // Then the one `staff add` printed.
      cy.task<string>("staffPassword").then((printed) => {
        changePassword(printed, NEW_PASSWORD);
      });

      // The account page: signed in on a token the authorization-code grant
      // minted (PKCE, an exact redirect URI, a checked state), whose exchange this
      // app's own server performed. Nothing else on this screen needs a token, so
      // that the page rendered AT ALL, and not "Signed in, but not authorised", is
      // what proves the grant.
      cy.location("pathname", { timeout: 20_000 }).should("eq", "/");
      cy.contains("h2", /^Signed in as /).should("be.visible");
      cy.contains(staffEmail()).should("be.visible");
      cy.contains("Signed in, but not authorised").should("not.exist");
      cy.contains("button", "Sign out").should("be.visible");
      expectNoWcagViolations("/, the account page");
    });

    it("keeps the session cookie httpOnly and the access token out of the browser entirely", () => {
      // Assumes the sign-in above. ADR-0017 decision 4, asserted in a browser
      // rather than a unit test: the session cookie is httpOnly, and the
      // `/dash/v1` bearer is never in the document at all — it lives in the
      // `staff_sessions` row and is read back server-side on every render.
      cy.getCookie(SESSION_COOKIE).should((cookie) => {
        expect(cookie, "the session cookie").to.not.equal(null);
        expect(cookie?.httpOnly, "httpOnly").to.equal(true);
        expect(cookie?.sameSite, "sameSite").to.equal("lax");
        // Asserted HERE and not only in `cookies.test.ts`, because this is the
        // attribute a unit test cannot observe: `Secure` is a property of what a
        // BROWSER accepted off the wire, and `__Host-` is the browser refusing
        // the cookie unless it is. Both being true means `localhost` was treated
        // as the secure context this file's header says it has to be.
        expect(cookie?.secure, "secure").to.equal(true);
        expect(cookie?.path, "path").to.equal("/");
      });
      cy.document().then((doc) => {
        expect(doc.cookie, "document.cookie").to.not.contain(
          "vpay_dash_session",
        );
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

    it("revokes the session on sign-out, not just the cookie (case 4)", () => {
      // Signing out DELETES the `staff_sessions` row, taking the access token
      // with it. A cleared cookie alone would leave a live session obtainable by
      // anyone who still held the string, and this asks vpay directly: the same
      // token that was alive a moment ago must now be refused.
      cy.getCookie(SESSION_COOKIE).then((cookie) => {
        const token = cookie?.value ?? "";
        expect(token, "a session to revoke").to.not.equal("");
        const ask = () =>
          cy.request({
            url: `${vpay()}/dash/v1/staff/session`,
            // `vpay_api::staff::SESSION_HEADER`. Not logged: the body of a live
            // session read carries the minted access token.
            headers: {
              "x-vpay-staff-session": token,
              accept: "application/json",
            },
            failOnStatusCode: false,
            log: false,
          });

        // Alive first, so the refusal below is a change and not a token that
        // never worked.
        ask().its("status").should("eq", 200);

        cy.contains("button", "Sign out").click();
        cy.location("pathname").should("eq", "/login");
        cy.getCookie(SESSION_COOKIE).should("not.exist");

        ask().its("status").should("eq", 401);
      });

      // And a signed-out browser is refused the account page again.
      cy.visit("/");
      cy.location("pathname").should("eq", "/login");
    });

    it("signs the same person back in with the password they chose (case 1)", () => {
      // A plain sign-in: it proves the change above really replaced the
      // credential (a change that silently did nothing would leave every
      // assertion before it green), that enrolment is not repeated, and that
      // with the one-time password behind them the person lands on the account
      // page directly. vpay's replay guard admits only a strictly greater TOTP
      // step than the enrolment's, so this waits for the next one.
      waitForNextTotpStep();

      cy.visit("/login");
      signIn(staffEmail(), NEW_PASSWORD);

      cy.location("pathname").should("eq", "/login/totp");
      cy.get('[data-testid="totp-qr"]').should("not.exist");
      cy.contains("h2", "Enter your code").should("be.visible");

      carriedForward(TOTP_SECRET)
        .then((secret) => totpDigits(secret))
        .then((code) => {
          cy.get("#dashboard-step-code").type(code);
          cy.contains("button", "Continue").click();
        });

      // Straight to the account page: `/login/password` is not on the way any
      // more.
      cy.location("pathname", { timeout: 20_000 }).should("eq", "/");
      cy.contains("h2", /^Signed in as /).should("be.visible");
      cy.contains(staffEmail()).should("be.visible");
    });
  },
);
