/**
 * Wire types for vpay's `/v1` resource contract
 * (docs/api/README.md, docs/flows/merchant-auth.md's "Objects" table).
 */

/**
 * The five-value state machine `vpay_core::state::IntentStatus` defines
 * (docs/flows/payment-lifecycle.md). There is deliberately no `failed`
 * status — a rail failure returns the intent to
 * `requires_payment_method` with {@link LastPaymentError} populated.
 */
export type PaymentIntentStatus =
  | "requires_payment_method"
  | "requires_action"
  | "processing"
  | "succeeded"
  | "canceled";

/** Rail codes this SDK's `payment_method_types` values may hold. */
export type PaymentMethodType = "mtn_momo" | "orange_money";

/** Redirect-rail next step, Stripe's own `next_action.redirect_to_url` shape. */
export interface NextAction {
  type: "redirect_to_url";
  redirect_to_url: {
    url: string;
    return_url: string | null;
  };
}

/**
 * The closed failure-code vocabulary `vpay_core::failure` owns
 * (docs/flows/failures.md). Adapters map rail-specific error strings into
 * this list; merchants integrate against it once.
 *
 * **Not every rail can produce every code.** The list is the *core's*, and it
 * does not shrink when a rail is added — but a merchant branching on all
 * eleven should know that on the two MVP rails, MTN MoMo reaches all of them
 * and Orange Money reaches three (`payer_timeout`,
 * `provider_account_blocked`, `provider_error`). Orange documents five
 * statuses and no sub-reason for `FAILED`, so its protocol has no way to say
 * "not enough funds"; a payer who cancels on its hosted page arrives as
 * `payer_timeout`, because the rail does not distinguish that from an
 * abandoned page. The table, code by code with the rail reason that produces
 * it and the conformance case that proves it, is
 * `docs/flows/failures.md` § "Which rail can produce which code".
 *
 * `payer_declined` in particular was produced by **no** adapter until
 * 2026-09-10 (vpay issue #59), while being typed here and given buyer copy by
 * `examples/shop`. It is now MTN's `PAYMENT_NOT_APPROVED` /
 * `APPROVAL_REJECTED`.
 */
export type FailureCode =
  | "insufficient_funds"
  | "payer_timeout"
  | "payer_declined"
  | "invalid_payer"
  | "payer_limit_reached"
  | "payer_account_blocked"
  | "invalid_payee"
  | "payee_account_blocked"
  | "provider_account_blocked"
  | "provider_unavailable"
  | "provider_error";

export interface LastPaymentError {
  code: FailureCode;
  message: string;
}

export interface PaymentIntent {
  id: string;
  object: "payment_intent";
  /** Integer minor units. XAF is zero-decimal: 5000 means 5,000 FCFA. */
  amount: number;
  /** Lowercase ISO 4217 code, e.g. `xaf`. */
  currency: string;
  status: PaymentIntentStatus;
  payment_method_types: string[];
  next_action: NextAction | null;
  last_payment_error: LastPaymentError | null;
  metadata: Record<string, string>;
  description: string | null;
  /**
   * The `cus_…` this intent is for, or `null` (S4a).
   *
   * The **id**, never the expanded object: vpay does not implement `expand`,
   * and rendering the customer unasked would put a payer's name, email and
   * phone number into every `payment_intent.*` webhook body. Read it with
   * `client.customers.retrieve`.
   *
   * `?` because a vpay predating 2026-09-06 omits the key entirely — the
   * same reason `Refund.fee` carries one, and with a *weaker* consequence:
   * unlike `fee`, `undefined` and `null` mean the same thing here ("no
   * customer"), so nothing is lost by conflating them.
   */
  customer?: string | null;
  /** Unix seconds. */
  created: number;
  livemode: boolean;
  /**
   * `pi_…_secret_…` — the payer credential `/v1/browser` accepts to confirm
   * this intent from a browser (hand it to `@vaam-apps/vpay-stripe-js`).
   *
   * Present **only** on `create()` and `retrieve()` responses (Step 5c's
   * D2, `vpay_api::model::PaymentIntentWithSecret`); absent — this property
   * is missing entirely, not `null` — on every `list()` item and on
   * `event.data.object`, so it never reaches a merchant's listing view or a
   * webhook body that's stored and forwarded at-least-once. Never log this
   * value.
   */
  client_secret?: string;
}

export type RefundStatus = "pending" | "succeeded" | "failed" | "canceled";

export interface Refund {
  id: string;
  object: "refund";
  /**
   * Integer minor units of {@link Refund.currency}. **The payer's money** —
   * never net of {@link Refund.fee}, which is a separate cost with a
   * different owner.
   */
  amount: number;
  currency: string;
  payment_intent: string;
  status: RefundStatus;
  reason: string | null;
  metadata: Record<string, string>;
  created: number;
  /**
   * What the rail charged to execute this refund, in minor units of
   * {@link Refund.currency} — never a second currency, never a float.
   *
   * **Three answers, and they are not interchangeable.** `0` means the rail
   * reported the movement was free. `null` means it reported no fee at all.
   * `undefined` — the key absent — means the vpay that answered predates the
   * field (added by issue #46). The last two are both *unknown*; only `0` is
   * a measured zero, and substituting it for either is how an invented number
   * reaches a merchant's settlement statement.
   *
   * Optional **and** `| null` on purpose: it is the `?` that puts
   * `undefined` in the read type, so `Refund["fee"]` is exactly
   * `number | null | undefined` and all three answers survive. That is
   * pinned by a type-level assertion — see `types fee so that absent, null
   * and a measured zero stay three different answers` in `types.test.ts`,
   * which fails if either the `?` or the `| null` is dropped.
   *
   * This package's `exactOptionalPropertyTypes` is **not** what creates the
   * distinction, although an earlier version of this comment said it was.
   * Measured 2026-09-05 with the repo's own `tsc`: the read type is
   * `number | null | undefined` with the flag and without it, and an absent
   * property never reads as `null` in either. What the flag actually adds is
   * narrower — `{ fee: undefined }` stops being a legal way to spell
   * "absent".
   *
   * Narrow with `typeof refund.fee === "number"`; `refund.fee ?? 0` and
   * `refund.fee || 0` are both the bug.
   *
   * **It is `null` from every vpay deployment today.** Neither rail reports a
   * refund fee — Orange has no refund API and MTN refunds are the
   * Disbursements product vpay has never called — so nothing populates it.
   * See `docs/status.md`.
   */
  fee?: number | null;
}

/**
 * The real Stripe event types docs/flows/webhooks.md commits to. A custom
 * type would be silently dropped by a merchant's exhaustive `switch` over
 * Stripe's own typed union — this SDK types `type` as `string` rather than
 * this literal union so an event carrying a type it does not yet know about
 * is still deliverable, and narrows with the exported guards below instead.
 *
 * `checkout.session.expired` is emitted when vpay's hourly sweep moves a
 * session past its 24-hour horizon (Step 9's D10); its `data.object` is a
 * {@link CheckoutSession}, which makes it the only member here whose payload
 * is neither a {@link PaymentIntent} nor a {@link Refund}. Narrow it with
 * {@link isCheckoutSessionEvent}.
 */
export type KnownEventType =
  | "payment_intent.created"
  | "payment_intent.processing"
  | "payment_intent.succeeded"
  | "payment_intent.payment_failed"
  | "payment_intent.canceled"
  | "charge.refunded"
  | "charge.refund.updated"
  | "checkout.session.expired"
  /**
   * A Customer was created by `POST /v1/customers`. `data.object` is the
   * {@link Customer} as the insert stored it — the canonical `phone`, and
   * `metadata` as sent. Written in the same transaction as the row (vpay
   * issue #66, 2026-09-10).
   */
  | "customer.created"
  /**
   * A Customer was changed by `POST /v1/customers/{id}`. `data.object` is the
   * {@link Customer} **after** the change, including the key-wise `metadata`
   * merge — which vpay computes under the row's lock, so two concurrent
   * updates produce two events describing the two committed states rather
   * than one describing a merge that was lost.
   *
   * A request that changes nothing — a bodiless `POST`, which is what "touch
   * this object" is on the wire — emits **no** event.
   */
  | "customer.updated"
  /**
   * A Customer was deleted — by `DELETE /v1/customers/{id}` or by vpay's
   * twelve-month retention sweep (S4a). `data.object` is a
   * {@link Customer} as it stood immediately before the delete, which is the
   * **only** way to learn its `name`, `email` and `phone`: the row is gone,
   * and a `GET` afterwards is byte-identical to one for an id that never
   * existed.
   */
  | "customer.deleted"
  /**
   * A draft invoice was created by `POST /v1/invoices` (S4b). The insert and
   * this event are one transaction, so an event you received describes a row
   * that committed. `data.object` is an {@link Invoice}.
   */
  | "invoice.created"
  /**
   * A draft was issued: it has its {@link Invoice.number}, its lines are
   * frozen and its amounts are final. `data.object` is an {@link Invoice} in
   * `"open"`.
   */
  | "invoice.finalized"
  /**
   * A settlement paid an invoice in full. `data.object` is an
   * {@link Invoice} in `"paid"`.
   *
   * Emitted inside the settlement transaction itself, **beside** the
   * `payment_intent.succeeded` for the same money — a handler receives both
   * and must not read them as two payments.
   */
  | "invoice.paid"
  /**
   * The merchant cancelled an issued invoice. `data.object` is an
   * {@link Invoice} in `"void"`, **keeping** its number.
   *
   * There is deliberately no `invoice.marked_uncollectible` beside this one:
   * vpay does not write it, so a union entry would be a claim about vpay that
   * is false. A write-off is learned from
   * `client.invoices.list({ status: "uncollectible" })`. The same goes for
   * Stripe's `invoice.payment_failed`, which a merchant meets as
   * `payment_intent.payment_failed`.
   */
  | "invoice.voided";

export interface Event {
  id: string;
  object: "event";
  type: string;
  /** Unix seconds. */
  created: number;
  livemode: boolean;
  /**
   * Kept as raw JSON with typed accessors below, rather than a typed union,
   * so an event carrying an object shape this SDK does not model is still
   * deliverable (docs/flows/merchant-auth.md, "Objects").
   */
  data: { object: unknown };
}

/** Narrows a {@link PaymentIntent}-shaped value out of `event.data.object`. */
export function isPaymentIntentEvent(
  event: Event,
): event is Event & { data: { object: PaymentIntent } } {
  return event.type.startsWith("payment_intent.");
}

/** Narrows a {@link Refund}-shaped value out of `event.data.object`. */
export function isRefundEvent(
  event: Event,
): event is Event & { data: { object: Refund } } {
  return event.type.startsWith("charge.refund");
}

/**
 * Narrows a {@link CheckoutSession}-shaped value out of `event.data.object`.
 *
 * Matched on the `checkout.session.` prefix rather than on the one literal
 * type vpay emits today, exactly as the two guards above are: a later
 * `checkout.session.*` type carries the same object, and a guard that had to
 * be edited for each one is a guard that will be out of date before the
 * `switch` that calls it is.
 *
 * The session on an event **never** carries `client_secret`, and its `url` is
 * always `null`: both would put a live payer credential in a body that is
 * stored, delivered at-least-once and replayed
 * (`vpay_api::model::CheckoutSessionObject::expired_snapshot`). So
 * {@link CheckoutSession.url} being `null` here does not mean the session was
 * embedded — read {@link CheckoutSession.ui_mode} for that.
 */
export function isCheckoutSessionEvent(
  event: Event,
): event is Event & { data: { object: CheckoutSession } } {
  return event.type.startsWith("checkout.session.");
}

/**
 * Narrows an {@link Invoice}-shaped value out of `event.data.object`, for one
 * of the four `invoice.*` events (S4b).
 *
 * Matched on the `invoice.` prefix rather than on the four literals, as every
 * guard above is.
 *
 * **The invoice on an event carries no lines.** `data.object.lines.data` is
 * empty on every `invoice.*` body — the event is rendered inside the
 * transition's own transaction, where the lines are not read — so
 * {@link Invoice.lines} here is an empty {@link List}, and that is not the
 * same statement as "this invoice has no lines". A handler that needs them
 * calls `client.invoices.retrieve`, whose response always carries them.
 */
export function isInvoiceEvent(
  event: Event,
): event is Event & { data: { object: Invoice } } {
  return event.type.startsWith("invoice.");
}

/**
 * A Checkout Session's lifecycle state (Step 9's D10) — vpay's own three
 * values, not Stripe's. There is no `failed`: a session whose intent
 * reached a terminal non-success state is `expired` with
 * {@link CheckoutSession.payment_status} `failed`.
 */
export type CheckoutSessionStatus = "open" | "complete" | "expired";

/** Whether the session's intent has been paid. D10's second axis. */
export type CheckoutPaymentStatus = "unpaid" | "paid" | "failed";

/**
 * Which surface the session is rendered on. `hosted` gets a
 * {@link CheckoutSession.url} to send the payer to; `embedded` gets a
 * `client_secret` to hand `@vaam-apps/vpay-stripe-js`'s `initEmbeddedCheckout`.
 */
export type CheckoutSessionUiMode = "hosted" | "embedded";

/**
 * A `/v1/checkout/sessions` object (Step 9's D1).
 *
 * The session **references** an existing PaymentIntent; it never creates
 * one. Amount, currency and the allowed rails stay on the intent, where
 * every existing invariant already guards them — which is why there is no
 * `line_items`, no `mode` and no `amount_total` here, however Stripe-shaped
 * the rest of the field names are.
 */
export interface CheckoutSession {
  /** `cs_…`. */
  id: string;
  object: "checkout.session";
  livemode: boolean;
  /**
   * The `pi_…` this session drives — an **id**, on every `/v1` route.
   *
   * `@vaam-apps/vpay-stripe-js`'s `CheckoutSession` types the same field as the whole
   * expanded {@link PaymentIntent}, because the *browser* session read
   * expands it (the checkout page confirms and polls the intent through the
   * browser routes and cannot fetch it separately). That is a deliberate
   * per-route difference, ruled on 2026-09-04, not a skew between the
   * SDKs: nothing on `/v1` ever expands it, so a merchant integration reads
   * an id here and calls `paymentIntents.retrieve` if it wants the object.
   */
  payment_intent: string;
  ui_mode: CheckoutSessionUiMode;
  status: CheckoutSessionStatus;
  payment_status: CheckoutPaymentStatus;
  /** Hosted mode only; `null` when embedded. May carry `{CHECKOUT_SESSION_ID}` (D5). */
  success_url: string | null;
  /** Hosted mode only; `null` when embedded. */
  cancel_url: string | null;
  /** Embedded mode only; `null` when hosted. */
  return_url: string | null;
  /** The page to send the payer to, hosted mode only; `null` when embedded. */
  url: string | null;
  /**
   * The `cus_…` this session is for, or `null` (S4a). Copied from the
   * session's intent at create when the intent has one. The id, never the
   * expanded object — {@link PaymentIntent.customer}'s reason, unchanged.
   */
  customer?: string | null;
  /** Unix seconds. 24 h from create. */
  expires_at: number;
  /** Unix seconds. */
  created: number;
  /**
   * `cs_…_secret_…` — the payer credential the browser presents to read
   * this session, and what `initEmbeddedCheckout`'s `fetchClientSecret`
   * must return.
   *
   * Present **only** on `create()` and `retrieve()` responses; absent —
   * missing entirely, not `null` — on every `list()` item. It is a
   * different credential from the intent's `client_secret`: it authorises
   * reading this session, never confirming the intent.
   *
   * Never log this value. Unlike {@link PaymentIntent}, whose plain
   * interface prints its secret through `console.log`, every
   * `CheckoutSession` this SDK returns carries a custom
   * `util.inspect` representation that redacts this field — see
   * `src/resources/checkout-sessions.ts`. `JSON.stringify` is deliberately
   * left faithful: an embedded integration has to serialise the secret to
   * get it to the browser at all.
   */
  client_secret?: string;
}

export interface BalanceAmount {
  amount: number;
  currency: string;
}

export interface Balance {
  object: "balance";
  available: BalanceAmount[];
  pending: BalanceAmount[];
}

/**
 * Who a mobile-money number is registered to, or the fact that the rail has
 * no record of it — `GET /v1/account_holders` (issue #47).
 *
 * `name: null` means **the rail answered and does not know this number**. It
 * does not mean "we could not ask": that throws. A caller matching a
 * nominated refund destination against a buyer's verified name must refuse
 * on both — but only one of them is the buyer's problem.
 *
 * A name and nothing else, by construction: the rail's body carries more (a
 * birthdate, a locale, a gender) and vpay projects it away before it can be
 * logged or stored. There is no `id`, because nothing is stored to address
 * later.
 */
export interface AccountHolder {
  object: "account_holder";
  /**
   * The rail the question was put to, echoed back.
   *
   * `string` and not {@link PaymentMethodType}, for the reason
   * `PaymentIntent.payment_method_types` is `string[]`: the request side is
   * closed and the response side must stay readable for a rail this SDK
   * version predates.
   */
  payment_method_type: string;
  /** The registered holder's name, or `null` when the rail has no record. */
  name: string | null;
  /**
   * `true` exactly when `name` is present.
   *
   * Not a claim that anything was cryptographically verified — it says the
   * rail named a holder.
   */
  verified: boolean;
}

/**
 * `GET /v1/account_holders` request fields.
 *
 * A `type` alias for {@link ListParams}' index-signature reason. Both fields
 * are required and neither is optional: a lookup with no number or no rail
 * is not a narrower query, it is not a query.
 *
 * **`payment_method_type`, snake_case, like every other params type in this
 * package** — the wire spelling, which is what the encoder walks and what
 * `sdks/rust/tests/resources.rs` pins byte for byte. A camelCase
 * `paymentMethodType` would be the only camelCase request field in the SDK
 * and would need a translation step the others do not have.
 */
export type RetrieveAccountHolderParams = {
  /**
   * The mobile-money number, in any of the three shapes vpay accepts:
   * `+2376XXXXXXXX`, `2376XXXXXXXX` or the national `6XXXXXXXX`. Validated
   * by the server, not here — see `AccountHoldersResource.retrieve`.
   */
  msisdn: string;
  /** Which rail to ask. Closed on the request side, as a create's list is. */
  payment_method_type: PaymentMethodType;
};

export interface List<T> {
  object: "list";
  data: T[];
  has_more: boolean;
  url: string;
}

/**
 * A `type` alias, not an `interface`, on purpose: TypeScript gives an
 * anonymous object type an *implicit index signature* and an interface none,
 * so only this form is assignable to the encoder's
 * `Record<string, FormValue>` without an `as unknown as` cast.
 *
 * Every optional property is written `?: T | undefined` rather than `?: T`,
 * so that a consumer compiling with `exactOptionalPropertyTypes` (as this
 * repo does) can pass a variable that is legitimately `T | undefined`.
 */
export type ListParams = {
  limit?: number | undefined;
  starting_after?: string | undefined;
  ending_before?: string | undefined;
};

/**
 * A postal address on a {@link Customer} — Stripe's six formal components
 * **and** the GPS point (issue #67).
 *
 * # vpay's address is both halves, and this is where Stripe's is not
 *
 * `latitude_microdeg` and `longitude_microdeg` have no counterpart on
 * Stripe's `address`. They are here because vpay's address *means* both
 * halves (the maintainer, 2026-09-11): formal addressing is unreliable
 * across the markets vpay serves, and a coordinate is how a place is actually
 * found. A merchant porting Stripe code to vpay gains two fields; one porting
 * the other way loses them, and should know that before they discover it.
 *
 * Every component is nullable and the server renders all eight even when they
 * are `null`: an address is one fact about a payer, and a key that appeared
 * and disappeared would change the shape of a signed `customer.*` webhook
 * body per payer. The object as a whole is `null` when there is no address.
 *
 * The same shape is sent and received. On {@link UpdateCustomerParams} an
 * address **replaces** the stored one rather than merging with it — a request
 * that names `line1` and not `city` clears the city — because an address
 * assembled by the server out of two requests is an address that was never
 * anybody's. Sending `null` removes it entirely.
 */
export interface Address {
  line1: string | null;
  line2: string | null;
  city: string | null;
  state: string | null;
  postal_code: string | null;
  /**
   * ISO 3166-1 alpha-2 — `CM`, `FR`, `NG`. Lower case is accepted and stored
   * **upper case**, so what comes back may not be what was sent: the same
   * wire contract {@link Customer.phone}'s canonicalisation is. A
   * three-letter code or a country name is a `400` naming `address`.
   */
  country: string | null;
  /**
   * Latitude in **microdegrees** — millionths of a degree, so 4.061°N is
   * `4061000`.
   *
   * A whole number, always. vpay has no floating-point coordinate anywhere —
   * not on the wire, not in its database — which is what the unit in the
   * field name is for: a field called `latitude` would be read as degrees,
   * and `4.061` is a value that cannot be stored without rounding it. The
   * server answers `400` naming `address` for a decimal rather than
   * approximating it.
   *
   * `null` unless {@link Address.longitude_microdeg} is also set: half a
   * coordinate names no place, and the server refuses one half.
   *
   * On an **erased** customer this is `null` while the six formal components
   * are `[redacted]`. That asymmetry is deliberate: there is no integer that
   * is not a possible place, so the marker is not a value this field can
   * take. See {@link Customer.deleted}.
   */
  latitude_microdeg: number | null;
  /** Longitude in **microdegrees**. See {@link Address.latitude_microdeg}. */
  longitude_microdeg: number | null;
}

/**
 * A `customer` — the merchant-owned record of a payer they expect to see
 * again (S4a).
 *
 * Nine keys, plus `deleted` on an erased one. `last_used_at` — the clock
 * vpay's twelve-month retention sweep reads — is deliberately **not** on the
 * wire.
 *
 * At least one of `name`, `email` and `phone` is always present. A phone
 * number **alone** is a complete customer, which is what the object is for
 * on a mobile money rail.
 */
export interface Customer {
  id: string;
  object: "customer";
  name: string | null;
  email: string | null;
  /**
   * Echoed back **canonicalised** (`2376XXXXXXXX`, no `+`) rather than as it
   * was sent, which is a wire contract and not a quirk: it is the value a
   * rail is given, so a merchant comparing this against a charge's payer
   * reference is comparing the same string.
   */
  phone: string | null;
  /**
   * The payer's postal address **and** GPS point, or `null`. Two of its keys
   * have no counterpart on Stripe's address — see {@link Address}.
   */
  address: Address | null;
  metadata: Record<string, string>;
  /** Unix seconds. */
  created: number;
  livemode: boolean;
  /**
   * `true` on a customer vpay has **erased**, and absent on a live one — the
   * server omits the key rather than sending `false`.
   *
   * `DELETE /v1/customers/{id}` removes a customer with no payment history
   * outright, and a later retrieve is a `404`. A customer an intent, a
   * checkout session or an invoice references cannot be removed — vpay never
   * detaches a payment from the payer it was taken from — so it is
   * **anonymised** instead: the row and the payment record stay, and every
   * identifier on it comes back as `[redacted]`. `metadata` is untouched,
   * because that is the merchant's own data.
   *
   * So a `cus_…` in your own records goes on resolving after an erasure,
   * which is why this key exists. Such a customer cannot be updated or
   * attached to a new payment — both answer `409`. See
   * `docs/flows/customers.md`.
   */
  deleted?: true | undefined;
}

/**
 * What `client.customers.del(…)` answers with — Stripe's deleted-object
 * shape.
 *
 * Deliberately **not** the customer that was removed: a merchant confirming
 * an erasure is the caller most likely to log the whole response, and
 * returning the payer's details in it would write them into a log *because*
 * they were deleted.
 */
export interface DeletedCustomer {
  id: string;
  object: "customer";
  deleted: true;
}

export interface CreateCustomerParams {
  /**
   * At least one of `name`, `email` and `phone` must be present, and this
   * SDK deliberately does not check that: which identifiers a customer needs
   * is a *market* rule vpay owns and may widen — the same line
   * `RetrieveAccountHolderParams.msisdn` takes on the MSISDN — so a copy here
   * would refuse offline a customer a later server version accepts. The
   * server answers `400` naming the parameter. `sdks/rust` takes the
   * identical line.
   */
  name?: string | undefined;
  email?: string | undefined;
  /**
   * In any spelling vpay's canonicaliser accepts (`+237 6 …`,
   * `237600000200`, `600000200`). Stored and echoed back **canonical**, so
   * what comes back may not be what was sent — see {@link Customer.phone}.
   */
  phone?: string | undefined;
  /**
   * The payer's postal address, sent as `address[line1]=…`. An address alone
   * does not name anybody, so it does not satisfy the one-of rule above: a
   * create carrying only an address is refused exactly as one carrying
   * nothing is.
   */
  address?: AddressParams | undefined;
  metadata?: Record<string, string> | undefined;
}

/**
 * The address components, in the shape the wire spells them — Stripe's six
 * **and** the GPS point.
 *
 * A `type` alias rather than an `interface` for {@link ListParams}' reason:
 * only this form is assignable to the form encoder's
 * `Record<string, FormValue>` without a cast.
 *
 * Separate from {@link Address} — which the *response* carries — because what
 * a merchant may send and what the server returns are two contracts, and the
 * second one grows keys the first must not accept. They happen to have the
 * same eight fields today. This SDK deliberately does not validate `country`
 * locally, exactly as it does not validate an MSISDN: which codes vpay
 * accepts is a rule vpay owns and may widen. The same line is taken on the
 * coordinate's range and its pair rule.
 */
export type AddressParams = {
  line1?: string | undefined;
  line2?: string | undefined;
  city?: string | undefined;
  state?: string | undefined;
  postal_code?: string | undefined;
  country?: string | undefined;
  /**
   * Latitude in **microdegrees** — `4061000` for 4.061°N, never `4.061`. The
   * range (±90,000,000) and the rule that it is sent with
   * {@link AddressParams.longitude_microdeg} or not at all are the server's,
   * and are not checked here for the reason `country` is not: they are
   * vpay's to widen, and a copy in this SDK would refuse offline an address a
   * later server accepts.
   */
  latitude_microdeg?: number | undefined;
  /** Longitude in **microdegrees**. See {@link AddressParams.latitude_microdeg}. */
  longitude_microdeg?: number | undefined;
};

/**
 * `POST /v1/customers/{id}` request fields (S4a).
 *
 * # Why the three scalars are `string | null | undefined`
 *
 * An update has three answers per field and a merchant depends on all three:
 * *leave it alone* (`undefined`, or the key absent), *set it* (a string), and
 * **clear it** (`null`, which this SDK sends as `name=`). A `string |
 * undefined` field collapses the first and the third, and a payer's email
 * would then be unclearable through the API that documents how to clear it.
 *
 * It is the same three-state shape `UpdateCustomerParams` carries in
 * `sdks/rust` (`Option<Option<String>>`) and `vpay_db::CustomerPatch` carries
 * on the server, so all three layers express the distinction rather than two
 * of them preserving it and one losing it.
 *
 * `metadata` has two states rather than three, and that is the wire contract:
 * it is **merged** key-wise by the server, and a key whose value is the empty
 * string is removed. So there is no separate "clear all metadata" — send each
 * key empty.
 */
export interface UpdateCustomerParams {
  name?: string | null | undefined;
  email?: string | null | undefined;
  phone?: string | null | undefined;
  /**
   * The address, in the same three states — with one difference worth
   * stating, because it is the one a merchant can get wrong silently.
   *
   * `undefined` leaves the stored address alone. `null` removes it (sent as
   * `address=`). An object **replaces** it whole: every component the request
   * does not name is cleared, so correcting a street means sending the city
   * again. vpay does not merge components, deliberately — an address
   * assembled out of two requests is an address that was never anybody's, and
   * its failure mode is a plausible wrong address rather than a visible one.
   */
  address?: AddressParams | null | undefined;
  metadata?: Record<string, string> | undefined;
}

/**
 * `GET /v1/customers` query parameters.
 *
 * There is no `email` filter, and that is the server's shape rather than an
 * omission here: a filter on a payer identifier turns the list into a lookup.
 * See `docs/flows/customers.md`.
 */
export type ListCustomersParams = {
  limit?: number | undefined;
  starting_after?: string | undefined;
  ending_before?: string | undefined;
};

/**
 * An invoice's lifecycle state (S4b).
 *
 * Exactly the five labels the server's `invoices_status_enum_check` allows,
 * and the transitions between them are the server's: `draft → open → paid |
 * void | uncollectible`. A closed union rather than `string`, and
 * `sdks/rust` spells the same five as a closed `enum` — a sixth label is a
 * schema migration and a breaking wire change, not something a merchant's
 * deployment can meet by surprise. That is the opposite of the split
 * {@link KnownEventType} makes, and deliberately so.
 *
 * `"uncollectible"` is the one a merchant filters on: a write-off emits **no
 * event**, so `invoices.list({ status: "uncollectible" })` is how they are
 * found.
 */
export type InvoiceStatus =
  "draft" | "open" | "paid" | "void" | "uncollectible";

/**
 * One line on an invoice — the `line_item` object.
 *
 * # Why the type is `InvoiceLine` and the resource is `client.invoiceItems`
 *
 * The wire has one object under two names and both are load-bearing: the
 * `object` field is `"line_item"` and the route that creates, reads, changes
 * and removes it is `/v1/invoice_items`. Stripe has two objects there
 * (`invoiceitem` and `line_item`); vpay has one. This type is named for the
 * object it decodes and the resource for the route it calls, so neither name
 * is invented — `sdks/rust` spells them `InvoiceLine` and
 * `client.invoice_items()` for the same reason.
 *
 * No price, no product, no proration and no period: a line is a description,
 * a quantity and a unit amount.
 */
export interface InvoiceLine {
  /** `ii_…` — the id `/v1/invoice_items/{id}` addresses. */
  id: string;
  /** Always `"line_item"`, and **not** `"invoice_item"`. See above. */
  object: "line_item";
  description: string;
  quantity: number;
  /** The price of one, in integer minor units. */
  unit_amount: number;
  /**
   * `quantity * unit_amount`, in integer minor units.
   *
   * Computed and checked by the database, never sent: `amount` is not a
   * parameter on {@link CreateInvoiceItemParams} or
   * {@link UpdateInvoiceItemParams}, ever.
   */
  amount: number;
  /** Lower-case ISO-4217, always the parent invoice's. */
  currency: string;
  livemode: boolean;
}

/**
 * When each of an invoice's transitions happened, in unix **seconds**.
 *
 * Every field is `null` until the transition happens, and at most two are
 * ever non-null — `finalized_at` plus whichever terminal one applies.
 */
export interface InvoiceStatusTransitions {
  finalized_at: number | null;
  paid_at: number | null;
  voided_at: number | null;
  marked_uncollectible_at: number | null;
}

/**
 * An `invoice` (S4b): a merchant's bill to one customer. **Eighteen keys.**
 *
 * # `lines` is always expanded, and always unpaged
 *
 * vpay does not implement Stripe's `expand[]` at all, and an invoice without
 * its lines is a total with no explanation — so `lines` arrives populated on
 * every `/v1/invoices` response, with `has_more` always `false`. Its `url` is
 * `/v1/invoice_items`, a route that exists, rather than Stripe's
 * `/v1/invoices/{id}/lines`, which vpay does not serve.
 *
 * **Inside an `invoice.*` webhook body `lines.data` is empty**, and that is
 * the one place the shape differs: the event is rendered inside the
 * transition's own transaction. Re-read the invoice with
 * `client.invoices.retrieve` if a handler needs the lines.
 *
 * # `hosted_invoice_url` is a checkout session, not an invoice page
 *
 * vpay has no invoice page. A payer following it meets the existing hosted
 * checkout, and it is `null` until `client.invoices.pay` has run.
 */
export interface Invoice {
  /** `in_…`. The id an API call uses; {@link Invoice.number} is the one a human quotes. */
  id: string;
  object: "invoice";
  /**
   * The `cus_…` this invoice bills.
   *
   * `string` and not `string | null`, unlike {@link PaymentIntent.customer},
   * and that is the wire contract rather than an assumption: the column is
   * `NOT NULL`, because an invoice naming no payer is one nobody can be asked
   * to pay.
   */
  customer: string;
  currency: string;
  status: InvoiceStatus;
  /**
   * `{prefix}-{000001}` from this merchant's own sequence, or `null`
   * **exactly** while the invoice is a `"draft"`.
   *
   * Assigned at finalize, never reused, and kept by a voided invoice.
   */
  number: string | null;
  /** The total, in integer minor units. Frozen at finalize. */
  amount_due: number;
  /** `0`, or {@link Invoice.amount_due}. Never between the two. */
  amount_paid: number;
  /** `amount_due - amount_paid`, and what `pay` mints an intent for. */
  amount_remaining: number;
  /**
   * What has been given back out of {@link Invoice.amount_paid}, as a
   * **gross** total.
   *
   * It is not subtracted from `amount_paid` and does not raise
   * `amount_remaining`: a refunded invoice is still `"paid"` with nothing
   * remaining, and vpay has no credit note object. `0` on every invoice today
   * — no vpay rail can refund yet, so nothing can move it. Do not read a
   * non-zero value as money being owed again.
   *
   * Optional in the type so a client of this version keeps compiling against
   * a server that predates migration `0042`, where the key is absent.
   */
  amount_refunded?: number;
  /**
   * Unix **seconds**, or `null`. **Advisory**: nothing in vpay reads it —
   * there is no dunning, no reminder and no automatic transition.
   */
  due_date: number | null;
  description: string | null;
  metadata: Record<string, string>;
  /** The `pi_…` paying, or that paid, this invoice — `null` until `pay`. */
  payment_intent: string | null;
  /** The checkout session for {@link Invoice.payment_intent}. See above. */
  hosted_invoice_url: string | null;
  /** Every line, in the order they were added. See above for when this is empty. */
  lines: List<InvoiceLine>;
  status_transitions: InvoiceStatusTransitions;
  /** Unix **seconds** — not milliseconds, and not RFC 3339. */
  created: number;
  livemode: boolean;
}

/**
 * What `client.invoices.del(…)` answers with — Stripe's deleted-object shape.
 *
 * A **draft** is deleted and an issued invoice is voided: a voided draft
 * would be a non-draft row with no number, a state the database refuses
 * outright.
 */
export interface DeletedInvoice {
  id: string;
  object: "invoice";
  deleted: true;
}

/**
 * What `client.invoiceItems.del(…)` answers with.
 *
 * Its `object` is `"line_item"` and not `"invoice_item"`, matching
 * {@link InvoiceLine.object} — the route and the object are named
 * differently and both spellings are the wire's.
 */
export interface DeletedInvoiceItem {
  id: string;
  object: "line_item";
  deleted: true;
}

/**
 * `POST /v1/invoices` request fields (S4b).
 *
 * `customer` is the only required one, and it is required for a reason worth
 * stating: an invoice is a bill to somebody, and one that names no payer is
 * one nobody can be asked to pay. The server answers `400` naming `customer`
 * for an absent, unknown *or other merchant's* `cus_…` — always the same
 * sentence, so the parameter cannot be used to discover which customers exist
 * under some other account.
 *
 * A created invoice is always a `"draft"` with no number, no lines and zero
 * amounts. Add lines with `client.invoiceItems.create`, then issue it with
 * `client.invoices.finalize`.
 */
export interface CreateInvoiceParams {
  /** The `cus_…` this invoice bills. */
  customer: string;
  /**
   * Lower-cased at encode time regardless of how it was supplied, as an
   * intent's is.
   *
   * **Required**, as it is on an intent. This field was optional until
   * 2026-09-08 and documented as letting "the server apply this deployment's
   * own default"; measured against a running vpay, a create with no
   * `currency` is `400 A three-letter \`currency\` code is required.` —
   * there is no such default. A type that can express only sendable requests
   * is why {@link CreatePaymentIntentParams.currency} is required too.
   */
  currency: string;
  /** At most 1000 characters, which the server checks. */
  description?: string | undefined;
  /**
   * Unix **seconds**, as Stripe spells it — not milliseconds.
   *
   * **Advisory**: nothing in vpay reads it, so setting this changes nothing
   * about what vpay does. It is a field the merchant's own systems may read
   * back off {@link Invoice.due_date}.
   */
  due_date?: number | undefined;
  metadata?: Record<string, string> | undefined;
}

/**
 * `POST /v1/invoices/{id}` request fields (S4b) — **draft only**.
 *
 * # Three states per field, exactly as {@link UpdateCustomerParams} has
 *
 * `undefined` (or an absent key) leaves the field alone, a value sets it, and
 * **`null` clears it** — which this SDK sends as `description=`. Collapsing
 * `null` and `undefined` makes a due date set by mistake unremovable, and
 * nothing else notices. `sdks/rust` spells the same three states
 * `Option<Option<T>>`.
 *
 * `customer` and `currency` are **not** patchable and deliberately absent
 * from this type: an invoice that changed who it bills, or what it is
 * denominated in, is a different document.
 *
 * `metadata` has two states rather than three and that is the wire contract:
 * it is merged key-wise, and a key whose value is the empty string is
 * removed.
 */
export interface UpdateInvoiceParams {
  description?: string | null | undefined;
  /** Unix **seconds**. `null` clears it. */
  due_date?: number | null | undefined;
  metadata?: Record<string, string> | undefined;
}

/**
 * `GET /v1/invoices` query parameters (S4b).
 *
 * A `type` alias for {@link ListParams}' index-signature reason.
 */
export type ListInvoicesParams = {
  limit?: number | undefined;
  starting_after?: string | undefined;
  ending_before?: string | undefined;
  customer?: string | undefined;
  /**
   * Only invoices in this state.
   *
   * Typed as the closed {@link InvoiceStatus} rather than `string`, and this
   * is the one filter where that earns its place: the server answers `400`
   * naming `status` for a label it does not have rather than silently
   * returning the whole first page, so a typo would cost a round trip and
   * read like an empty result.
   */
  status?: InvoiceStatus | undefined;
};

/**
 * `POST /v1/invoices/{id}/pay` request fields (S4b).
 *
 * **Both URLs are required**, unlike on a checkout session where the pair is
 * required only in hosted mode: `pay` mints a *hosted* session and there is
 * no other mode to be in. They take the same rules a session's do — http(s),
 * at most 2048 characters, `https` only under livemode, and `success_url` may
 * carry the literal `{CHECKOUT_SESSION_ID}` — and this SDK deliberately does
 * not duplicate them, for {@link CreateCheckoutSessionParams}' reason.
 */
export type PayInvoiceParams = {
  success_url: string;
  cancel_url: string;
};

/**
 * `POST /v1/invoice_items` request fields (S4b).
 *
 * There is no `currency` parameter and no `amount` one, and both absences are
 * the wire contract rather than an omission here: a line is always in its
 * invoice's currency (copied off the parent in the statement that writes the
 * row), and `amount` is `quantity * unit_amount` computed by the database. A
 * merchant who could send `amount` could send one that did not match its own
 * factors.
 */
export interface CreateInvoiceItemParams {
  /**
   * The `in_…` to add this line to, and it must be one of your **drafts** —
   * a `400` naming `invoice` otherwise, including for a finalized invoice and
   * for another merchant's.
   */
  invoice: string;
  /** At most 1000 characters. */
  description: string;
  /**
   * At least 1. Omitted from the body when absent, so the server applies its
   * own default of 1 rather than this SDK sending one.
   */
  quantity?: number | undefined;
  /**
   * The price of one, in integer minor units. A non-integer, a negative, or
   * anything past `Number.MAX_SAFE_INTEGER` throws `TypeError` before any
   * request — `sdks/rust` refuses the identical set.
   */
  unit_amount: number;
}

/**
 * `POST /v1/invoice_items/{id}` request fields (S4b) — **draft parent only**.
 *
 * # Why these are two-state and {@link UpdateInvoiceParams}' are three-state
 *
 * All three columns are `NOT NULL`, so there is no "clear it" state to carry:
 * an absent key leaves the field alone and a value sets it. Sending
 * `description=` is a `400` naming the parameter rather than a clear, which
 * is why this type cannot express it — no `| null`.
 *
 * `amount` is not here, ever — see {@link CreateInvoiceItemParams}.
 */
export interface UpdateInvoiceItemParams {
  description?: string | undefined;
  quantity?: number | undefined;
  unit_amount?: number | undefined;
}

export interface CreatePaymentIntentParams {
  /**
   * Integer minor units. A non-integer, a negative, or anything past
   * `Number.MAX_SAFE_INTEGER` throws `TypeError` before any request.
   */
  amount: number;
  currency: string;
  payment_method_types: PaymentMethodType[];
  metadata?: Record<string, string> | undefined;
  description?: string | undefined;
  /**
   * The `cus_…` this intent is for (S4a).
   *
   * Accepted and **dropped** by any vpay predating 2026-09-06, which is worth
   * knowing before relying on it: an older server answers `200` with
   * `customer: null` rather than refusing. Check the response.
   */
  customer?: string | undefined;
}

/** A `type` alias for the same index-signature reason as {@link ListParams}. */
export type MtnMomoPaymentMethodData = {
  type: "mtn_momo";
  mtn_momo: { msisdn: string };
};

/** A `type` alias for the same index-signature reason as {@link ListParams}. */
export type OrangeMoneyPaymentMethodData = {
  type: "orange_money";
};

export type ConfirmPaymentIntentParams =
  | { payment_method_data: MtnMomoPaymentMethodData }
  | { payment_method_data: OrangeMoneyPaymentMethodData; return_url: string };

/**
 * `POST /v1/checkout/sessions` request fields.
 *
 * The URL rules are the server's and are not duplicated here: `success_url`
 * and `cancel_url` are required for `hosted` and refused for `embedded`,
 * `return_url` the other way round, all http(s) and at most 2048
 * characters, `https` only under livemode. This SDK sends what it is given
 * and lets the server say no — a second copy of those rules here would be a
 * second thing to keep in step with them, and would refuse a combination a
 * later server version allows.
 */
export interface CreateCheckoutSessionParams {
  /** The `pi_…` to drive. Required; the session never creates one. */
  payment_intent: string;
  /** Defaults to `hosted` server-side when omitted. */
  ui_mode?: CheckoutSessionUiMode | undefined;
  /** Where to send a payer who paid. Hosted mode. May contain `{CHECKOUT_SESSION_ID}`. */
  success_url?: string | undefined;
  /** Where to send a payer who gave up. Hosted mode. */
  cancel_url?: string | undefined;
  /** Where vpay's framed page forwards the payer at the end. Embedded mode. */
  return_url?: string | undefined;
}

/**
 * `GET /v1/checkout/sessions` query parameters. A `type` alias for the same
 * index-signature reason as {@link ListParams}.
 */
export type ListCheckoutSessionsParams = {
  limit?: number | undefined;
  starting_after?: string | undefined;
  ending_before?: string | undefined;
  /** Only sessions for this `pi_…`. */
  payment_intent?: string | undefined;
};

export interface CreateRefundParams {
  payment_intent: string;
  /** Integer minor units. Omit for a full refund. */
  amount?: number | undefined;
  reason?: string | undefined;
  metadata?: Record<string, string> | undefined;
}

/**
 * Written flat rather than as `ListParams & { … }` for the same
 * index-signature reason as {@link ListParams}: only an anonymous object type
 * gets an implicit index signature, and an intersection of aliases does not.
 */
export type ListEventsParams = {
  limit?: number | undefined;
  starting_after?: string | undefined;
  ending_before?: string | undefined;
  type?: string | undefined;
};

/** Per-call options accepted by every resource method that issues a `POST`. */
export interface RequestOptions {
  /** Caller-supplied idempotency key; a fresh UUIDv4 is generated when omitted. */
  idempotencyKey?: string | undefined;
}
