/**
 * The client half of `/c/{id}` and `/e/{id}`.
 *
 * Everything that needs a browser lives here and nowhere else: reading the
 * fragment, resolving the framer, opening the `postMessage` channel,
 * building the `@vpay/stripe-js` client, and the auto-forward countdown.
 * The decisions it makes are all imported — `decideEntry`, `reduce`,
 * `forwardTarget` — so this file is wiring, not policy.
 */
'use client';

import { loadStripe, type Stripe } from '@vpay/stripe-js';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';

import { pickLocale, translator, type Locale } from '../i18n/index';
import { BrowserCheckoutApi } from '../lib/api';
import { CheckoutController } from '../lib/controller';
import { decideEntry } from '../lib/entry';
import { createFrameChannel, type FrameChannel } from '../lib/frame';
import { rememberPublishableKey } from '../lib/link';
import { forwardKindFor, forwardTarget } from '../lib/forward';
import { INITIAL_STATE, type CheckoutState } from '../lib/machine';
import { CheckoutView } from './checkout-view';

/** Seconds a payer has to read the outcome before the page forwards on its own. */
export const AUTO_FORWARD_SECONDS = 5;

export interface CheckoutClientProps {
  sessionId: string;
  /** `NEXT_PUBLIC_VPAY_API_URL` — the origin `/v1/browser/...` hangs off. */
  apiBaseUrl: string;
  mode: 'hosted' | 'embedded';
  /** Resolved server-side from `GET /v1/browser/checkout/origins`. Empty for a hosted page. */
  allowedOrigins: readonly string[];
  /** Chosen from `Accept-Language` on the server. The switch changes it here. */
  initialLocale: Locale;
}

export function CheckoutClient(props: CheckoutClientProps) {
  const [locale, setLocale] = useState<Locale>(props.initialLocale);
  const [state, setState] = useState<CheckoutState>(INITIAL_STATE);
  const [secondsLeft, setSecondsLeft] = useState<number | null>(null);
  const controllerRef = useRef<CheckoutController | null>(null);

  useEffect(() => {
    document.documentElement.lang = locale;
  }, [locale]);

  useEffect(() => {
    const decision = decideEntry({
      mode: props.mode,
      search: window.location.search,
      hash: window.location.hash,
      referrer: document.referrer,
      allowedOrigins: props.allowedOrigins,
      framed: window.parent !== window,
    });

    if (decision.kind === 'refused') {
      setState({ name: 'refused', reason: 'embed_not_allowed', context: null });
      return;
    }
    if (decision.kind === 'error') {
      setState({ name: 'error', error: { code: decision.code } });
      return;
    }

    // Only the publishable key, and only so the return page can find it in
    // this tab. Never the secret — see `link.ts`.
    rememberPublishableKey(window.sessionStorage, props.sessionId, decision.key);

    let channel: FrameChannel | null = null;
    if (decision.parentOrigin !== null) {
      channel = createFrameChannel({
        win: window,
        parentOrigin: decision.parentOrigin,
        observe: document.documentElement,
      });
    }

    let disposed = false;
    let unsubscribe: (() => void) | null = null;

    void (async () => {
      let stripe: Stripe;
      try {
        stripe = await loadStripe(decision.key, { baseUrl: props.apiBaseUrl });
      } catch {
        // `loadStripe` rejects only on a blank key or base URL — an
        // integration mistake, not a payer-facing failure. The thrown value
        // is not read: it is the one place in that package that throws, and
        // its message is not for a payer.
        setState({ name: 'error', error: { code: 'error.unexpected' } });
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
        channel,
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
  }, [props.apiBaseUrl, props.allowedOrigins, props.mode, props.sessionId]);

  const destination = useMemo(() => {
    if (state.name !== 'outcome') {
      return null;
    }
    return forwardTarget(
      state.context.session,
      forwardKindFor(state.context.session, state.kind === 'succeeded'),
    );
  }, [state]);

  const onContinue = useCallback(() => {
    if (destination !== null) {
      controllerRef.current?.forward(destination);
    }
  }, [destination]);

  // The countdown. Visible, and the Continue button is always there — an
  // auto-forward a payer cannot pre-empt is a payment page that reads its
  // own outcome to itself.
  useEffect(() => {
    if (state.name !== 'outcome' || destination === null) {
      setSecondsLeft(null);
      return;
    }
    setSecondsLeft(AUTO_FORWARD_SECONDS);
    let remaining = AUTO_FORWARD_SECONDS;
    const timer = setInterval(() => {
      remaining -= 1;
      setSecondsLeft(remaining);
      if (remaining <= 0) {
        clearInterval(timer);
        controllerRef.current?.forward(destination);
      }
    }, 1_000);
    return () => {
      clearInterval(timer);
    };
  }, [state.name, destination]);

  const t = useMemo(() => translator(locale), [locale]);

  return (
    <CheckoutView
      state={state}
      t={t}
      locale={locale}
      destination={destination}
      secondsLeft={secondsLeft}
      onChooseRail={(rail) => controllerRef.current?.chooseRail(rail)}
      onBack={() => controllerRef.current?.back()}
      onSubmitMsisdn={(msisdn) => void controllerRef.current?.submitMsisdn(msisdn)}
      onStartRedirect={() => void controllerRef.current?.startRedirect()}
      onRetryPoll={() => void controllerRef.current?.retryPoll()}
      onContinue={onContinue}
      onLocaleChange={setLocale}
    />
  );
}

/** Re-exported for the server components, which pick the locale before rendering. */
export { pickLocale };
