import '@testing-library/jest-dom/vitest';

/**
 * jsdom does not implement a handful of browser APIs Base UI's interactive
 * components rely on for pointer capture, positioning and click
 * simulation. Every one of these is a no-op or minimal polyfill, not a
 * behaviour stub — the components still make their own real decisions
 * (checked state, open state, selected value); only the browser plumbing
 * underneath them is faked in, the same way jsdom itself fakes in layout.
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
    this.pointerType = params.pointerType ?? 'mouse';
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

if (typeof window !== 'undefined' && !window.PointerEvent) {
  window.PointerEvent = PointerEventPolyfill as unknown as typeof PointerEvent;
}

if (typeof Element !== 'undefined') {
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

if (typeof window !== 'undefined' && !window.ResizeObserver) {
  class ResizeObserverPolyfill implements ResizeObserver {
    observe() {}
    unobserve() {}
    disconnect() {}
  }
  window.ResizeObserver = ResizeObserverPolyfill;
}
