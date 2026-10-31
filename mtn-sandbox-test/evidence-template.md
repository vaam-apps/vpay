# MTN MoMo sandbox: callback evidence

Copy this file into `evidence/` before filling it in, so the filled copy stays
out of Git.

| Field                     | Value                                    |
| ------------------------- | ---------------------------------------- |
| Tester                    |                                          |
| Date                      |                                          |
| Base URL                  | `https://sandbox.momodeveloper.mtn.com`  |
| Product                   | Collections                              |
| `providerCallbackHost`    |                                          |
| Inspector URL             |                                          |
| Setup time (UTC)          | from `evidence/setup.json`               |

Fill each table from `evidence/payment-N/payment.json` and from the
inspector. Write what you saw, not what you expected. If a callback never
arrived, the callback count is 0 and the rows about it say "none".

## Payment 1

| Field                                   | Value |
| --------------------------------------- | ----- |
| Inspector configured to answer          |       |
| Request sent at (UTC)                   |       |
| 202 received at (UTC)                   |       |
| `X-Reference-Id`                        |       |
| Callback count                          |       |
| Callback received at (inspector times)  |       |
| HTTP method                             |       |
| `Content-Type`                          |       |
| Body field names                        |       |
| `externalId` matches `X-Reference-Id`?  |       |
| `referenceId` present and matches?      |       |
| Final GET status                        |       |
| SUCCESSFUL at (UTC)                     |       |
| Notes                                   |       |

## Payment 2

| Field                                   | Value |
| --------------------------------------- | ----- |
| Inspector configured to answer          |       |
| Request sent at (UTC)                   |       |
| 202 received at (UTC)                   |       |
| `X-Reference-Id`                        |       |
| Callback count                          |       |
| Callback received at (inspector times)  |       |
| HTTP method                             |       |
| `Content-Type`                          |       |
| Body field names                        |       |
| `externalId` matches `X-Reference-Id`?  |       |
| `referenceId` present and matches?      |       |
| Final GET status                        |       |
| SUCCESSFUL at (UTC)                     |       |
| Notes                                   |       |

## Payment 3

| Field                                   | Value |
| --------------------------------------- | ----- |
| Inspector configured to answer          |       |
| Request sent at (UTC)                   |       |
| 202 received at (UTC)                   |       |
| `X-Reference-Id`                        |       |
| Callback count                          |       |
| Callback received at (inspector times)  |       |
| HTTP method                             |       |
| `Content-Type`                          |       |
| Body field names                        |       |
| `externalId` matches `X-Reference-Id`?  |       |
| `referenceId` present and matches?      |       |
| Final GET status                        |       |
| SUCCESSFUL at (UTC)                     |       |
| Notes                                   |       |

## Observations

- Did MTN retry after the inspector answered 500? How many times, and at what
  intervals?
- Did any callback arrive before the GET poll showed SUCCESSFUL?
- Anything in the callback that the GET response did not contain, or the
  other way round?

## Attachments

> **Reminder:** attach the inspector's screenshots or exports for every
> callback (one per request received, including retries) and the
> `evidence/payment-N/` folders. Before attaching, check that no screenshot
> or export shows a subscription key, API key or access token.

- [ ] Payment 1: inspector screenshot or export
- [ ] Payment 2: inspector screenshot or export
- [ ] Payment 3: inspector screenshot or export
- [ ] `evidence/setup.json` and `evidence/payment-1/` to `payment-3/`
