/**
 * The client half of `/c/{id}/return`.
 *
 * Top-level in both modes: the payer got here by a full-page redirect from
 * the rail, so there is no parent to talk to and `window.location.assign`
 * is the forward.
 *
 * **No timer navigates**, for the same reason as the payment page: the
 * outcome screen has a button and nothing else. This page carried the same
 * five-second countdown until 2026-09-06.
 *
 * There is no page memory here either. The return trip has no form to
 * prefill, and a page that cannot confirm has nothing to remember.
 */
'use client';

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';

import type { Branding } from '../config/settings';
import { translator, type Locale } from '../i18n/index';
import { BrowserCheckoutApi } from '../lib/api';
import { decideReturnEntry } from '../lib/entry';
import { forwardKindFor, forwardTarget } from '../lib/forward';
import { recallPublishableKey } from '../lib/link';
import { RETURN_INITIAL_STATE, ReturnController, type ReturnState } from '../lib/return';
import { ReturnView } from './return-view';

export interface ReturnClientProps {
  sessionId: string;
  apiBaseUrl: string;
  initialLocale: Locale;
  /** `branding.yaml`, read at container start. The return page carries the same mark as the payment page. */
  branding: Branding;
}

export function ReturnClient(props: ReturnClientProps) {
  const [locale, setLocale] = useState<Locale>(props.initialLocale);
  const [state, setState] = useState<ReturnState>(RETURN_INITIAL_STATE);
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

  const onReturnToMerchant = useCallback(() => {
    if (destination !== null) {
      controllerRef.current?.forward(destination);
    }
  }, [destination]);

  const t = useMemo(() => translator(locale), [locale]);

  return (
    <ReturnView
      state={state}
      t={t}
      locale={locale}
      branding={props.branding}
      destination={destination}
      onReturnToMerchant={onReturnToMerchant}
      onLocaleChange={setLocale}
    />
  );
}
