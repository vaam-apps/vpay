/**
 * `examples/shop` → vpay's HOSTED checkout page → back to the shop, in a real
 * browser, against the real compose stack (Step 9, lane 6).
 *
 * Nothing in this file stubs a rail. The MTN and Orange stubs are WireMock
 * hosts in `compose.yml` (ADR-0006), `vpay-worker` is the process that polls
 * them, and the only thing that ever marks an order `paid` is the shop's own
 * webhook handler writing its own database after verifying vpay's signature.
 * Every assertion below about money having moved is an assertion about a page
 * that reads that database (`OrderPoller` → `orders.get`) and nothing else —
 * which is why a spec here cannot pass while `vpay-worker` is down.
 *
 * The MSISDNs are the DIGITS-ONLY steering numbers added in Step 9 lane 2b
 * (`backends/tests/conformance/wiremock/mtn/mappings/`): vpay's page validates
 * Cameroon E.164 and refuses the older hex-suffixed twins (`237600000ce0` and
 * friends), which is correct — a form that accepted letters as a phone number
 * would accept them for every payer. See `docs/plans/step9-notes/lane-3.md` §4c.
 *
 * Cross-origin: the shop, vpay's page and the Orange stub are three different
 * ports, and Cypress treats a differing port as a differing origin
 * (`getSuperDomainOrigin` = protocol + superdomain + port), so each leg on a
 * foreign origin is inside its own `cy.origin()` with everything it needs
 * passed through `args`.
 *
 * **`color-contrast` checks, issue #73, plan §7 row 6.** Four of this file's
 * outcome screens — `CheckoutView`'s `succeeded` and `failed`, `ReturnView`'s
 * `succeeded` and `failed` — run axe-core's `color-contrast` rule in a REAL
 * browser, against vpay's own compiled stylesheet under `bumblebee`. A first
 * draft of this fix (`outcomes.axe.test.tsx`, since deleted) ran the same axe
 * rule in jsdom instead, which parses no CSS and computes no layout — a
 * `color-contrast` run there always finds zero violations whether or not one
 * exists, so ten green tests proved nothing and were worse than none: their
 * names read as "contrast checked". Here the browser paints the page and
 * axe-core reads its computed styles, so a violation found here is real.
 *
 * **Why axe-core directly, and not the `cypress-axe` package plan §7 row 6
 * names.** Measured, in order:
 *
 * 1. `cy.origin()` runs its callback in its own realm and does not inherit
 *    the support file's `Cypress.Commands.add('injectAxe', …)` registration
 *    — `TypeError: cy.injectAxe is not a function`.
 * 2. `Cypress.require("cypress-axe")` (Cypress's own escape hatch, behind
 *    `experimentalOriginDependencies`) fixes that, but `injectAxe()` itself
 *    calls `cy.readFile()` to load `axe-core/axe.min.js`, and `cy.readFile()`
 *    is refused inside `cy.origin()` — "must only be invoked from the spec
 *    file or support file".
 * 3. Reading the file once here and `window.eval`-ing it inside `cy.origin()`
 *    (exactly what `injectAxe()` does internally) works around that. But
 *    `cy.checkA11y()`'s violation callback also calls `cy.task()` to get a
 *    violation's ratios into the terminal (Cypress.log's consoleProps never
 *    reach a headless `cypress run`) — and `cy.task()` is refused inside
 *    `cy.origin()` for the same reason `cy.readFile()` is. Proven with a
 *    forced violation (mutation below): the test failed with `CypressError:
 *    cy.task() must only be invoked from the spec file or support file`
 *    instead of a useful message.
 *
 * So every check below runs axe-core directly inside `cy.origin()` (only
 * `cy.window()`, unrestricted there) and returns its result as the value
 * `cy.origin()` yields, exactly like `msisdn` travels in via `args`; the
 * assertion and the ratio logging happen in `reportContrast()`, outside
 * `cy.origin()`, where `cy.task()` is allowed. The page's forward button is
 * clicked in a second, separate `cy.origin()` call to the same origin (no
 * reload — the browser context is already there) so a click that navigates
 * away is never the same callback that still has a value to hand back.
 *
 * **The decisive mutation, and what it actually found — three rounds.**
 * (1) Reverting `@vpay/ui/src/styles.css`'s `--color-error-content` fix
 * (`oklch(32% .13 25.723)`) to daisyUI 5's own bumblebee default
 * (`oklch(39% .141 25.723)`, the documented 3.53:1) did not fail this check.
 * (2) Setting it to the exact same value as `--color-error`
 * (`oklch(70% .191 22.216)`, foreground literally equal to background —
 * confirmed identical via `getComputedStyle`) did not either. (3) Replacing
 * BOTH `--color-error` and `--color-error-content` with plain, identical,
 * axe-parseable hex (`#ff0000`/`#ff0000`, confirmed applied — the compiled
 * CSS carries `--color-error:red;--color-error-content:red` as an unlayered
 * override, which wins by CSS cascade-layer rules regardless of source
 * order) **still did not fail it.**
 *
 * Every round's `incomplete` result names the same cause:
 * `messageKey: "bgImage"` on the `.mt-4.alert.alert-error` element itself —
 * "Element's background color could not be determined due to a background
 * image". Not an oklch-parsing problem after all (round 3 used no oklch at
 * all): axe-core's background-colour walk gives up the moment ANY ancestor
 * carries a `background-image`, and one always does here. `daisyui@5.7.28`'s
 * own `base/rootscrollgutter.css` installs, unconditionally, on `:root`:
 *
 *     background-image: var(--page-scroll-lock)
 *       linear-gradient(var(--root-bg,#0000), var(--root-bg,#0000));
 *
 * part of a CSS-only scrollbar-gutter-stability trick for locking page
 * scroll behind an open `<dialog>`/`Drawer` — present, and this rule fires,
 * on every page using daisyUI 5's `@plugin 'daisyui'` whether or not a
 * dialog is open or `--root-bg` is anything but transparent. `:root` is an
 * ancestor of every element on the page, so this is not specific to the
 * outcome screens, this component, or this repository: **axe-core's
 * `color-contrast` rule cannot produce a `violation` verdict for ANY element
 * on ANY daisyUI-5 page**, only ever `incomplete`. `dequelabs/axe-core#4007`
 * ("Axe fails with new color spec", open) is a related, separately-reported
 * gap in the same area (oklch parsing) but is not what fires here — this
 * was traced to a specific rule, in a specific upstream file, not inferred.
 *
 * `reportContrast()` still asserts on `violations` (in case some future axe
 * release, or a change to this backdrop mechanism, lets a real verdict
 * through) and logs every `incomplete` result via `cy.task('dump', …)`, so
 * a reader of this file's own CI output sees the gap on every run rather
 * than a silent pass. **A clean run of this file is not evidence the four
 * screens clear WCAG AA — it is evidence axe-core could not check, four
 * times, for a reason outside this file's or this repository's control.**
 * Fixing it means either an axe-core release that tolerates a fully
 * transparent `background-image`, or moving contrast measurement onto
 * something that does not walk the DOM for a background colour at all —
 * `@vpay/ui`'s `theme-contrast.test.ts` already does the latter for the
 * theme's raw tone palette (computes WCAG ratios from the compiled OKLCh
 * values directly), and the 2026-09-07 Lane B review measured this exact
 * regression by sampling a committed screenshot's own pixels instead of
 * asking a browser API to. Neither approach is this file's to build without
 * the maintainer choosing which one is worth building — see
 * `docs/flows/hosted-checkout.md`'s Status section, and issue #73, which
 * this leaves open on the contrast half.
 *
 * Not covered: the `canceled` outcome kind (`PaymentIntent.status ===
 * "canceled"`, distinct from the `failed` cases this file drives). No spec
 * anywhere reaches it — it is not a rail decline, it is the intent being
 * cancelled out from under an open checkout — so its contrast is unmeasured
 * by this file or any other, on top of the gap above.
 */

import type axeCore from "axe-core";

import {
  MTN,
  buyOnVpaysPage,
  checkoutOrigin,
  orangeOrigin,
  orderIdFromUrl,
  readOrder,
  waitForOrderStatus,
} from "../support/shop";

/**
 * `window.axe`, set by `window.eval(axeSource)` below — never imported as a
 * module (that would pull axe-core into the app bundle instead of the real
 * one already compiled into the page), only typed so `win.axe.run(…)`
 * checks.
 */
declare global {
  interface Window {
    axe: typeof axeCore;
  }
}

/** The subset of axe-core's `Result` shape this file reads. */
interface AxeResult {
  id: string;
  impact: string | null;
  help: string;
  nodes: Array<{
    target: string[];
    failureSummary?: string;
    html: string;
    any: Array<{
      data?: {
        contrastRatio?: number;
        expectedContrastRatio?: string;
        fgColor?: string;
        bgColor?: string;
        messageKey?: string;
      };
    }>;
  }>;
}

/** What each `cy.origin()` color-contrast check below yields. */
interface ContrastCheck {
  violations: AxeResult[];
  incomplete: AxeResult[];
}

/**
 * Asserts on `violations` and logs both `violations` and `incomplete` via
 * `cy.task` — called OUTSIDE `cy.origin()`, where `cy.task()` is allowed.
 * See this file's header comment for why `incomplete` is logged rather than
 * asserted on: axe-core's `color-contrast` rule cannot produce a
 * `violation` verdict for any element on this page, because daisyUI 5's own
 * `:root` scroll-lock CSS carries a `background-image` axe's walk gives up
 * on — measured for every one of this file's four checks, every run.
 */
function reportContrast(result: ContrastCheck, label: string): void {
  if (result.incomplete.length > 0) {
    cy.task("dump", {
      what: `color-contrast incomplete (${label}) — axe-core could not determine a verdict (daisyUI 5's :root background-image; see this file's header comment)`,
      value: JSON.stringify(result.incomplete, null, 2),
    });
  }
  if (result.violations.length > 0) {
    cy.task("logA11yViolations", result.violations);
  }
  expect(
    result.violations,
    `${label}: color-contrast violations axe-core actually detected`,
  ).to.have.length(0);
}

describe("the shop, paid on vpay's hosted page", () => {
  /**
   * axe-core's minified source — read once here because `cy.readFile()` is
   * refused inside `cy.origin()` (see header comment) — and threaded through
   * every `cy.origin()` call's `args` below, exactly like `msisdn`.
   */
  let axeSource = "";
  before(() => {
    cy.readFile("node_modules/axe-core/axe.min.js").then((source: string) => {
      axeSource = source;
    });
  });

  it("MTN push: the payer pays on vpay's page and the shop reaches `paid` through the webhook", () => {
    buyOnVpaysPage();

    cy.origin(
      checkoutOrigin(),
      { args: { msisdn: MTN.succeeds, axeSource } },
      ({ msisdn, axeSource }) => {
        cy.window({ log: false }).then((win) => {
          win.eval(axeSource);
        });

        // The rail selector, because the shop's intent offers both rails and
        // vpay's page can drive both (D9). Its presence is already a fact
        // about the intent: the page renders what `payment_method_types`
        // says, never a hard-coded list.
        cy.get('[data-screen="select_rail"]', { timeout: 60_000 }).should(
          "be.visible",
        );
        cy.get('[data-testid="amount"]').should("contain.text", "12");
        cy.get('button[data-rail="orange_money"]').should("exist");
        cy.get('button[data-rail="mtn_momo"]').click();

        cy.get('[data-screen="collect_msisdn"]').should("be.visible");
        cy.get("#vpay-msisdn").type(msisdn);
        cy.get('button[type="submit"]').click();

        // `waiting` is the page polling `/v1/browser/payment_intents/{id}`
        // through `@vaam-apps/vpay-stripe-js`. What moves the intent underneath it is
        // `vpay-worker` polling MTN's stub — nothing in this spec pushes the
        // status forward.
        cy.get('[data-screen="waiting"]', { timeout: 60_000 }).should("exist");
        cy.get('[data-outcome="succeeded"]', { timeout: 120_000 }).should(
          "be.visible",
        );
        // color-contrast, `CheckoutView`'s succeeded outcome, under
        // bumblebee — the last command, so its result is what cy.origin()
        // yields. See this file's header comment.
        cy.window({ log: false }).then((win) =>
          win.axe.run('[data-outcome="succeeded"]', {
            runOnly: { type: "rule", values: ["color-contrast"] },
          }),
        );
      },
    ).then((results) => {
      reportContrast(
        results as ContrastCheck,
        "CheckoutView succeeded outcome",
      );
    });

    // The forward is the merchant's `success_url` with `{CHECKOUT_SESSION_ID}`
    // substituted (D5). The click is the ONLY way off this screen: the
    // five-second auto-forward this comment used to describe was removed on
    // 2026-09-06 and nothing replaced it, so a spec that merely waited here
    // would wait forever. A separate `cy.origin()` call — no reload, the
    // browser context is already there — because a navigating click cannot
    // share a callback with a value that still needs handing back.
    cy.origin(checkoutOrigin(), () => {
      cy.get('[data-outcome="succeeded"] button').click();
    });

    // Back on the shop. This page reads the shop's database; it takes no
    // decision from the `session_id` vpay put in the query string.
    cy.url({ timeout: 60_000 }).should("include", "/return");
    cy.get('[data-testid="return-session-id"]').should("contain.text", "cs_");
    cy.get('[data-testid="paid-message"]', { timeout: 120_000 }).should(
      "be.visible",
    );
    cy.get('[data-testid="order-status"]').should("have.text", "Paid");
    cy.get('[data-testid="confirming"]').should("not.exist");

    orderIdFromUrl().then((orderId) => {
      readOrder(orderId).then((order) => {
        expect(order.status, "the shop's own orders.get").to.equal("paid");
        expect(order.paymentIntentId).to.match(/^pi_/);
        expect(order.checkoutSessionId).to.match(/^cs_/);
      });
    });
  });

  it("Orange redirect: the payer pays on the rail's own page and the shop reaches `paid`", () => {
    buyOnVpaysPage();

    cy.origin(checkoutOrigin(), () => {
      cy.get('[data-screen="select_rail"]', { timeout: 60_000 }).should(
        "be.visible",
      );
      cy.get('button[data-rail="orange_money"]').click();
      // No form: a redirect rail collects nothing here.
      cy.get('[data-screen="ready_redirect"]').should("be.visible");
      // `data-testid`, not `button.btn-primary`: exp26 (2026-09-07) made
      // `Button` a `@vpay/ui` component whose primary look is a `cva`
      // variant default rather than a class this spec should know the name
      // of — see docs/plans/2026-09-07-ui-revamp.md §4.1.
      cy.get('[data-testid="continue"]').click();
      // The confirm answers `next_action.redirect_to_url` and the page
      // navigates top-level to the rail. `cy.origin` for the rail follows.
    });

    // Orange's own hosted page — a WireMock mapping (D7), not a vpay page.
    // Its two links `302` to the `return_url` and `cancel_url` THAT submit
    // carried, which since Step 9 lane 2 is vpay's own return page for this
    // session, carrying the `t=` return token.
    //
    // **This click now decides the charge, and until 2026-09-10 it decided
    // nothing** (vpay issue #58). The stub used to answer the very first
    // `transactionstatus` `SUCCESS`, about 449 ms after the submit, so this
    // charge was already `paid` by the time the page below had rendered —
    // the spec passed, and it was passing for a reason that had nothing to
    // do with the payer. The stub now answers `PENDING` while a payer is on
    // the page, and the two controls point back at the stub itself so it can
    // tell a payer who paid from one who wandered off. See
    // `backends/tests/conformance/wiremock/orange/mappings/stub-hosted-page.json`.
    cy.origin(orangeOrigin(), () => {
      cy.get("#pay", { timeout: 60_000 }).should("be.visible");
      // Both controls go back through the rail's own container. A link
      // straight to the merchant is a payer the rail never hears from
      // again, which is the shape this had before and the reason its test
      // numbers did not work from a browser.
      cy.get("#pay")
        .invoke("attr", "href")
        .should("match", /^\/stub-hosted-page\/[^/]+\/pay\?/u);
      cy.get("#cancel")
        .invoke("attr", "href")
        .should("match", /^\/stub-hosted-page\/[^/]+\/cancel\?/u);
      cy.get("#pay").click();
    });

    // vpay's return page: it holds the return token, not the intent's
    // secret, and it polls until the rail's status query settles.
    cy.origin(
      checkoutOrigin(),
      { args: { axeSource } },
      ({ axeSource }) => {
        cy.window({ log: false }).then((win) => {
          win.eval(axeSource);
        });

        cy.url({ timeout: 60_000 }).should("include", "/return");
        cy.get('[data-outcome="succeeded"]', { timeout: 120_000 }).should(
          "be.visible",
        );
        // color-contrast, `ReturnView`'s succeeded outcome, under bumblebee.
        cy.window({ log: false }).then((win) =>
          win.axe.run('[data-outcome="succeeded"]', {
            runOnly: { type: "rule", values: ["color-contrast"] },
          }),
        );
      },
    ).then((results) => {
      reportContrast(
        results as ContrastCheck,
        "ReturnView succeeded outcome",
      );
    });

    cy.origin(checkoutOrigin(), () => {
      cy.get('[data-outcome="succeeded"] button').click();
    });

    cy.url({ timeout: 60_000 }).should("include", "/return");
    cy.get('[data-testid="paid-message"]', { timeout: 120_000 }).should(
      "be.visible",
    );
    cy.get('[data-testid="order-status"]').should("have.text", "Paid");

    orderIdFromUrl().then((orderId) => {
      readOrder(orderId).then((order) => {
        expect(order.status).to.equal("paid");
      });
    });
  });

  it("Orange redirect: the payer cancels on the rail's own page and the order reaches `failed`", () => {
    // **The other half of vpay issue #58, and the one that could not be
    // written at all before 2026-09-10.**
    //
    // Until then the rail stub's `#cancel` link pointed straight at the
    // `cancel_url` the submit carried, so a payer who clicked it never
    // touched the stub again — and the stub, asked for a status it had not
    // been steered on, answered `SUCCESS`. A payer who cancelled was paid.
    // The comment on the case above says so, because it was true.
    //
    // The link now goes to `/stub-hosted-page/{token}/cancel` on the rail's
    // own container, which arms `EXPIRED` and *then* `302`s to the same URL.
    // `EXPIRED` and not a `CANCELLED` of this repository's invention: Orange
    // documents five statuses and that is not one of them
    // (`docs/flows/adapter-orange-money.md`), so a payer who abandons the
    // page is a page that ended unpaid, which is `payer_timeout`.
    //
    // Nothing here stubs anything. The order moves because `vpay-worker`
    // asked the rail, the rail said `EXPIRED`, vpay emitted
    // `payment_intent.payment_failed`, and this shop's webhook handler
    // verified the signature and wrote its own database.
    buyOnVpaysPage();

    cy.origin(checkoutOrigin(), () => {
      cy.get('[data-screen="select_rail"]', { timeout: 60_000 }).should(
        "be.visible",
      );
      cy.get('button[data-rail="orange_money"]').click();
      cy.get('[data-screen="ready_redirect"]').should("be.visible");
      cy.get('[data-testid="continue"]').click();
    });

    cy.origin(orangeOrigin(), () => {
      cy.get("#cancel", { timeout: 60_000 }).should("be.visible");
      cy.get("#cancel").click();
    });

    // `cancel_url` and `return_url` are the same value on this rail, so the
    // payer lands back on vpay's return page either way and the page polls
    // until the rail's status query settles. What differs from the case
    // above is the outcome it settles on.
    cy.origin(
      checkoutOrigin(),
      { args: { axeSource } },
      ({ axeSource }) => {
        cy.window({ log: false }).then((win) => {
          win.eval(axeSource);
        });

        cy.url({ timeout: 60_000 }).should("include", "/return");
        cy.get('[data-outcome="failed"]', { timeout: 120_000 }).should(
          "be.visible",
        );
        cy.get('[data-testid="outcome-body"]').should("not.be.empty");
        // color-contrast, `ReturnView`'s failed outcome, under bumblebee —
        // the screen that tells a payer their money did not move.
        cy.window({ log: false }).then((win) =>
          win.axe.run('[data-outcome="failed"]', {
            runOnly: { type: "rule", values: ["color-contrast"] },
          }),
        );
      },
    ).then((results) => {
      reportContrast(
        results as ContrastCheck,
        "ReturnView failed outcome",
      );
    });

    cy.origin(checkoutOrigin(), () => {
      cy.get('[data-outcome="failed"] button').click();
    });

    // A hosted session sends every non-success to the merchant's
    // `cancel_url` (`forwardKindFor(session, paid)` with `paid === false`).
    cy.url({ timeout: 60_000 }).should("include", "/cancelled");
    cy.get('[data-testid="cancelled-message"]').should("be.visible");

    orderIdFromUrl().then((orderId) => {
      // `waitForOrderStatus` fails the moment it ever reads `paid` — which
      // is precisely what this spec would have read before the stub grew a
      // payer's window, and is the assertion that makes this case a gate on
      // the fix rather than a description of it.
      waitForOrderStatus(orderId, "failed");
    });
  });

  it("a payment that does not succeed lands on the shop's cancel_url, and the order never becomes `paid`", () => {
    // WHY A DECLINE AND NOT THE RAIL PAGE'S "Cancel" LINK. The plan's third
    // hosted case is spelled '"Cancel" → the shop's cancelled page with the
    // order still unpaid'. Two facts of the code as merged make the literal
    // reading unreachable, and neither is this spec's to change:
    //
    //  1. vpay's hosted page has no cancel control. Its only exits are the
    //     outcome screen's forward and closing the tab.
    //  2. The Orange stub's "Cancel" link went straight to the `cancel_url`
    //     THAT SUBMIT carried, and `vpay_adapter_orange_money` sends the
    //     charge's single `return_url` as both — so the link went to vpay's
    //     return page, the stub never heard about the click, and its
    //     `transactionstatus` mapping answered SUCCESS for any order_id it
    //     was not steered on. A payer who "cancelled" on the stub was paid
    //     anyway.
    //
    //     **The second one stopped being true on 2026-09-10** (vpay issue
    //     #58): the stub's Cancel link now goes through the stub, which arms
    //     `EXPIRED` before forwarding, and the case below this one drives
    //     exactly that. This case stays as it is — it is the MTN half of the
    //     same property, and a decline at the rail and an abandonment at the
    //     rail's page are two different things arriving at one `cancel_url`.
    //
    // What actually reaches `cancel_url` is `forwardKindFor(session, paid)`
    // with `paid === false`: a hosted session sends every non-success there.
    // That is the property worth proving, and this is how it is reachable.
    //
    // The order does not stay `unpaid` either: vpay emits
    // `payment_intent.payment_failed`, the shop's webhook handler maps it to
    // `failed`, and asserting `unpaid` would be asserting the shop's webhook
    // did nothing. What is asserted is what matters — it is never `paid`, and
    // the cancelled page itself writes nothing.
    buyOnVpaysPage();

    cy.origin(
      checkoutOrigin(),
      { args: { msisdn: MTN.insufficientFunds, axeSource } },
      ({ msisdn, axeSource }) => {
        cy.window({ log: false }).then((win) => {
          win.eval(axeSource);
        });

        cy.get('[data-screen="select_rail"]', { timeout: 60_000 }).should(
          "be.visible",
        );
        cy.get('button[data-rail="mtn_momo"]').click();
        cy.get("#vpay-msisdn").type(msisdn);
        cy.get('button[type="submit"]').click();

        cy.get('[data-outcome="failed"]', { timeout: 120_000 }).should(
          "be.visible",
        );
        cy.get('[data-testid="outcome-body"]').should("not.be.empty");
        // color-contrast, `CheckoutView`'s failed outcome, under bumblebee —
        // the screen that tells a payer their money did not move.
        cy.window({ log: false }).then((win) =>
          win.axe.run('[data-outcome="failed"]', {
            runOnly: { type: "rule", values: ["color-contrast"] },
          }),
        );
      },
    ).then((results) => {
      reportContrast(
        results as ContrastCheck,
        "CheckoutView failed outcome",
      );
    });

    cy.origin(checkoutOrigin(), () => {
      cy.get('[data-outcome="failed"] button').click();
    });

    cy.url({ timeout: 60_000 }).should("include", "/cancelled");
    cy.get('[data-testid="cancelled-message"]').should("be.visible");
    cy.get('[data-testid="order-status"]').should("not.have.text", "Paid");

    orderIdFromUrl().then((orderId) => {
      // The shop's own record. `failed` because the signed
      // `payment_intent.payment_failed` was delivered and verified — the
      // cancel URL is a navigation, not an authority. `waitForOrderStatus`
      // fails the moment it ever reads `paid`.
      waitForOrderStatus(orderId, "failed");
    });
  });
});
