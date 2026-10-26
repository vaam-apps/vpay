# @vpay/dash-client

Generated CrateStack TypeScript client with a fetch transport.

```ts
import { VpayDashClientClient } from "@vpay/dash-client";

const client = new VpayDashClientClient("https://api.example.com");
```

The generated client uses `/dash/v1` as its API base path by default.

## Runtime Setup

```ts
const client = new VpayDashClientClient("https://api.example.com", {
  basePath: "/dash/v1",
  headers: async () => ({
    authorization: `Bearer ${await tokenStore.getAccessToken()}`,
    "x-request-id": crypto.randomUUID(),
  }),
});
```

Per-call headers are also supported:

```ts
const headers = {
  authorization: `Bearer ${accessToken}`,
  "idempotency-key": idempotencyKey,
};
```

## Models

- `client.currencies`
- `client.providers`
- `client.paymentIntents`
- `client.charges`
- `client.refunds`
- `client.checkoutSessions`
- `client.ledgerTransactions`
- `client.ledgerEntries`
- `client.disabledClients`
- `client.events`
- `client.webhookDeliveries`
- `client.customers`
- `client.staffMembers`
- `client.staffSessions`
- `client.staffSsoAttempts`
- `client.oauthAuthorizationCodes`
- `client.credentials`
- `client.invoices`
- `client.invoiceItems`
- `client.manualPayments`
- `client.rateLimitWindows`

### List

```ts
const pageOrItems = await client.currencies.list({
  query: {
    fields: ["id"],
    include: [],
    includeFields: {},
    limit: 20,
    offset: 0,
    sort: ["-id"],
  },
});
```

### Detail

```ts
const item = await client.currencies.get(id, {
  query: {
    fields: ["id"],
  },
  headers,
});
```

### Create, Update, Delete

```ts
const created = await client.currencies.create(input, { headers });
const updated = await client.currencies.update(created.id, patch, { headers });
await client.currencies.delete(updated.id, { headers });
```


## Procedures

- `client.procedures.searchPaymentIntents`
- `client.procedures.searchRefunds`
- `client.procedures.searchWebhookDeliveries`
- `client.procedures.searchCustomers`
- `client.procedures.searchCheckoutSessions`

```ts
const result = await client.procedures.searchPaymentIntents(args, {
  headers,
});
```

## React Native

React Native app runtimes provide `fetch`; pass the mobile API origin and keep auth token lookup in your app layer.

```ts
export function createClient(accessToken: string | null) {
  return new VpayDashClientClient(process.env.EXPO_PUBLIC_API_ORIGIN!, {
    basePath: "/dash/v1",
    headers: {
      ...(accessToken ? { authorization: `Bearer ${accessToken}` } : {}),
      "x-client": "sample-mobile",
    },
  });
}
```

## Decimal Fields

A `Decimal`-typed schema field is generated as a real `decimal.js`-backed `Decimal`
(cratestack#498), not a `string`:

```ts
import { Decimal } from "@vpay/dash-client";
const item = await client.currencies.get(id);
console.log(item.amountField.toString()); // e.g. "0.0000001", never scientific notation
const total = item.amountField.plus(item.taxField); // real arbitrary-precision arithmetic
```

**Migration from pre-#498 generated clients:** code that treated a `Decimal` field as a
`string` (string concatenation, `Number()`/`parseFloat`, `===` comparison) needs to switch
to `Decimal`'s own API — `new Decimal(input)` to construct one, `.toString()` to format,
`.plus()`/`.minus()`/`.times()`/`.div()`/`.cmp()`/`.equals()` for arithmetic and comparison.
`DecimalFilter`'s `eq`/`ne`/`lt`/`lte`/`gt`/`gte`/`in` fields need the same treatment when
building a `Where`/`FindMany` argument by hand — encoding a `Decimal` still works with
`JSON.stringify` alone (no extra glue needed), but decoding one from raw JSON does not.

This parses correctly regardless of which `Decimal` backend built the server
(`decimal-rust-decimal` or `decimal-bigdecimal` — see `cratestack-core`'s README): both
plain positional notation (`"0.0000001"`) and the scientific notation `bigdecimal` emits
past `rust_decimal`'s ~28-29 significant-digit capacity (`"1E-7"`) decode to the identical
value, and this package always re-encodes in plain notation.

## BigInt Fields

A `BigInt`-typed schema field is a real `bigint` (ADR 0019), exact over the whole signed
64-bit range, where a `number` is exact only to 2^53 - 1. On the wire it is always a
canonical decimal **string**, on JSON and CBOR alike, so this client converts at the edges
and your code never sees the string:

```ts
const item = await client.currencies.get(id);
console.log(item.balanceField); // a bigint, e.g. 9007199254740993n
const next = item.balanceField + 1n; // exact: 9007199254740994n, sent as "9007199254740994"
```

- **Decoding** turns the string (or each string of a `BigInt[]`) into a `bigint`. A JSON
  number, or a CBOR integer, at a `BigInt` field **throws** a `TypeError` naming the field,
  instead of being read as a number: a value that arrives that way may already have been
  rounded by the server or a proxy.
- **Encoding** turns every `bigint` in a request into its string, on every path this client
  owns: REST bodies and query values, RPC unary, batch and stream inputs, `computedParams`,
  and `@cratestack/link-batch`. Do not pass `Number(x)`; it throws away what `bigint` keeps.
- **Filters** are `BigIntFilter`, i.e. `ComparableFilter<bigint>`.
- `JSON.stringify` throws on a `bigint`. If you serialise one yourself (a log line, a cache
  key), convert it with `.toString()` first.