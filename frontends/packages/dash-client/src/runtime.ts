// A value import back into `models.ts`, which imports `JsonValue` from
// this file in the other direction. The cycle is fine and already
// established by `rpc-runtime.ts.j2`'s identical import: the other leg is
// `import type`, which is erased entirely at compile time.
import { encodeBinaryAsJson } from "./models.js";

export type JsonPrimitive = string | number | boolean | null;
export type JsonValue = JsonPrimitive | JsonValue[] | { [key: string]: JsonValue };

// Hex-encoded SHA-256 of the schema's source bytes (issue #178) — baked in
// at generation time from `TypeScriptGeneratorConfig::schema_sha256`. Sent
// as `x-cratestack-schema-sha` on every request so a client compiled
// against a stale `.cstack` schema shows up as a server-side
// `tracing::warn!`, never a rejection. Empty when the CLI wasn't given a
// schema fingerprint (e.g. this crate used as a library directly, or a
// test) — the header is simply omitted in that case.
export const SCHEMA_SHA256: string = "ff0be8444058eb156f4781cee1e53847c5648e944cc0dacb2725558bc81e7740";
const SCHEMA_SHA_HEADER = "x-cratestack-schema-sha";

// Per-op contract digests (binding version 2, cratestack#1123): what a signed
// request to each op binds into its COSE AAD, keyed like the Rust client's
// `OP_CONTRACTS` (the RPC op id, or "<METHOD> <route template>" on REST, plus
// `batch` for `transport rpc`). Computed by the same `cratestack_core`
// function the Rust macros call, so the two cannot disagree. Not used by the
// unsigned runtime below: the sealers (ADR 0006 §11) read them.
export const OP_CONTRACTS: Readonly<Record<string, string>> = {
  "DELETE /charges/{id}": "598e5d0e4a3e748d4211f61a02b6e047da463a5b6e88bf872bc3a83c93204ba3",
  "DELETE /checkout_sessions/{id}": "763f1af11cb89524ac87123ac7a9fb27b4bc46f9d2bcad3b68231a4c8448cb4c",
  "DELETE /console_keys/{id}": "386ceecfbe1b41c45c273c3461c42292480139a40c14b7e75f9371799217b3ee",
  "DELETE /credentials/{id}": "0ab78a80f0561f4ac356e604a848e8f160e564f4787eae5d8266971a93daef6e",
  "DELETE /currencies/{id}": "b88e7eefcbb295b20519c16d1819941718ae0ceebdbb17f5fb22b72dc5170dc8",
  "DELETE /customers/{id}": "16ad1b08e99684697328d7954c04591edcbb8aa73ddd9987fa50f9d0a7044dce",
  "DELETE /disabled_clients/{id}": "4d6ba5f986b9a0951564638ca5a85604382d108f8e9d413484a838f226e27199",
  "DELETE /envelope_nonces/{id}": "cf2458889804fe0b106c71ae615d9f67852d656b1871cbc0e7856ebf434ead99",
  "DELETE /events/{id}": "fec0bdf9952c4590cc4bebd6bbef88a62c3825bcac689d0eb215083e0e9f69bf",
  "DELETE /invoice_items/{id}": "1f1ffe104e677969d9ed634b2d666d31f6d2b68bdcc4c24837c1fe675b1e646f",
  "DELETE /invoices/{id}": "4d5abd01adaff77c36daf499095daceb286852a47f8782f4b4a02a749f0876c9",
  "DELETE /ledger_entries/{id}": "61fed9ba9b1893697d1456798ad48c9f0aa9dee98acdbbaa81a031c43bec72bb",
  "DELETE /ledger_transactions/{id}": "be70959e65fddc5227e5ff80c32a6881c46b02ccad95bd8bb98baed0ad0c6729",
  "DELETE /manual_payments/{id}": "1f9710899695d948219e8e872e872230102b3bf937069b37515c3ccdbf9759cd",
  "DELETE /oauth_authorization_codes/{id}": "211a323f650d8bc6360fa7bb781d9920ce20eb996a176453e864c16ff300180d",
  "DELETE /payment_intents/{id}": "0659b4c3f8efb5c9f1b8e940f3fa5746dbbb99d4110d1eff4b5d9d2f40b466d9",
  "DELETE /providers/{id}": "b41f2a7a8ba8705bb7aa8c92ac6f54ef742e94d08e3bfacdee08930d47cd2d29",
  "DELETE /rate_limit_windows/{id}": "b33df2a38ce1cb617165c775eeba2a9f8bda8234f514d5ecb281a351893dc242",
  "DELETE /refunds/{id}": "2a73f48b3daddd3737bd90167d576af78a1118804bae96a686fecd03c487b663",
  "DELETE /staff_members/{id}": "297924590c456f5103a8ee50854645af3047abc1cdc1a4eea0461571e5f40293",
  "DELETE /staff_sessions/{id}": "1b478df8e4f83a42eede9a733a9746a406cca1975f167dbb8e1738da6c300ebf",
  "DELETE /staff_sso_attempts/{id}": "3fde5a604a2f95783f8714d7fa42ee53a60dcb5210a1b1518342c8f8cda35d66",
  "DELETE /webhook_deliveries/{id}": "c933ad51461dfef2f33e0347c976adcf1c813cfcc0b250851b8e899ada2a90c5",
  "GET /audit_events": "1a75fab039020267a65aadeb44db4ae16495db856204c77b5a7ee9291ebef9c2",
  "GET /audit_events/{id}": "6956d546ec33089f82db47e6efeaeecb82b188778987ae54b4b186587210c3de",
  "GET /charges": "0e9a81ca23812c11414b9a9e44d362953a1e8116a34b4f8a0abbb370fdb8f855",
  "GET /charges/{id}": "ca701fa246134f191377644352925997a5d08c90e298d37d4385a5d31ff01aca",
  "GET /checkout_sessions": "f4900d7fd5044553c47e1f8b75edf52323bb57870f175b73228dfe1d41502518",
  "GET /checkout_sessions/{id}": "8603e2d3cbdaa546a8bbe08a4e242e365010c42fd19a11c436fb786d715da8e8",
  "GET /console_keys": "2b3a3d621c03c19efac5b2e8ce98e11fa93e35888897eb21e6bf1bb5017b6311",
  "GET /console_keys/{id}": "aa48c7bff2a473f637672214c8d7c072517abb1e0b1ef52b7fb1b41c3859ae4a",
  "GET /credentials": "db4fdd28da93439809d98fefa748e4e58e7685f4db44ca18f49df9f8e13a7908",
  "GET /credentials/{id}": "a8d7908efe1336d2f8818ff149d8ebdbd5aace70ea035eb3475ba48bd307a588",
  "GET /currencies": "b5aaec840f469b71f0211e82c049a6dea4ad156fc50cae663d4fb75fdf904f11",
  "GET /currencies/{id}": "1fffd1f2a6ac759a9f55f5820d165a6f2f6fd4d750bab9f56b5b06bff55f9aba",
  "GET /customers": "f27a94881ad3f78571f6634ef246fe511619e4705cabed0742dbe003e3216784",
  "GET /customers/{id}": "c759651d49b58d1083bc82ced07da27238e9c05b3fb3a791514cfe1281ef4307",
  "GET /disabled_clients": "c0dcbb6e041307c965fd9bb19ff46bb7118454deae8fb587b13d8bcf67a15b0a",
  "GET /disabled_clients/{id}": "9ac9b1e37d74eded3a6e9bd0aeff88883ce101455d207f31d7b5633118af6cff",
  "GET /envelope_nonces": "44db09f18848d96c9d9e12bb08bcedd8acbbb6418944e01d2c5f0204a07818c5",
  "GET /envelope_nonces/{id}": "1ea53a4ec0114449ad5f36860c4cb92c46c77486090cb821d7837f0abbebce60",
  "GET /events": "58d70b640a2f1bdaf2ba1b12c727d052c67d97c84bac6009be02c5885633ae99",
  "GET /events/{id}": "ec4c54444c1c3fd47c9ab196aafb1c725066cc4408806c968abeed3e8e84a9ca",
  "GET /invoice_items": "4504deb1650e754b031acbcbd3aba2f773458460c30fe3181a08f3a05090427e",
  "GET /invoice_items/{id}": "5d77eb3a0c324da0a7e35501ff9f1ff409d9889ece7f7c48f54978b1a1da7381",
  "GET /invoices": "5aa10b2b8003f5edff2575159fcaa18fc261e180958adb1b83ed415548a25780",
  "GET /invoices/{id}": "549bfba2795046ca0759530512a2a46b6c3ed71866d40d196b456e805af19f06",
  "GET /ledger_entries": "9103352892e96015339b65e0374eb0ba240dc6a9427a5b5638117a4de6bc3665",
  "GET /ledger_entries/{id}": "3de067e2573f7e34eb1d141ba80d1ace022b84c0717498647151a86d3611ef89",
  "GET /ledger_transactions": "04736ea143e622f6f1f327dab107aa4174b0dd1d9765a5e6deea8a1cf3693a36",
  "GET /ledger_transactions/{id}": "75293f6ba38b12ed60af311b6c2ecb37067b604d23c4ab19899a897c991fde3b",
  "GET /manual_payments": "298f734580ab4369f0f266005e0a7128ffffb9a3cf6c73d670d286306a5f6b04",
  "GET /manual_payments/{id}": "04ca77cec95407260633f7e16594ed1e8003dcc01b3e5facfcac99a16025e99a",
  "GET /oauth_authorization_codes": "fa847eb5b96a51adee9dacd85eaf1778919f002c6b2a7d90c2fd2e9c8086b315",
  "GET /oauth_authorization_codes/{id}": "aa604c8cd3aced8225bba3c402a076cb353d3fbc72b22072f2e8c63205a8c0c4",
  "GET /payment_intents": "c0c346730adee71d05e1e94ee07af4ca6bee107b5d6752300acea6389f93a97f",
  "GET /payment_intents/{id}": "a89cecdd013e31daafd3b108103ac45f4b70519503e392fd286a6161724346f0",
  "GET /providers": "e97e69f2a99a6263930d066d25dbe9f0b6eb97812a76368d0c5bbad39677f0cb",
  "GET /providers/{id}": "dc94cbc7ee345fc97d1f14066e6003ffc724458aeba382a23bf55a08504e4007",
  "GET /rate_limit_windows": "dd79409754f3cb94d06eb7d7db8ee9aadb9a034fcf9324c569763e79e73c1a94",
  "GET /rate_limit_windows/{id}": "b39d7948e85d828f260b0271bb52ae117eb47622c8358b9a592fd5dddeccbe08",
  "GET /refunds": "952213763c147faa835a2519368b2e0585baef9b253290e7278a44456e889d00",
  "GET /refunds/{id}": "f479099fc39102e5de951618f6a7434f3f08c6c92ed091e6c0ff3b57b2e044f4",
  "GET /staff_members": "4457b142704afbb576673ffaac74722298230be7acc4e84f3844568fc5227329",
  "GET /staff_members/{id}": "c39046216a60f97b6e72acd77bc5573d7126848165258e4998f882499d2f746e",
  "GET /staff_sessions": "2071c0619b318a3dc8fef2d5431d8a44146beb5eb26e2c065eec48ed418d7ef4",
  "GET /staff_sessions/{id}": "99b3258cff7611df49ddb3309143ec14a248206c98cfb9ea1f1c05cf816cba03",
  "GET /staff_sso_attempts": "945f6acc5615ba60168b4a47c735b7659f3754c20dd701301cc7429c9b849a44",
  "GET /staff_sso_attempts/{id}": "c2fdd77b6cdd4c90919ada1d67538c84210be645b9406b8edee41c0a9ca72d23",
  "GET /webhook_deliveries": "9badb9354e589222950861d232dda972eb4699574a9ec1ce3facfb1a16430ed2",
  "GET /webhook_deliveries/{id}": "a86d427b4c6965633b6da74948b237843cf2841d680ec93b4c44161f9ce920cb",
  "PATCH /charges/{id}": "eaa43f86e3abd1c13f3ae5f948be228ce23f8ef2e8dd03ca1d8f63ed070417e2",
  "PATCH /checkout_sessions/{id}": "738033f83acf726686ecf1c50395bcd542f62b581e304172d77fff1f1a62e115",
  "PATCH /console_keys/{id}": "c45066cd69a1c9d066d9109f0a3a598eb545d0da7905d32edc60eb4ca4ce55c3",
  "PATCH /credentials/{id}": "32f116443fb88a97a6481175d74646603c3a4884ff9251e8051aad09aca23755",
  "PATCH /currencies/{id}": "2151b094964e2e12f94124ef2700c2fefdc50329e1509cb08364042841a23787",
  "PATCH /customers/{id}": "b581ad203ec2f697a55b1f17065417ab1735d42aed3a77e5049c5ee30c289586",
  "PATCH /disabled_clients/{id}": "0ea4babb381dd900bcb0e6d4fd1f1fddd0f7dc2215aea09f11823cce94074b9f",
  "PATCH /envelope_nonces/{id}": "d4b552876de54d4739079cc393d3b6c78b5c03e74b8744b12e5367014037e14a",
  "PATCH /events/{id}": "198617379aa63fe51ebb71d94f22fd48a7a1db25bf609e29d562a1068a8b7975",
  "PATCH /invoice_items/{id}": "ef87e51da8b2808c428bb69f6487ae3dd0dfec76680f909c36c20137d6e37321",
  "PATCH /invoices/{id}": "b0dae1fd8eb1c59d3741b8d78f83793646ec8d923c1350bb4648b6afa8d42323",
  "PATCH /ledger_entries/{id}": "37a059052057b1df3daab3fa5389973b1eaeb758d679415ee6640ca92e2d2206",
  "PATCH /ledger_transactions/{id}": "ae331f6db48ce2ca9c4b6d988be26afc0224071dfedd6d14fc88d285164b3ea8",
  "PATCH /manual_payments/{id}": "055cd6ebe71fc3feef95c7680ddd3da3519a3c6e3b7936ea61d0c34fa0117d70",
  "PATCH /oauth_authorization_codes/{id}": "cd31e144adb7560852821bcaade261ce13f9221479373df5cfa4f86e90d3151a",
  "PATCH /payment_intents/{id}": "b4bd268214884d97341203531d359dbfe8a9965ca734882807a729748543b7d2",
  "PATCH /providers/{id}": "51febc1907365d72056b320953ab29fce123eb2c1714b78226b08b0cc6211bd8",
  "PATCH /rate_limit_windows/{id}": "81a82cfc58030a94150a43f8a4a5486d22194b8f1e956cdda49cac2c6f8a6da4",
  "PATCH /refunds/{id}": "c40ad873ce2f01a0e4885cc01523d761f906ed0c943265639b58cafeca0e813a",
  "PATCH /staff_members/{id}": "b3d30f0956ce4e9f801ccc48d259765349f5b81742e652c65b52eb1eaec56f0f",
  "PATCH /staff_sessions/{id}": "8966f3d246071c32c2e5ae9b25762a006713ac948653b1fe2e7017b7b2539415",
  "PATCH /staff_sso_attempts/{id}": "dc001e530f59765ed9c88e64c84fc91feea8e45d8dfbc79d20be607cf11e1451",
  "PATCH /webhook_deliveries/{id}": "4a966ed237b8442fb573de1c184851d34ed54acea66b071c197281550042fc45",
  "POST /$procs/cancelPaymentIntent": "7dec3afff1cf9a49bf135dbbe58089e84fe64ca8653738fe47b8c4f20be81418",
  "POST /$procs/cancelRefund": "401399d5ec73e662f9289510d3f10fa2614aa18321c6e5894623b52645d72d06",
  "POST /$procs/confirmPaymentIntent": "0cba3ee16f89d5dc9d2be49a17b841b9d4af5889e46d6bd19b6d8d8026af3b90",
  "POST /$procs/issueRefund": "30d0fb9e9f56f8a39640b9d06aa4f35b5240e46c23024d3e615370726a360967",
  "POST /$procs/openPaymentIntent": "52fedf0701980bed924a080f6383be7ba3314bbd1b5da8d33182a50c4b42b870",
  "POST /$procs/retrievePaymentIntent": "289fa50eeb06cb3d086efa078d175ac336b05be8176ac6282fd14be5350062aa",
  "POST /$procs/retrieveRefund": "57af31818ccbbd1cd3c68c56901ec50e5fce80e5bc3755859b465bbf87f4229c",
  "POST /$procs/searchCheckoutSessions": "1abb3f94d2e59eb32b7d63c6ff100ce24eae2e879c9129fbf6f90812c5094df2",
  "POST /$procs/searchCustomers": "85388306bff8f657c3ef9f6a8d3b0ca2dac3223cd71f93731a78e208d33131e0",
  "POST /$procs/searchPaymentIntents": "2f918f5b87a52f5db1db458f1ca96aa56534f6599b72cfce10320d69152122c9",
  "POST /$procs/searchRefunds": "5a2f8082d20c4d73117248f0928959d089dd8e88c0f205e2d40142953780f556",
  "POST /$procs/searchWebhookDeliveries": "308f1caac73923b98cadd75717df8512413b8850f5a4cd38d0f61d1578fa6b8d",
  "POST /charges": "4cbf391f8dd56a93a5e1f0721ee0285968587333f34616eb9a0c45a8db74c8ad",
  "POST /checkout_sessions": "8983bafe6d77df4734613c6a7a91bb704344ca03ea56b20dd20d73db1a28c7a0",
  "POST /console_keys": "e576d6cb5b4d39d64fa3e5c3d29619b5d31f65c94f4bcd7776232e0def53acd5",
  "POST /credentials": "20ee60986023f9dd46c4385b06fc215c6b63686da489842a165ee87f08804d03",
  "POST /currencies": "8c95b4b818ec41127a5486ca0d2e20a1f3d504c483d9cb4f7d4adc47cffefcac",
  "POST /customers": "f7a893fbe52424cdb6a74a8c66c5d310120aeb07ad282b3dbfcc6022a214d093",
  "POST /disabled_clients": "fb0ab2f78b4cdbe135c24125320c738568b06701273920a6f0586831cf79d42c",
  "POST /envelope_nonces": "e14adce0db4b909db024eddadc43452399df4fee3b83bcd5ed97e67c098c7d61",
  "POST /events": "7f101fe8ee4aa9c1bb40913dae74098168d802e9c0b49ce56d1ea88e0ff0d0a3",
  "POST /invoice_items": "bd6038e88e3065d487cf88102da32bda22efe1c1e1ffc547d31367566060d46d",
  "POST /invoices": "c9d1fc74d3f0ccd8a66d4bcb00a686e04ab61357d69010ad599d9baa524a0263",
  "POST /ledger_entries": "5a4a1c00f356f2ce27cfae0b9fbe6c79eeada9786e4fe9289ac058cc8c62a320",
  "POST /ledger_transactions": "0a7e160c2aa4460ed55d0d97228544c39c023a275e8a575e79bde53c69d955ff",
  "POST /manual_payments": "0513b0cd46b54cf8d2deee7a1732938cb33eced4c583ae3e543c1dd67f559965",
  "POST /oauth_authorization_codes": "58cb6f1bafdb5793af321cbc10e695517e6cf3b91e1e44ae25d37104de7427f8",
  "POST /payment_intents": "d33057dfae015f0b8a2c08dba5fb44b9e47cd97b2b54a1832914d490f5fa55e4",
  "POST /providers": "7378574557244ae9bd2af136e81fec4be2da4621cdcf56c354958fbdd342f6b8",
  "POST /rate_limit_windows": "96a928e39fa773693232871aa0dbd27deaa168d1b3a620f3aa2370dbe16ba27f",
  "POST /refunds": "fbdd505ea688d8eb7d7914b438770afc51533542f5aad11ebad8eac3c6559961",
  "POST /staff_members": "d0478d912e102d3f35725b4385ee5426d4767e7b0b01c1e925b99660b410d6f5",
  "POST /staff_sessions": "7e0e8b50aa357e6d9e052a1be0e0e654885a49ffcfaf9fc1a74d6c11ffe176db",
  "POST /staff_sso_attempts": "dc3d688e9fef6ee44615053c9c68200db18f6f8cf546c4562321bcb7614c5148",
  "POST /webhook_deliveries": "313534735f7d8114795740a6a144bf641e1f055f7ccdc9407c946c0c60a35ba6",
};
// Hex of the whole-contract digest: moves when any op's contract does.
export const CLIENT_CONTRACT_SHA256: string = "141caa8f4f756159ed6224fffb9c211f3e50abc230449b1e8d26ec8f22ae909c";

export interface CratestackClientOptions {
  basePath?: string;
  fetch?: typeof fetch;
  headers?: HeadersInit | (() => HeadersInit | Promise<HeadersInit>);
}

export interface CratestackRequestOptions {
  body?: unknown;
  headers?: HeadersInit | undefined;
  query?: Record<string, unknown> | undefined;
  signal?: AbortSignal | undefined;
}

// Issue #610: `request()` decodes the body and discards the `Response`,
// so no caller can ever reach a response header (`ETag`, most notably —
// the generated server stamps it on every `@version` model's GET/detail
// response, and requires it back as `If-Match` on PATCH/DELETE). This
// envelope is what `requestWithResponse()`/`getWithResponse()` return
// instead, so the header becomes reachable without changing `request()`'s
// existing return shape for every other call site.
export interface CratestackResponseEnvelope<T> {
  value: T;
  response: Response;
}

export class CratestackHttpError extends Error {
  readonly status: number;
  readonly response: Response;
  readonly payload: unknown;

  constructor(response: Response, payload: unknown) {
    super(`CrateStack request failed with status ${response.status}`);
    this.name = "CratestackHttpError";
    this.status = response.status;
    this.response = response;
    this.payload = payload;
  }
}

export class CratestackRuntime {
  readonly origin: string;
  readonly basePath: string;
  readonly fetchFn: typeof fetch;
  readonly defaultHeaders: HeadersInit | (() => HeadersInit | Promise<HeadersInit>) | undefined;

  constructor(origin: string, options: CratestackClientOptions = {}) {
    this.origin = origin.replace(/\/+$/, "");
    this.basePath = options.basePath ?? "/dash/v1";
    // `.bind(globalThis)`, not the bare global — some browsers' `fetch`
    // is spec'd to throw `TypeError: Illegal invocation` when called
    // with a receiver other than the global object (verified for real:
    // storing the bare function on `this` and calling it as
    // `this.fetchFn(...)` reproduces exactly that in Chrome/Vite dev,
    // even though the same code runs fine under Node's `fetch`, which
    // is why this was never caught by a Node-only test). A caller-
    // supplied `options.fetch` is trusted to already be correctly bound.
    this.fetchFn = options.fetch ?? fetch.bind(globalThis);
    this.defaultHeaders = options.headers;
  }

  async request<T>(
    method: string,
    path: string,
    options: CratestackRequestOptions = {},
  ): Promise<T> {
    const { value } = await this.requestWithResponse<T>(method, path, options);
    return value;
  }

  // Same request as `request()`, but returns the `Response` alongside the
  // decoded value (issue #610) instead of discarding it — `request()` is
  // now a thin wrapper around this that keeps only `.value`, so every
  // existing call site's return shape is unchanged.
  async requestWithResponse<T>(
    method: string,
    path: string,
    options: CratestackRequestOptions = {},
  ): Promise<CratestackResponseEnvelope<T>> {
    const headers = new Headers(await resolveHeaders(this.defaultHeaders));
    if (SCHEMA_SHA256 !== "") {
      headers.set(SCHEMA_SHA_HEADER, SCHEMA_SHA256);
    }
    headers.set("Accept", "application/json");

    let body: BodyInit | undefined;
    if (options.body !== undefined) {
      headers.set("Content-Type", "application/json");
      // `encodeBinaryAsJson` rewrites every `Uint8Array` (a `Bytes`
      // field's client-side type) into the integer array the wire uses —
      // `JSON.stringify` alone turns one into an index-keyed object no
      // server-side `Vec<u8>` can decode — and every `bigint` (a `BigInt`
      // field) into its canonical decimal string, which `JSON.stringify`
      // alone refuses to serialize at all. Unconditional here, unlike the
      // RPC transport's codec-dependent handling, because a REST request
      // body is always JSON.
      body = JSON.stringify(encodeBinaryAsJson(options.body));
    }

    for (const [key, value] of new Headers(options.headers)) {
      headers.set(key, value);
    }

    const response = await this.fetchFn(this.url(path, options.query), {
      method,
      headers,
      body: body ?? null,
      signal: options.signal ?? null,
    });

    const payload = await readResponsePayload(response);
    if (!response.ok) {
      throw new CratestackHttpError(response, payload);
    }
    return { value: payload as T, response };
  }

  get<T>(path: string, options: Omit<CratestackRequestOptions, "body"> = {}): Promise<T> {
    return this.request<T>("GET", path, options);
  }

  // Issue #610: the READ half of the ETag/If-Match round trip — read
  // `.response.headers.get("etag")` off the result, then pass that value
  // as `ifMatch` to a generated model's `update`/`delete` method.
  getWithResponse<T>(
    path: string,
    options: Omit<CratestackRequestOptions, "body"> = {},
  ): Promise<CratestackResponseEnvelope<T>> {
    return this.requestWithResponse<T>("GET", path, options);
  }

  post<T>(path: string, body: unknown, options: Omit<CratestackRequestOptions, "body"> = {}): Promise<T> {
    return this.request<T>("POST", path, { ...options, body });
  }

  patch<T>(path: string, body: unknown, options: Omit<CratestackRequestOptions, "body"> = {}): Promise<T> {
    return this.request<T>("PATCH", path, { ...options, body });
  }

  delete<T>(path: string, options: Omit<CratestackRequestOptions, "body"> = {}): Promise<T> {
    return this.request<T>("DELETE", path, options);
  }

  private url(path: string, query?: Record<string, unknown>): string {
    const normalizedBase = this.basePath === "/" ? "" : this.basePath.replace(/\/+$/, "");
    const normalizedPath = path.startsWith("/") ? path : `/${path}`;
    const url = new URL(`${normalizedBase}${normalizedPath}`, `${this.origin}/`);
    appendQuery(url.searchParams, query);
    return url.toString();
  }
}

async function resolveHeaders(
  headers: HeadersInit | (() => HeadersInit | Promise<HeadersInit>) | undefined,
): Promise<HeadersInit | undefined> {
  if (typeof headers === "function") {
    return headers();
  }
  return headers;
}

async function readResponsePayload(response: Response): Promise<unknown> {
  if (response.status === 204) {
    return undefined;
  }

  const text = await response.text();
  if (text.length === 0) {
    return undefined;
  }

  const contentType = response.headers.get("Content-Type") ?? "";
  if (contentType.includes("application/json")) {
    return JSON.parse(text);
  }
  return text;
}

function appendQuery(searchParams: URLSearchParams, query?: Record<string, unknown>): void {
  if (!query) {
    return;
  }

  for (const [key, value] of Object.entries(query)) {
    appendQueryValue(searchParams, key, value);
  }
}

function appendQueryValue(searchParams: URLSearchParams, key: string, value: unknown): void {
  if (value === undefined || value === null) {
    return;
  }

  if (Array.isArray(value)) {
    for (const item of value) {
      appendQueryValue(searchParams, key, item);
    }
    return;
  }

  if (typeof value === "object") {
    // The same walk the body gets: an object-valued query entry
    // (`computedParams`) may hold a `bigint`, which `JSON.stringify` throws
    // on. A scalar `bigint` below needs nothing: `String(7n)` is `"7"`.
    searchParams.set(key, JSON.stringify(encodeBinaryAsJson(value)));
    return;
  }

  searchParams.append(key, String(value));
}