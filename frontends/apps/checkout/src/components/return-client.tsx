/**
 * The client half of `/c/{id}/return`.
 *
 * Top-level in both modes: the payer got here by a full-page redirect from
 * the rail, so there is no parent to talk to and `window.location.assign`
 * is the forward.
 */
'use client';

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';

import { translator, type Locale } from '../i18n/index';
import { BrowserCheckoutApi } from '../lib/api';
import { decideReturnEntry } from '../lib/entry';
import { forwardKindFor, forwardTarget } from '../lib/forward';
import { recallPublishableKey } from '../lib/link';
import { RETURN_INITIAL_STATE, ReturnController, type ReturnState } from '../lib/return';
import { AUTO_FORWARD_SECONDS } from './checkout-client';
import { ReturnView } from './return-view';

export interface ReturnClientProps {
  sessionId: string;
  apiBaseUrl: string;
  initialLocale: Locale;
}

export function ReturnClient(props: ReturnClientProps) {
  const [locale, setLocale] = useState<Locale>(props.initialLocale);
  const [state, setState] = useState<ReturnState>(RETURN_INITIAL_STATE);
  const [secondsLeft, setSecondsLeft] = useState<number | null>(null);
  const controllerRef = useRef<ReturnController | null>(null);

  useEffect(() => {
    document.documentElement.lang = locale;
  }, [locale]);

  useEffect(() => {
    const decision = decideReturnEntry({
      search: window.location.search,
      rememberedKey: recallPublishableKey(window.sessionStorage, props.sessionId),
    });
    if (decision.kind === 'error') {
      // REAL finding, same shape as `checkout-client.tsx`: the return trip's
      // token is read from the URL, which only the browser can do.
      // eslint-disable-next-line react-hooks/set-state-in-effect
      setState({ name: 'error', error: { code: decision.code } });
      return;
    }
    const controller = new ReturnController({
      sessionId: props.sessionId,
      credentials: { key: decision.key, returnToken: decision.returnToken },
      api: new BrowserCheckoutApi({ baseUrl: props.apiBaseUrl }),
      navigate: (url) => window.location.assign(url),
      channel: null,
    });
    controllerRef.current = controller;
    const unsubscribe = controller.subscribe(setState);
    setState(controller.state);
    void controller.start();
    return () => {
      unsubscribe();
      controllerRef.current = null;
    };
  }, [props.apiBaseUrl, props.sessionId]);

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

  useEffect(() => {
    if (state.name !== 'outcome' || destination === null) {
      // REAL finding: see the same countdown in `checkout-client.tsx`.
      // eslint-disable-next-line react-hooks/set-state-in-effect
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
    <ReturnView
      state={state}
      t={t}
      locale={locale}
      destination={destination}
      secondsLeft={secondsLeft}
      onContinue={onContinue}
      onLocaleChange={setLocale}
    />
  );
}
