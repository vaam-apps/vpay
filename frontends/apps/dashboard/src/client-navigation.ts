/**
 * Which clicks on a plain `<a>` this app may turn into a client-side
 * navigation, and where to.
 *
 * # Why this exists: the rail reloaded the whole page
 *
 * `@vaam-apps/ui`'s `SideNav` renders its destinations as plain `<a href>`
 * — the package has no dependency on Next, so it cannot render `next/link` —
 * and every one of them was therefore a full document load. Measured on a
 * production build, 2026-10-21: five of five rail clicks at 1440px each
 * fetched a new document, re-downloaded and re-hydrated the app, threw away
 * Refine's query cache and repainted the shell, ~1.0–1.4 s apiece where the
 * row links (`next/link`) took ~0.25 s.
 *
 * `AppShell` delegates one click handler around the rail and asks this
 * function whether the click is one a browser would have treated as an
 * ordinary same-tab navigation. React's synthetic events bubble through
 * portals along the component tree, so the floating rails and the More
 * sheet — both portalled to `document.body` — reach the handler too.
 *
 * Anything that is not that ordinary case is left alone, so the browser
 * does what it always did: a modified click (new tab, new window,
 * download), a middle click, a link with a `target` or a `download`
 * attribute, another origin, and a click something else already handled.
 *
 * @returns the path, query and hash to push, or `null` to let the browser
 * have the click.
 */
export interface ClickLike {
  readonly button: number;
  readonly metaKey: boolean;
  readonly ctrlKey: boolean;
  readonly shiftKey: boolean;
  readonly altKey: boolean;
  readonly defaultPrevented: boolean;
  readonly target: EventTarget | null;
}

export function inAppNavigationTarget(
  event: ClickLike,
  origin: string,
): string | null {
  if (event.defaultPrevented || event.button !== 0) {
    return null;
  }
  if (event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) {
    return null;
  }
  const target = event.target;
  if (typeof Element === "undefined" || !(target instanceof Element)) {
    return null;
  }
  const anchor = target.closest("a[href]");
  if (!(anchor instanceof HTMLAnchorElement)) {
    return null;
  }
  const frame = anchor.getAttribute("target");
  if (
    (frame !== null && frame !== "" && frame !== "_self") ||
    anchor.hasAttribute("download")
  ) {
    return null;
  }
  let url: URL;
  try {
    url = new URL(anchor.href, origin);
  } catch {
    return null;
  }
  if (url.origin !== origin) {
    return null;
  }
  return `${url.pathname}${url.search}${url.hash}`;
}
