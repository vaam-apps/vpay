/**
 * Objects shaped like `/dash/v1`'s, for this app's own tests.
 *
 * **Imported by test files and by nothing under `app/`.** These are not
 * fixtures a page renders and never become one: a dashboard screen showing
 * invented rows is the failure mode `CLAUDE.md` names third, and the way to
 * keep that true is for the invented rows to exist only where a test can see
 * them.
 *
 * Every id is deliberately, obviously not a real one (`pi_example_1`), so a
 * value that escaped into a screenshot or a log would be recognisable as
 * this file's.
 */
import type {
  ChargeSummary,
  PaymentDetail,
  PaymentIntentObject,
  TimelineEvent,
} from '../server/api';

/** A succeeded XAF intent. */
export const INTENT: PaymentIntentObject = {
  id: 'pi_example_1',
  object: 'payment_intent',
  amount: 5000,
  currency: 'xaf',
  status: 'succeeded',
  payment_method_types: ['mtn_momo', 'orange_money'],
  last_payment_error: null,
  description: 'One order',
  customer: null,
  created: 1_757_000_000,
  livemode: false,
};

/** A second intent, in a different state, for a two-row list. */
export const OTHER_INTENT: PaymentIntentObject = {
  ...INTENT,
  id: 'pi_example_2',
  status: 'processing',
  amount: 125_000,
  created: 1_757_000_600,
};

/**
 * A charge whose `payer_ref_masked` is `null` — which is every charge this
 * deployment has ever written, because nothing writes that column.
 */
export const CHARGE: ChargeSummary = {
  object: 'charge',
  id: 'ch_example_1',
  provider_code: 'mtn_momo',
  provider_reference_id: '00000000-0000-4000-8000-000000000001',
  provider_txn_id: 'MTN-EXAMPLE-1',
  state: 'settled',
  amount: 5000,
  currency: 'xaf',
  payer_ref_masked: null,
  failure_code: null,
  failure_raw: null,
  created: 1_757_000_100,
  updated: 1_757_000_200,
};

/** Two events about the intent and its charge. */
export const EVENTS: readonly TimelineEvent[] = [
  {
    object: 'event',
    id: 'evt_example_1',
    type: 'payment_intent.created',
    object_id: 'pi_example_1',
    created: 1_757_000_000,
  },
  {
    object: 'event',
    id: 'evt_example_2',
    type: 'payment_intent.succeeded',
    object_id: 'pi_example_1',
    created: 1_757_000_200,
  },
];

/** The whole detail envelope. */
export const DETAIL: PaymentDetail = {
  object: 'dashboard.payment_detail',
  payment_intent: INTENT,
  charge: CHARGE,
  refunds: [],
  events: EVENTS,
};
