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
 * 3. the same screen — one valid code, which is what commits the enrolment.
 * 4. `/login/password` — `staff add` sets `password_change_required`, and
 *    every authenticated route refuses such a session, `/oauth/authorize`
 *    included. So no `/dash/v1` token can exist until this is done.
 * 5. only then `/payments`, rendered from a token minted by the
 *    authorization-code grant with PKCE, whose exchange this app's own server
 *    performed.
 */
import { totpDigits, waitForNextTotpStep } from "../support/dashboard";

/** The staff address `just demo-staff` created. */
const staffEmail = (): string => Cypress.expose("STAFF_EMAIL") as string;

/** A password this spec sets, replacing the printed one-time password. */
const NEW_PASSWORD = "a-long-enough-demo-password";

describe("the dashboard", () => {
  it("sends an unauthenticated visitor to the sign-in form, and no further", () => {
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
        cy.get("#dashboard-totp-code").type(code);
        cy.contains("button", /finish enrolment/i).click();
      });

    // ---- leg 4: the printed password must be replaced --------------------
    cy.location("pathname").should("eq", "/login/password");
    cy.get("#dashboard-new-password").type(NEW_PASSWORD, { log: false });
    cy.get("#dashboard-confirm-password").type(NEW_PASSWORD, { log: false });
    cy.contains("button", "Set password").click();

    // ---- leg 5: signed in, on a token the code exchange minted -----------
    cy.location("pathname").should("eq", "/payments");
    cy.contains(staffEmail()).should("be.visible");
  });

  it("keeps the session token httpOnly and the access token out of the browser entirely", () => {
    // ADR-0008 and ADR-0017 decision 4, asserted in the browser rather than
    // in a unit test: the session cookie is httpOnly, and the `/dash/v1`
    // bearer token is never in the document at all — it lives in the
    // `staff_sessions` row and is read back server-side on every render.
    cy.getCookie("vpay_dash_session").should((cookie) => {
      expect(cookie, "the session cookie").to.not.equal(null);
      expect(cookie?.httpOnly, "httpOnly").to.equal(true);
      expect(cookie?.sameSite, "sameSite").to.equal("lax");
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

  it("lists this merchant's payments and opens one of them", () => {
    // A payment of this spec's own, so the assertion does not depend on
    // whichever other spec ran first. `checkout.cy.ts` creates payments for
    // the same tenant and they show up in this list too — this one is here
    // because its id is known.
    cy.task<{ id: string }>("mintCheckoutPaymentIntent").then((minted) => {
      cy.visit("/payments");

      cy.contains("h2", "Payments").should("be.visible");
      cy.get("table tbody tr").should("have.length.at.least", 1);

      // The row for the payment just created, opened by its own link.
      cy.get(`[data-payment-id="${minted.id}"]`)
        .should("exist")
        .find("a")
        .click();

      cy.location("pathname").should("eq", `/payments/${minted.id}`);
      cy.get('[data-testid="detail-id"]').should("have.text", minted.id);

      // The timeline is real events out of the `events` table, not a
      // rendered guess: an intent that has just been created has exactly the
      // one event that says so.
      cy.contains("payment_intent.created").should("be.visible");

      // An intent nobody has confirmed has no charge, and the page says that
      // rather than rendering an empty charge block.
      cy.contains("No charge").should("be.visible");
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
        });
      };
      check(0);
    });
  });

  it("refuses everything again after signing out", () => {
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
