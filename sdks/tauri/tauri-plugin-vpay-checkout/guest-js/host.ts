/**
 * Picking a host. One line of policy, kept in its own module so that
 * `host-tauri.ts` and `host-web.ts` do not have to import each other.
 */
import { isTauri } from "@tauri-apps/api/core";

import { tauriHost } from "./host-tauri.js";
import { webHost } from "./host-web.js";
import type { CheckoutHost } from "./types.js";

/**
 * {@link tauriHost} inside a Tauri window, {@link webHost} in a plain
 * browser.
 *
 * `isTauri()` reads `__TAURI_INTERNALS__` off the global object, so this is
 * a property of the page rather than of the build — the same front-end
 * bundle really does run in both places, which is the whole reason this
 * package has two hosts.
 *
 * Neither factory touches `window` while being built, so calling this in
 * Node or during SSR is safe; a `webHost` that never finds a window
 * rejects `show` with a fixed message instead.
 */
export function defaultHost(): CheckoutHost {
  return isTauri() ? tauriHost() : webHost();
}
