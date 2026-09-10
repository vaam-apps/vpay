/**
 * The client half of `/c/{id}` and `/e/{id}`.
 *
 * Everything that needs a browser lives here and nowhere else: reading the
 * fragment, resolving the framer, opening the `postMessage` channel,
 * building the `@vaam-apps/vpay-stripe-js` client, and reading and writing
 * this device's page memory. The decisions it makes are all imported —
 * `decideEntry`, `reduce`, `forwardTarget`, `memoryRecordFor` — so this file
 * is wiring, not policy.
 *
 * **No timer navigates.** There was a five-second auto-forward here until
 * 2026-09-06; the outcome screen now has a button and nothing else. What
 * remains of that machinery is `forward`, called from the button's handler.
 */
"use client";

import { loadStripe, type Stripe } from "@vaam-apps/vpay-stripe-js";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import type { Branding, CheckoutSettings } from "../config/settings";
import { pickLocale, translator, type Locale } from "../i18n/index";
import { BrowserCheckoutApi } from "../lib/api";
import { CheckoutController } from "../lib/controller";
import { decideEntry } from "../lib/entry";
import { createFrameChannel, type FrameChannel } from "../lib/frame";
import { rememberPublishableKey } from "../lib/link";
import { forwardKindFor, forwardTarget } from "../lib/forward";
import { INITIAL_STATE, type CheckoutState } from "../lib/machine";
import { browserPageMemory } from "../lib/memory-idb";
import {
  memoryRecordFor,
  pageMemoryFor,
  type PageMemoryRecord,
} from "../lib/memory";
import { normalizeCameroonMsisdn } from "../lib/msisdn";
import { CheckoutView } from "./checkout-view";

export interface CheckoutClientProps {
  sessionId: string;
  /** `NEXT_PUBLIC_VPAY_API_URL` — the origin `/v1/browser/...` hangs off. */
  apiBaseUrl: string;
  mode: "hosted" | "embedded";
  /** Resolved server-side from `GET /v1/browser/checkout/origins`. Empty for a hosted page. */
  allowedOrigins: readonly string[];
  /** Chosen from `Accept-Language` on the server. The switch changes it here. */
  initialLocale: Locale;
  /** `branding.yaml`, read at container start and passed down rather than fetched. */
  branding: Branding;
  /** `config.yaml`'s `checkout:` section, same. */
  settings: CheckoutSettings;
}

export function CheckoutClient(props: CheckoutClientProps) {
  const [locale, setLocale] = useState<Locale>(props.initialLocale);
  const [state, setState] = useState<CheckoutState>(INITIAL_STATE);
  const [remembered, setRemembered] = useState<PageMemoryRecord | null>(null);
  const [remember, setRemember] = useState(false);
  const [forgotten, setForgotten] = useState(false);
  const controllerRef = useRef<CheckoutController | null>(null);

  /**
   * The store, or the one that remembers nothing.
   *
   * Memoised on the flag alone, which is a prop that does not change while a
   * payer is on the page — so this value is stable for the page's life and
   * safe to name in the dependency array of the effect that reads it. It was
   * held in a ref for one revision; a ref written during render is a React
   * defect (`react-hooks/refs` caught it) and there was nothing here that
   * needed one.
   */
  const { memory: pageMemory, offered: memoryOffered } = useMemo(
    () =>
      pageMemoryFor(props.settings.features.pageMemory, browserPageMemory()),
    [props.settings.features.pageMemory],
  );

  useEffect(() => {
    document.documentElement.lang = locale;
  }, [locale]);

  /**
   * One warning, in the browser console, when this container's
   * `checkout.public_base_url` disagrees with the origin the page was
   * actually loaded from.
   *
   * It changes nothing a payer sees. It exists because the misconfiguration
   * it names — a proxy in front of this container that the API's own
   * `checkout.public_base_url` does not know about — produces links that
   * work for whoever built them and fail for everyone else, with no error
   * anywhere.
   */
  useEffect(() => {
    const configured = props.settings.publicBaseUrl;
    if (configured === null) {
      return;
    }
    let expected: string;
    try {
      expected = new URL(configured).origin;
    } catch {
      return;
    }
    if (expected !== window.location.origin) {
      // A deployment diagnostic naming two origins. Both are public by
      // construction: one comes from a mounted config file and the other is
      // `window.location.origin`, and neither is a session, a payer or a
      // credential.
      //
      // NOTE that `secrets.test.ts`' console trace does NOT cover this line.
      // That trace spies on `console` around the two CONTROLLERS, which is
      // where every value that could be a secret lives; this component is not
      // driven by it. What keeps this call safe is the two values it names,
      // not a test.
      // eslint-disable-next-line no-console -- see the paragraph above.
      console.warn(
        `[vpay-checkout] configured public_base_url origin ${expected} is not the origin this page was loaded from (${window.location.origin})`,
      );
    }
  }, [props.settings.publicBaseUrl]);

  useEffect(() => {
    const decision = decideEntry({
      mode: props.mode,
      search: window.location.search,
      hash: window.location.hash,
      referrer: document.referrer,
      allowedOrigins: props.allowedOrigins,
      framed: window.parent !== window,
      // A popup: `/c/{id}` opened by `window.open` from the merchant's page.
      // `window.parent` is this window, so nothing above would have found a
      // peer — see `frame.ts`'s `ChannelPeer`.
      hasOpener: window.opener !== null && window.opener !== undefined,
    });

    if (decision.kind === "refused") {
      // REAL finding, not suppressed as a false positive: `decideEntry` reads
      // `window.location`/`document.referrer`, which a Next server render
      // cannot, so the first state is settled in an effect. Fixing it properly
      // means reshaping this page's entry state machine; that is not a lint
      // pass's change.
      // eslint-disable-next-line react-hooks/set-state-in-effect
      setState({ name: "refused", reason: "embed_not_allowed", context: null });
      return;
    }
    if (decision.kind === "error") {
      setState({ name: "error", error: { code: decision.code } });
      return;
    }

    // Only the publishable key, and only so the return page can find it in
    // this tab. Never the secret — see `link.ts`.
    rememberPublishableKey(
      window.sessionStorage,
      props.sessionId,
      decision.key,
    );

    let channel: FrameChannel | null = null;
    if (decision.parentOrigin !== null) {
      channel = createFrameChannel({
        win: window,
        parentOrigin: decision.parentOrigin,
        observe: document.documentElement,
      });
    } else if (decision.openerOrigin !== null) {
      // No `observe`: a popup sizes itself, and the channel would refuse to
      // report a height to an opener anyway.
      channel = createFrameChannel({
        win: window,
        peer: "opener",
        parentOrigin: decision.openerOrigin,
      });
    }

    let disposed = false;
    let unsubscribe: (() => void) | null = null;

    void (async () => {
      // Memory is read BEFORE the session, and before any screen that could
      // show a prefilled field exists. That ordering is what lets the MSISDN
      // input stay uncontrolled: by the time a form renders, the value it
      // would default to is already known, so a payer is never typing into a
      // field about to be replaced. The read is local and the session read
      // that follows is a network round trip, so it costs nothing visible.
      const record = await pageMemory.read();
      if (disposed) {
        return;
      }
      setRemembered(record);
      setRemember(record !== null);

      let stripe: Stripe;
      try {
        stripe = await loadStripe(decision.key, { baseUrl: props.apiBaseUrl });
      } catch {
        // `loadStripe` rejects only on a blank key or base URL — an
        // integration mistake, not a payer-facing failure. The thrown value
        // is not read: it is the one place in that package that throws, and
        // its message is not for a payer.
        setState({ name: "error", error: { code: "error.unexpected" } });
        return;
      }
      if (disposed) {
        return;
      }
      const controller = new CheckoutController({
        sessionId: props.sessionId,
        credentials: { key: decision.key, clientSecret: decision.clientSecret },
        api: new BrowserCheckoutApi({ baseUrl: props.apiBaseUrl }),
        stripe,
        navigate: (url) => window.location.assign(url),
        closeWindow: () => window.close(),
        opener: () => window.opener as Window | null,
        channel,
        allowedMethods: props.settings.allowedMethods,
      });
      controllerRef.current = controller;
      unsubscribe = controller.subscribe(setState);
      setState(controller.state);
      await controller.start();
    })();

    return () => {
      disposed = true;
      unsubscribe?.();
      channel?.dispose();
      controllerRef.current = null;
    };
  }, [
    pageMemory,
    props.apiBaseUrl,
    props.allowedOrigins,
    props.mode,
    props.sessionId,
    props.settings.allowedMethods,
  ]);

  const destination = useMemo(() => {
    if (state.name !== "outcome") {
      return null;
    }
    return forwardTarget(
      state.context.session,
      forwardKindFor(state.context.session, state.kind === "succeeded"),
    );
  }, [state]);

  const onReturnToMerchant = useCallback(() => {
    if (destination !== null) {
      controllerRef.current?.forward(destination);
    }
  }, [destination]);

  /**
   * The one moment anything is written to this device.
   *
   * Ticked box → the record; unticked box → the record is removed. Both run
   * on a deliberate submit, so a payer who unticks and pays has cleared the
   * device by paying, and a payer who merely visits has left nothing behind.
   */
  const rememberOnSubmit = useCallback(
    (msisdn: string | null, rail: string | null) => {
      if (!remember) {
        void pageMemory.clear();
        setRemembered(null);
        return;
      }
      const record = memoryRecordFor({ msisdn, rail }, Date.now());
      if (record === null) {
        return;
      }
      void pageMemory.write(record);
      setRemembered(record);
    },
    [pageMemory, remember],
  );

  const onSubmitMsisdn = useCallback(
    (raw: string) => {
      // Normalised here as well as in the controller, and deliberately: what
      // is stored is the canonical `2376XXXXXXXX`, and a number the page
      // would refuse to send is a number it must not keep either.
      rememberOnSubmit(normalizeCameroonMsisdn(raw), "mtn_momo");
      void controllerRef.current?.submitMsisdn(raw);
    },
    [rememberOnSubmit],
  );

  const onStartRedirect = useCallback(() => {
    const rail = state.name === "ready_redirect" ? state.rail.code : null;
    rememberOnSubmit(null, rail);
    void controllerRef.current?.startRedirect();
  }, [rememberOnSubmit, state]);

  const onForget = useCallback(() => {
    void pageMemory.clear();
    setRemembered(null);
    setRemember(false);
    setForgotten(true);
  }, [pageMemory]);

  const t = useMemo(() => translator(locale), [locale]);

  return (
    <CheckoutView
      state={state}
      t={t}
      locale={locale}
      branding={props.branding}
      destination={destination}
      defaultMsisdn={remembered?.msisdn ?? null}
      lastRail={remembered?.rail ?? null}
      memory={{
        offered: memoryOffered,
        remember,
        onRememberChange: (next) => {
          setRemember(next);
          setForgotten(false);
        },
        hasRecord: remembered !== null,
        onForget,
        forgotten,
      }}
      onChooseRail={(rail) => controllerRef.current?.chooseRail(rail)}
      onBack={() => controllerRef.current?.back()}
      onSubmitMsisdn={onSubmitMsisdn}
      onStartRedirect={onStartRedirect}
      onRetryPoll={() => void controllerRef.current?.retryPoll()}
      onReturnToMerchant={onReturnToMerchant}
      onLocaleChange={setLocale}
    />
  );
}

/** Re-exported for the server components, which pick the locale before rendering. */
export { pickLocale };
