/**
 * The Tauri host: the two `plugin:vpay-checkout|…` commands and the
 * `Channel` the one event arrives on.
 *
 * This is the thin half of the plugin. Everything interesting — a partial
 * Custom Tab on Android, a detented `SFSafariViewController` on iOS, the
 * default browser on desktop — is behind `invoke`, and this file's whole job
 * is to spell the argument keys exactly as `src/models.rs`,
 * `VpayCheckoutPlugin.kt` and `VpayCheckoutPlugin.swift` expect them
 * (camelCase) and to make sure at most one event is forwarded.
 */
import { Channel, invoke } from "@tauri-apps/api/core";

import type {
  CheckoutHost,
  CheckoutWindowEvent,
  ShowCheckoutRequest,
} from "./types.js";

/** `plugin:{identifier}|{command}`, spelled once each. */
const SHOW_COMMAND = "plugin:vpay-checkout|show";
const DISMISS_COMMAND = "plugin:vpay-checkout|dismiss";

/**
 * The two `@tauri-apps/api/core` members this host uses, injectable so the
 * suite can drive it on Node where there is no Tauri IPC at all.
 *
 * Typed as `typeof invoke`/`typeof Channel` rather than as a narrower
 * structural pair, so that the real members are assignable without a cast
 * and a fake is the side that has to say it is one.
 */
export interface TauriHostDeps {
  invoke?: typeof invoke | undefined;
  Channel?: typeof Channel | undefined;
}

/**
 * A {@link CheckoutHost} backed by the Rust plugin.
 *
 * Note what is *not* here: no URL inspection, no stop-URL matching, no
 * outcome decision. `stopUrls` is forwarded for the native side to match a
 * deep link against (D2), and the event that comes back says only which of
 * two things happened (D1).
 */
export function tauriHost(deps: TauriHostDeps = {}): CheckoutHost {
  const invokeImpl = deps.invoke ?? invoke;
  const ChannelImpl = deps.Channel ?? Channel;

  return {
    async show(
      request: ShowCheckoutRequest,
      onEvent: (event: CheckoutWindowEvent) => void,
    ): Promise<void> {
      const channel = new ChannelImpl<CheckoutWindowEvent>();
      let reported = false;
      channel.onmessage = (event: CheckoutWindowEvent): void => {
        // The contract is one event per `show`. A host that sent two would
        // otherwise resolve `start` twice — harmless on the promise, but
        // the second would start a second poll loop against the same
        // intent. Ignored here rather than defended against in four hosts.
        if (reported) {
          return;
        }
        reported = true;
        onEvent(event);
      };
      // The keys are the wire contract. `onEvent` carries the Channel
      // itself: Tauri serialises it to the id the Rust side turns back into
      // a `tauri::ipc::Channel<CheckoutWindowEvent>`.
      await invokeImpl(SHOW_COMMAND, {
        url: request.url,
        stopUrls: request.stopUrls,
        allowInsecureUrl: request.allowInsecureUrl,
        onEvent: channel,
      });
    },

    async dismiss(): Promise<void> {
      await invokeImpl(DISMISS_COMMAND);
    },
  };
}
