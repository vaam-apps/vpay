/**
 * One literal value per screen the page can show.
 *
 * Shared by the rendering tests and by the Storybook stories, so the states
 * a designer looks at and the states the tests assert on are the same list —
 * and a screen added without being added here is a screen with neither.
 *
 * It lives under `src/testing/` rather than beside the components it
 * describes because it is built out of `./fixtures` and is imported by a
 * `.test.tsx` and a `.stories.tsx` and by nothing else. While it sat in
 * `src/components/` the one sentence stating that invariant
 * (`fixtures.ts`: "nothing under `src/testing` is imported from `app/` or
 * from any component") was false as written, and nothing checked it either
 * way. `src/testing/no-runtime-imports.test.ts` checks it now.
 */
import type { CheckoutState } from '../lib/machine';
import type { ReturnState } from '../lib/return';
import { makeContext, makePublicIntent, makeSession } from './fixtures';

const BOTH_RAILS = { payment_method_types: ['mtn_momo', 'orange_money'] };
const MTN = {
  code: 'mtn_momo',
  flow: 'mobile_money_push',
  label: 'rail.mtn_momo',
} as const;
const ORANGE = {
  code: 'orange_money',
  flow: 'redirect',
  label: 'rail.orange_money',
} as const;
const RAILS = {
  supported: [MTN, ORANGE],
  unsupported: [],
} as const;

export const CHECKOUT_SCREENS: Record<string, CheckoutState> = {
  loading: { name: 'loading' },
  error: { name: 'error', error: { code: 'error.session_not_found' } },
  refused_embed: { name: 'refused', reason: 'embed_not_allowed', context: null },
  refused_rail: {
    name: 'refused',
    reason: 'no_supported_rail',
    context: makeContext({}, { payment_method_types: ['zzz_pay'] }),
  },
  expired: { name: 'expired', context: makeContext({ status: 'expired' }) },
  select_rail: {
    name: 'select_rail',
    context: makeContext({}, BOTH_RAILS),
    rails: { supported: [...RAILS.supported], unsupported: ['zzz_pay'] },
  },
  collect_msisdn: {
    name: 'collect_msisdn',
    context: makeContext({}, BOTH_RAILS),
    rails: { supported: [...RAILS.supported], unsupported: [] },
    rail: MTN,
    problem: null,
  },
  collect_msisdn_invalid: {
    name: 'collect_msisdn',
    context: makeContext({}, BOTH_RAILS),
    rails: { supported: [...RAILS.supported], unsupported: [] },
    rail: MTN,
    problem: 'msisdn.invalid',
  },
  ready_redirect: {
    name: 'ready_redirect',
    context: makeContext({}, BOTH_RAILS),
    rails: { supported: [...RAILS.supported], unsupported: [] },
    rail: ORANGE,
    problem: null,
  },
  confirming: { name: 'confirming', context: makeContext(), rail: MTN },
  waiting: { name: 'waiting', context: makeContext({}, { status: 'processing' }), rail: MTN, notice: null },
  waiting_notice: {
    name: 'waiting',
    context: makeContext({}, { status: 'processing' }),
    rail: MTN,
    notice: 'error.network',
  },
  redirecting: {
    name: 'redirecting',
    context: makeContext({}, BOTH_RAILS),
    rail: ORANGE,
    url: 'https://rail.example/stub-hosted-page/tok_1',
  },
  outcome_succeeded: {
    name: 'outcome',
    context: makeContext({ status: 'complete', payment_status: 'paid' }, { status: 'succeeded' }),
    kind: 'succeeded',
    failure: null,
  },
  outcome_failed: {
    name: 'outcome',
    context: makeContext(
      { status: 'expired', payment_status: 'failed' },
      {
        status: 'requires_payment_method',
        last_payment_error: { code: 'insufficient_funds', message: 'x' },
      },
    ),
    kind: 'failed',
    failure: 'insufficient_funds',
  },
  outcome_canceled: {
    name: 'outcome',
    context: makeContext({ status: 'expired', payment_status: 'failed' }, { status: 'canceled' }),
    kind: 'canceled',
    failure: null,
  },
  forwarding: {
    name: 'forwarding',
    context: makeContext({ status: 'complete', payment_status: 'paid' }, { status: 'succeeded' }),
    kind: 'succeeded',
    url: 'https://shop.example/ok?sid=cs_test_fixture000000000001',
  },
};

export const RETURN_SCREENS: Record<string, ReturnState> = {
  loading: { name: 'loading' },
  error: { name: 'error', error: { code: 'error.missing_return_token' } },
  polling: {
    name: 'polling',
    context: {
      session: makeSession(),
      intent: makePublicIntent({ status: 'requires_action' }),
      merchant: { name: 'Boutique Test' },
    },
    notice: null,
  },
  expired: {
    name: 'expired',
    context: {
      session: makeSession({ status: 'expired' }),
      intent: makePublicIntent(),
      merchant: { name: 'Boutique Test' },
    },
  },
  outcome_succeeded: {
    name: 'outcome',
    context: {
      session: makeSession({ status: 'complete', payment_status: 'paid' }),
      intent: makePublicIntent({ status: 'succeeded' }),
      merchant: { name: 'Boutique Test' },
    },
    kind: 'succeeded',
    failure: null,
  },
  outcome_failed: {
    name: 'outcome',
    context: {
      session: makeSession({ status: 'expired', payment_status: 'failed' }),
      intent: makePublicIntent({
        status: 'requires_payment_method',
        last_payment_error: { code: 'payer_timeout', message: 'x' },
      }),
      merchant: { name: 'Boutique Test' },
    },
    kind: 'failed',
    failure: 'payer_timeout',
  },
};
