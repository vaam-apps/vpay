import { cleanup } from "@testing-library/react";
import { afterEach } from "vitest";

import "@testing-library/jest-dom/vitest";

/**
 * Unmount between tests.
 *
 * Testing Library only registers this itself when `globals` is on, and this
 * app's `vitest.config.ts` deliberately keeps it off. Without it every render
 * in a file accumulates in one `document.body`, so `getByRole` finds the
 * previous test's copy of a control and a suite that is really passing
 * reports "found multiple elements".
 */
afterEach(cleanup);

/**
 * jsdom does not implement a handful of browser APIs Base UI's interactive
 * components rely on for pointer capture and click simulation. Every one of
 * these is a no-op or minimal polyfill, not a behaviour stub — the
 * components still make their own real decisions; only the browser
 * plumbing underneath them is faked in.
 *
 * Identical to `frontends/packages/ui/vitest.setup.ts` — this app's recipe
 * tests render the same Base UI primitives (`Input`, `Button`, `Field`)
 * that package's own component tests do, and hit the same gap.
 */

class PointerEventPolyfill extends MouseEvent implements PointerEvent {
  readonly pointerId: number;
  readonly width: number;
  readonly height: number;
  readonly pressure: number;
  readonly tangentialPressure: number;
  readonly tiltX: number;
  readonly tiltY: number;
  readonly twist: number;
  readonly pointerType: string;
  readonly isPrimary: boolean;
  readonly altitudeAngle: number;
  readonly azimuthAngle: number;

  constructor(type: string, params: PointerEventInit = {}) {
    super(type, params);
    this.pointerId = params.pointerId ?? 0;
    this.width = params.width ?? 1;
    this.height = params.height ?? 1;
    this.pressure = params.pressure ?? 0;
    this.tangentialPressure = params.tangentialPressure ?? 0;
    this.tiltX = params.tiltX ?? 0;
    this.tiltY = params.tiltY ?? 0;
    this.twist = params.twist ?? 0;
    this.pointerType = params.pointerType ?? "mouse";
    this.isPrimary = params.isPrimary ?? true;
    this.altitudeAngle = params.altitudeAngle ?? 0;
    this.azimuthAngle = params.azimuthAngle ?? 0;
  }

  getCoalescedEvents(): PointerEvent[] {
    return [];
  }

  getPredictedEvents(): PointerEvent[] {
    return [];
  }
}

if (typeof window !== "undefined" && !window.PointerEvent) {
  window.PointerEvent = PointerEventPolyfill;
}

if (typeof Element !== "undefined") {
  if (!Element.prototype.hasPointerCapture) {
    Element.prototype.hasPointerCapture = () => false;
  }
  if (!Element.prototype.setPointerCapture) {
    Element.prototype.setPointerCapture = () => {};
  }
  if (!Element.prototype.releasePointerCapture) {
    Element.prototype.releasePointerCapture = () => {};
  }
  if (!Element.prototype.scrollIntoView) {
    Element.prototype.scrollIntoView = () => {};
  }
}

if (typeof window !== "undefined" && !window.ResizeObserver) {
  class ResizeObserverPolyfill implements ResizeObserver {
    observe() {}
    unobserve() {}
    disconnect() {}
  }
  window.ResizeObserver = ResizeObserverPolyfill;
}

/**
 * `window.matchMedia`, for `vaul` — the drawer engine under `@vaam-apps/ui`'s
 * `Drawer` / `QuickDetailDrawer` / `MoreDetailDrawer`, and since 0.4.0 under
 * `SideNav`'s More sheet, which `app-shell.test.tsx` and `a11y.test.tsx`
 * open.
 *
 * Measured, not assumed: without this, mounting an OPEN drawer throws
 * `TypeError: window.matchMedia is not a function` out of
 * `vaul/dist/index.mjs:855` — `window.matchMedia('(display-mode: standalone)')`,
 * in the effect that suppresses vaul's Safari-toolbar position-fixed hack for
 * a PWA. It throws from a passive effect, so vitest reports it as an
 * *unhandled error* beside a **passing** test rather than as a failure: the
 * drawer silently never opens and every assertion about its contents
 * vacuously holds. That is precisely the "test that asserts nothing" shape,
 * which is why this lives here rather than being stubbed per test file.
 *
 * `matches: false` for every query, matching the rest of this block's rule:
 * minimal plumbing, no behaviour stubbed. jsdom computes no layout, so no
 * media query it could be asked has a true answer here; vaul reads exactly
 * one and treats `false` as "not a PWA", its normal browser path.
 */
if (typeof window !== "undefined" && !window.matchMedia) {
  window.matchMedia = (query: string): MediaQueryList =>
    ({
      matches: false,
      media: query,
      onchange: null,
      addListener: () => {},
      removeListener: () => {},
      addEventListener: () => {},
      removeEventListener: () => {},
      dispatchEvent: () => false,
    }) as MediaQueryList;
}
