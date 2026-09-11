// Shared e2e setup. No request stubbing lives here: the rails are stubbed by
// WireMock at the infrastructure layer (compose.yml), not by intercepting
// inside the browser — see docs/adr/0006.
//
// `shop-hosted.cy.ts`'s `color-contrast` checks (issue #73) drive axe-core
// directly (`window.eval`d, then `win.axe.run()`) rather than through the
// `cypress-axe` package's commands — that file's header comment says why:
// `cy.origin()` runs its callback in an isolated realm that shares neither
// `Cypress.Commands.add` registrations nor `cy.task()`/`cy.readFile()` with
// the support file, which is exactly what `cypress-axe`'s `injectAxe`/
// `checkA11y` commands are built on. Nothing here needs registering for that
// reason — `cypress-axe` was removed from this package's dependencies
// 2026-09-11 (b1e review) once it turned out to add plumbing rather than
// remove it.
export {};
