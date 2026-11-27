// A value import back into `models.ts`, which imports `JsonValue` from
// this file in the other direction. The cycle is fine and already
// established by `rpc-runtime.ts.j2`'s identical import: the other leg is
// `import type`, which is erased entirely at compile time.
import { encodeBinaryAsJson } from "./models.js";
import { COMPACT_JSON_CONTENT_TYPE, RestCompactSession } from "./compact.js";
import type { RestCompactCall } from "./compact.js";

export type JsonPrimitive = string | number | boolean | null;
export type JsonValue = JsonPrimitive | JsonValue[] | { [key: string]: JsonValue };

// Hex-encoded SHA-256 of the schema's source bytes (issue #178) — baked in
// at generation time from `TypeScriptGeneratorConfig::schema_sha256`. Sent
// as `x-cratestack-schema-sha` on every request so a client compiled
// against a stale `.cstack` schema shows up as a server-side
// `tracing::warn!`, never a rejection. Empty when the CLI wasn't given a
// schema fingerprint (e.g. this crate used as a library directly, or a
// test) — the header is simply omitted in that case.
export const SCHEMA_SHA256: string = "a901c158536617c1d7346470f755684191d960eef30105e6fc52837973587eeb";
const SCHEMA_SHA_HEADER = "x-cratestack-schema-sha";

// Per-op contract digests (binding version 2, cratestack#1123): what a signed
// request to each op binds into its COSE AAD, keyed like the Rust client's
// `OP_CONTRACTS` (the RPC op id, or "<METHOD> <route template>" on REST, plus
// `batch` for `transport rpc`). Computed by the same `cratestack_core`
// function the Rust macros call, so the two cannot disagree. Not used by the
// unsigned runtime below: the sealers (ADR 0006 §11) read them.
export const OP_CONTRACTS: Readonly<Record<string, string>> = {
  "DELETE /charges/{id}": "9b8c647e7ca7d781f4de60207afb3a08864c1bb533286379e1dd033ebea72c54",
  "DELETE /checkout_sessions/{id}": "0d1080ba72403134eb2764e7741f2a330e6eba39ed03f9d3349ca7ce0cd99178",
  "DELETE /console_keys/{id}": "386ceecfbe1b41c45c273c3461c42292480139a40c14b7e75f9371799217b3ee",
  "DELETE /credentials/{id}": "0ab78a80f0561f4ac356e604a848e8f160e564f4787eae5d8266971a93daef6e",
  "DELETE /currencies/{id}": "02e5a07bad4df10959a5cf216c0c7f5ce56ff736e2be06547dbe842cb8aa8e0b",
  "DELETE /customers/{id}": "16ad1b08e99684697328d7954c04591edcbb8aa73ddd9987fa50f9d0a7044dce",
  "DELETE /disabled_clients/{id}": "4d6ba5f986b9a0951564638ca5a85604382d108f8e9d413484a838f226e27199",
  "DELETE /envelope_nonces/{id}": "cf2458889804fe0b106c71ae615d9f67852d656b1871cbc0e7856ebf434ead99",
  "DELETE /events/{id}": "fec0bdf9952c4590cc4bebd6bbef88a62c3825bcac689d0eb215083e0e9f69bf",
  "DELETE /invoice_items/{id}": "1f1ffe104e677969d9ed634b2d666d31f6d2b68bdcc4c24837c1fe675b1e646f",
  "DELETE /invoices/{id}": "4d5abd01adaff77c36daf499095daceb286852a47f8782f4b4a02a749f0876c9",
  "DELETE /ledger_entries/{id}": "6477b96b3eb0f343140dc328d831648dd9ec918f6e3ce0bc98fa2c0b7cdc577a",
  "DELETE /ledger_transactions/{id}": "4b22e786473e6591e1b27374ad7e057dedbf37a7702b60d9b209b9cc6123631a",
  "DELETE /manual_payments/{id}": "1f9710899695d948219e8e872e872230102b3bf937069b37515c3ccdbf9759cd",
  "DELETE /oauth_authorization_codes/{id}": "211a323f650d8bc6360fa7bb781d9920ce20eb996a176453e864c16ff300180d",
  "DELETE /payment_intents/{id}": "1014d3ade15eb618bf80c9d25f25dcf2d88f428265b88b4ab68549321edbcdaf",
  "DELETE /providers/{id}": "6bbd55615cfe38d3ba39aee5eeefc1df1fc13595a3bf7802433044869f97e4b3",
  "DELETE /rate_limit_windows/{id}": "b33df2a38ce1cb617165c775eeba2a9f8bda8234f514d5ecb281a351893dc242",
  "DELETE /refunds/{id}": "96110ada324e3e38bcbeceac087a0e8a0379f5e144d67471e9eff19748bf9770",
  "DELETE /staff_members/{id}": "297924590c456f5103a8ee50854645af3047abc1cdc1a4eea0461571e5f40293",
  "DELETE /staff_sessions/{id}": "1b478df8e4f83a42eede9a733a9746a406cca1975f167dbb8e1738da6c300ebf",
  "DELETE /staff_sso_attempts/{id}": "3fde5a604a2f95783f8714d7fa42ee53a60dcb5210a1b1518342c8f8cda35d66",
  "DELETE /webhook_deliveries/{id}": "c933ad51461dfef2f33e0347c976adcf1c813cfcc0b250851b8e899ada2a90c5",
  "GET /audit_events": "1a75fab039020267a65aadeb44db4ae16495db856204c77b5a7ee9291ebef9c2",
  "GET /audit_events/{id}": "6956d546ec33089f82db47e6efeaeecb82b188778987ae54b4b186587210c3de",
  "GET /charges": "c9eae5cddde4390cf7e76fb36b27eb3f16c5ef49086bfe2829ce331e8c3f180e",
  "GET /charges/{id}": "3e2a195f1bbde14f0807e2e29fb138cbaaa8beb3191b18830fe0f0a25bf01db4",
  "GET /checkout_sessions": "a8a5acd5ce50f7f2f5a0ed48d2a74dc46031ca9733d91e7e70096b18cdb2f7a7",
  "GET /checkout_sessions/{id}": "6b0c3eb84f0e8c66dc4423f4b3ecd99c5b2737d480f1504a39188d10a1c77772",
  "GET /console_keys": "2b3a3d621c03c19efac5b2e8ce98e11fa93e35888897eb21e6bf1bb5017b6311",
  "GET /console_keys/{id}": "aa48c7bff2a473f637672214c8d7c072517abb1e0b1ef52b7fb1b41c3859ae4a",
  "GET /credentials": "db4fdd28da93439809d98fefa748e4e58e7685f4db44ca18f49df9f8e13a7908",
  "GET /credentials/{id}": "a8d7908efe1336d2f8818ff149d8ebdbd5aace70ea035eb3475ba48bd307a588",
  "GET /currencies": "767c5087e3e2568876f74c734236a758892d8c0edb272e99e9dcbda881c3ea4b",
  "GET /currencies/{id}": "dedf50c64a6855faff8b48cb724d053ab412467dfa6aea170ac572bf87fc017d",
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
  "GET /ledger_entries": "92617ad828f7956db491c8949b884b302b431c6ab01cfdb1f4956913562a1d7d",
  "GET /ledger_entries/{id}": "fa2eb14c780a09aaad040fd66ecb224c3cc04dc2f9e3ce1f6736d2a34b92b986",
  "GET /ledger_transactions": "0d8c99780dd509460353eef50dce71abeff46b5372aa80b7d779c6bba3193dbe",
  "GET /ledger_transactions/{id}": "929cfa02559f25f474e2a6e36eb65c7ad401b75af42a864c8396727b0bce02b4",
  "GET /manual_payments": "298f734580ab4369f0f266005e0a7128ffffb9a3cf6c73d670d286306a5f6b04",
  "GET /manual_payments/{id}": "04ca77cec95407260633f7e16594ed1e8003dcc01b3e5facfcac99a16025e99a",
  "GET /oauth_authorization_codes": "fa847eb5b96a51adee9dacd85eaf1778919f002c6b2a7d90c2fd2e9c8086b315",
  "GET /oauth_authorization_codes/{id}": "aa604c8cd3aced8225bba3c402a076cb353d3fbc72b22072f2e8c63205a8c0c4",
  "GET /payment_intents": "6d9f703058f26e1596733998501611ec05f5644b0142de9a244b2a6452d0afa1",
  "GET /payment_intents/{id}": "07d08140e6811facdbe6c626d698171d75d0b9a57de983500160acd013d3da89",
  "GET /providers": "753e469a77ac463af69bfcc662cc20507b29d3a03ca8a08b2619f37f344fe611",
  "GET /providers/{id}": "0927d25559d507dc357bff10887d3c07a584fd1701fba2b2083541a6f72a4ece",
  "GET /rate_limit_windows": "dd79409754f3cb94d06eb7d7db8ee9aadb9a034fcf9324c569763e79e73c1a94",
  "GET /rate_limit_windows/{id}": "b39d7948e85d828f260b0271bb52ae117eb47622c8358b9a592fd5dddeccbe08",
  "GET /refunds": "4cedc2937916c9d36806a488ad4a5743726e10dc3609cffd5e637a8677fa3860",
  "GET /refunds/{id}": "c94c6d679ee822bf673b64489f50a18b8f6346820eccc99cb9fd9924224707f7",
  "GET /staff_members": "4457b142704afbb576673ffaac74722298230be7acc4e84f3844568fc5227329",
  "GET /staff_members/{id}": "c39046216a60f97b6e72acd77bc5573d7126848165258e4998f882499d2f746e",
  "GET /staff_sessions": "2071c0619b318a3dc8fef2d5431d8a44146beb5eb26e2c065eec48ed418d7ef4",
  "GET /staff_sessions/{id}": "99b3258cff7611df49ddb3309143ec14a248206c98cfb9ea1f1c05cf816cba03",
  "GET /staff_sso_attempts": "945f6acc5615ba60168b4a47c735b7659f3754c20dd701301cc7429c9b849a44",
  "GET /staff_sso_attempts/{id}": "c2fdd77b6cdd4c90919ada1d67538c84210be645b9406b8edee41c0a9ca72d23",
  "GET /webhook_deliveries": "9badb9354e589222950861d232dda972eb4699574a9ec1ce3facfb1a16430ed2",
  "GET /webhook_deliveries/{id}": "a86d427b4c6965633b6da74948b237843cf2841d680ec93b4c44161f9ce920cb",
  "PATCH /charges/{id}": "f684fae70aa91e155110926c159025b6d1c7911e09f201b182f1da7813173208",
  "PATCH /checkout_sessions/{id}": "f0e1749da390a450594f5df92a7a637596a60984ad63cdded1861809e6454f3c",
  "PATCH /console_keys/{id}": "c45066cd69a1c9d066d9109f0a3a598eb545d0da7905d32edc60eb4ca4ce55c3",
  "PATCH /credentials/{id}": "32f116443fb88a97a6481175d74646603c3a4884ff9251e8051aad09aca23755",
  "PATCH /currencies/{id}": "dad8484be36e4453c906dbbb1b191d3e9b45841d8c50151161dd67f5b34f7e05",
  "PATCH /customers/{id}": "b581ad203ec2f697a55b1f17065417ab1735d42aed3a77e5049c5ee30c289586",
  "PATCH /disabled_clients/{id}": "0ea4babb381dd900bcb0e6d4fd1f1fddd0f7dc2215aea09f11823cce94074b9f",
  "PATCH /envelope_nonces/{id}": "d4b552876de54d4739079cc393d3b6c78b5c03e74b8744b12e5367014037e14a",
  "PATCH /events/{id}": "198617379aa63fe51ebb71d94f22fd48a7a1db25bf609e29d562a1068a8b7975",
  "PATCH /invoice_items/{id}": "ef87e51da8b2808c428bb69f6487ae3dd0dfec76680f909c36c20137d6e37321",
  "PATCH /invoices/{id}": "b0dae1fd8eb1c59d3741b8d78f83793646ec8d923c1350bb4648b6afa8d42323",
  "PATCH /ledger_entries/{id}": "6bce55d33f51d4d683f84d418be42026e5692304176ad146ce5d1f04c30ca582",
  "PATCH /ledger_transactions/{id}": "72e3fe94db1e07e71da9c4ee46ad6c523cdf51db8ed558f0beeb63f59d500ff9",
  "PATCH /manual_payments/{id}": "055cd6ebe71fc3feef95c7680ddd3da3519a3c6e3b7936ea61d0c34fa0117d70",
  "PATCH /oauth_authorization_codes/{id}": "cd31e144adb7560852821bcaade261ce13f9221479373df5cfa4f86e90d3151a",
  "PATCH /payment_intents/{id}": "c76f76aceb47fdc4456d3fa52fbac84acf460787e1d33aab64429eceaaaadabb",
  "PATCH /providers/{id}": "538c491b622e3f907835e6b502324e30c5b3cccd5198fe4811f1789bdd278d4d",
  "PATCH /rate_limit_windows/{id}": "81a82cfc58030a94150a43f8a4a5486d22194b8f1e956cdda49cac2c6f8a6da4",
  "PATCH /refunds/{id}": "63a6b934d483610acd4fd2111c4378b9f2e8f2a0d2313bc9c0593f6a1d3f72eb",
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
  "POST /charges": "8f5c29a729419fe9f9f9db6929c640f272c8b047c1400ff343bfa3458069f079",
  "POST /checkout_sessions": "b4f47f4229e9998766e7a74cb4fb71daf3d8a337c8dd4ec153758a5316880ea5",
  "POST /console_keys": "e576d6cb5b4d39d64fa3e5c3d29619b5d31f65c94f4bcd7776232e0def53acd5",
  "POST /credentials": "20ee60986023f9dd46c4385b06fc215c6b63686da489842a165ee87f08804d03",
  "POST /currencies": "b3f34dd94e47edc0c9d5c7aac6ee06d552f8839966b2a4f28c8b632631f681a6",
  "POST /customers": "f7a893fbe52424cdb6a74a8c66c5d310120aeb07ad282b3dbfcc6022a214d093",
  "POST /disabled_clients": "fb0ab2f78b4cdbe135c24125320c738568b06701273920a6f0586831cf79d42c",
  "POST /envelope_nonces": "e14adce0db4b909db024eddadc43452399df4fee3b83bcd5ed97e67c098c7d61",
  "POST /events": "7f101fe8ee4aa9c1bb40913dae74098168d802e9c0b49ce56d1ea88e0ff0d0a3",
  "POST /invoice_items": "bd6038e88e3065d487cf88102da32bda22efe1c1e1ffc547d31367566060d46d",
  "POST /invoices": "c9d1fc74d3f0ccd8a66d4bcb00a686e04ab61357d69010ad599d9baa524a0263",
  "POST /ledger_entries": "bd494bad63e266c6bfa24bb6f94c1e5a7a16171aeb92c2d1fef8ac441b45e936",
  "POST /ledger_transactions": "14fe9dbb218e80659ae7c7d75bf94b9165b7c1c6b75d89e50ffa3b03d43402a5",
  "POST /manual_payments": "0513b0cd46b54cf8d2deee7a1732938cb33eced4c583ae3e543c1dd67f559965",
  "POST /oauth_authorization_codes": "58cb6f1bafdb5793af321cbc10e695517e6cf3b91e1e44ae25d37104de7427f8",
  "POST /payment_intents": "a95756e5c01ac97c64de834eb56c0f454f7b57de7f9f6d0d37afe205e45ed5e2",
  "POST /providers": "a36d105709a26dc6c69ce0f36c1d2239c182cb76627e548bc5e8e53a266084e1",
  "POST /rate_limit_windows": "96a928e39fa773693232871aa0dbd27deaa168d1b3a620f3aa2370dbe16ba27f",
  "POST /refunds": "a17ecccc0f5da451a54c489be3782007310cde93c22e2570631794bed23c346b",
  "POST /staff_members": "d0478d912e102d3f35725b4385ee5426d4767e7b0b01c1e925b99660b410d6f5",
  "POST /staff_sessions": "7e0e8b50aa357e6d9e052a1be0e0e654885a49ffcfaf9fc1a74d6c11ffe176db",
  "POST /staff_sso_attempts": "dc3d688e9fef6ee44615053c9c68200db18f6f8cf546c4562321bcb7614c5148",
  "POST /webhook_deliveries": "313534735f7d8114795740a6a144bf641e1f055f7ccdc9407c946c0c60a35ba6",
};
// Hex of the whole-contract digest: moves when any op's contract does.
export const CLIENT_CONTRACT_SHA256: string = "4c08adc05f145e649f0339cb33a00ac3789fb391a452db0d8b456316e0ce69ae";

// Base64 of the compact wire key tables for every op above (the artifact
// `cratestack_core::compact::KeyTables::to_bytes` writes), read only when
// `compactKeys` is on (see `./compact`).
export const KEY_TABLES: string = "gwGYSpgsZmFtb3VudABqY3JlYXRlZF9hdABoY3VycmVuY3kDbWN1cnJlbmN5X2NvZGUAbGZhaWx1cmVfY29kZQBrZmFpbHVyZV9yYXcAYmlkAHNsZWRnZXJfdHJhbnNhY3Rpb25zggAGaXBheWVyX3JlZgBwcGF5ZXJfcmVmX21hc2tlZABucGF5bWVudF9pbnRlbnQHcXBheW1lbnRfaW50ZW50X2lkAGhwcm92aWRlcghzcHJvdmlkZXJfYWNjb3VudF9pZABtcHJvdmlkZXJfY29kZQB1cHJvdmlkZXJfcmVmZXJlbmNlX2lkAG9wcm92aWRlcl90eG5faWQAbHJlZGlyZWN0X3VybABncmVmdW5kc4IACWpyZXR1cm5fdXJsAGVzdGF0ZQBqdXBkYXRlZF9hdACYKGpjYW5jZWxfdXJsAHRjbGllbnRfc2VjcmV0X3N1ZmZpeABqY3JlYXRlZF9hdABoY3VzdG9tZXIEa2N1c3RvbWVyX2lkAGpleHBpcmVzX2F0AGJpZABobGl2ZW1vZGUAa21lcmNoYW50X2lkAG5wYXltZW50X2ludGVudAdxcGF5bWVudF9pbnRlbnRfaWQAbnBheW1lbnRfc3RhdHVzAG9wdWJsaXNoYWJsZV9rZXkAbHJldHVybl90b2tlbgBqcmV0dXJuX3VybABjc2VxAGZzdGF0dXMAa3N1Y2Nlc3NfdXJsAGd1aV9tb2RlAGp1cGRhdGVkX2F0AIxnY2hhcmdlc4IAAWRjb2RlAGhleHBvbmVudABubGVkZ2VyX2VudHJpZXOCAAVvcGF5bWVudF9pbnRlbnRzggAHZ3JlZnVuZHOCAAmYJmxhZGRyZXNzX2NpdHkAb2FkZHJlc3NfY291bnRyeQB4GWFkZHJlc3NfbGF0aXR1ZGVfbWljcm9kZWcAbWFkZHJlc3NfbGluZTEAbWFkZHJlc3NfbGluZTIAeBphZGRyZXNzX2xvbmdpdHVkZV9taWNyb2RlZwBzYWRkcmVzc19wb3N0YWxfY29kZQBtYWRkcmVzc19zdGF0ZQBtYW5vbnltaXplZF9hdABqY3JlYXRlZF9hdABlZW1haWwAYmlkAGxsYXN0X3VzZWRfYXQAaGxpdmVtb2RlAGttZXJjaGFudF9pZABkbmFtZQBlcGhvbmUAY3NlcQBqdXBkYXRlZF9hdACSZ2FjY291bnQAZmFtb3VudABoY3VycmVuY3kDbWN1cnJlbmN5X2NvZGUAaWRpcmVjdGlvbgBiaWQAa21lcmNoYW50X2lkAGt0cmFuc2FjdGlvbgZudHJhbnNhY3Rpb25faWQAimZjaGFyZ2UBaWNoYXJnZV9pZABqY3JlYXRlZF9hdABnZW50cmllc4IABWJpZACYLGZhbW91bnQAb2Ftb3VudF9yZWNlaXZlZAB1YW1vdW50X3JlZnVuZF9wZW5kaW5nAG9hbW91bnRfcmVmdW5kZWQAZ2NoYXJnZXOCAAFxY2hlY2tvdXRfc2Vzc2lvbnOCAAJ0Y2xpZW50X3NlY3JldF9zdWZmaXgAamNyZWF0ZWRfYXQAaGN1cnJlbmN5A21jdXJyZW5jeV9jb2RlAGhjdXN0b21lcgRrY3VzdG9tZXJfaWQAa2Rlc2NyaXB0aW9uAGJpZAB3bGFzdF9wYXltZW50X2Vycm9yX2NvZGUAeBpsYXN0X3BheW1lbnRfZXJyb3JfbWVzc2FnZQBobGl2ZW1vZGUAa21lcmNoYW50X2lkAGdyZWZ1bmRzggAJY3NlcQBmc3RhdHVzAGp1cGRhdGVkX2F0AJJnY2hhcmdlc4IAAWRjb2RlAHJkZWxpdmVyc19jYWxsYmFja3MAbGRpc3BsYXlfbmFtZQBnZW5hYmxlZABkZmxvdwB1cmVxdWlyZXNfaXBfYWxsb3dsaXN0AHgYc3VwcG9ydHNfcGFydGlhbF9yZWZ1bmRzAHBzdXBwb3J0c19yZWZ1bmRzAJggZmFtb3VudABmY2hhcmdlAWljaGFyZ2VfaWQAamNyZWF0ZWRfYXQAaGN1cnJlbmN5A21jdXJyZW5jeV9jb2RlAGxmYWlsdXJlX2NvZGUAa2ZhaWx1cmVfcmF3AGNmZWUAYmlkAG5wYXltZW50X2ludGVudAdxcGF5bWVudF9pbnRlbnRfaWQAdXByb3ZpZGVyX3JlZmVyZW5jZV9pZABmcmVhc29uAGZzdGF0dXMAanVwZGF0ZWRfYXQAkmNhbGcAamV4cGlyZXNfYXQAYmlkAGVsYWJlbABpcGFpcmVkX2F0AGpwdWJsaWNfa2V5AGpyZXZva2VkX2F0AG9zdGFmZl9tZW1iZXJfaWQAanRodW1icHJpbnQAlmdjb3VudGVyAGpjcmVhdGVkX2F0AGpleHBpcmVzX2F0AGJpZABmaXNzdWVyAGRraW5kAGhtYXRlcmlhbABrbXVzdF9jaGFuZ2UAb3N0YWZmX21lbWJlcl9pZABnc3ViamVjdABqdXBkYXRlZF9hdACGaWNsaWVudF9pZABrZGlzYWJsZWRfYXQAZnJlYXNvbgCEamV4cGlyZXNfYXQAYmlkAJBqY3JlYXRlZF9hdABsZmFub3V0X3N0YXRlAGJpZABobGl2ZW1vZGUAa21lcmNoYW50X2lkAGlvYmplY3RfaWQAY3NlcQBkdHlwZQCYGGZhbW91bnQAamNyZWF0ZWRfYXQAbWN1cnJlbmN5X2NvZGUAa2Rlc2NyaXB0aW9uAGJpZABqaW52b2ljZV9pZABobGl2ZW1vZGUAa21lcmNoYW50X2lkAGhxdWFudGl0eQBjc2VxAGt1bml0X2Ftb3VudABqdXBkYXRlZF9hdACYLGphbW91bnRfZHVlAGthbW91bnRfcGFpZABvYW1vdW50X3JlZnVuZGVkAHBhbW91bnRfcmVtYWluaW5nAGpjcmVhdGVkX2F0AG1jdXJyZW5jeV9jb2RlAGtjdXN0b21lcl9pZABrZGVzY3JpcHRpb24AaGR1ZV9kYXRlAGxmaW5hbGl6ZWRfYXQAYmlkAGhsaXZlbW9kZQB3bWFya2VkX3VuY29sbGVjdGlibGVfYXQAa21lcmNoYW50X2lkAGZudW1iZXIAZ3BhaWRfYXQAcHBhaWRfb3V0X29mX2JhbmQAcXBheW1lbnRfaW50ZW50X2lkAGNzZXEAZnN0YXR1cwBqdXBkYXRlZF9hdABpdm9pZGVkX2F0AJZmYW1vdW50AGpjcmVhdGVkX2F0AG1jdXJyZW5jeV9jb2RlAGJpZABqaW52b2ljZV9pZABobGl2ZW1vZGUAa21lcmNoYW50X2lkAGZtZXRob2QAcHBhaWRfb3V0X29mX2JhbmQAa3JlY2VpdmVkX2F0AGlyZWZlcmVuY2UAmBppY2xpZW50X2lkAG5jb2RlX2NoYWxsZW5nZQB1Y29kZV9jaGFsbGVuZ2VfbWV0aG9kAGljb2RlX2hhc2gAa2NvbnN1bWVkX2F0AGpjcmVhdGVkX2F0AGpleHBpcmVzX2F0AGttZXJjaGFudF9pZABlbm9uY2UAbHJlZGlyZWN0X3VyaQBlc2NvcGUAanNlc3Npb25faWQAaHN0YWZmX2lkAIxoYXR0ZW1wdHMAYmlkAGVzY29wZQBqdXBkYXRlZF9hdABxd2luZG93X2V4cGlyZXNfYXQAcXdpbmRvd19zdGFydGVkX2F0AJJqY3JlYXRlZF9hdABsZGlzcGxheV9uYW1lAGVlbWFpbABiaWQAaGlzX2FkbWluAG9sYXN0X3NpZ25faW5fYXQAa21lcmNoYW50X2lkAGZzdGF0dXMAanVwZGF0ZWRfYXQAkGxhY2Nlc3NfdG9rZW4Ad2FjY2Vzc190b2tlbl9leHBpcmVzX2F0AGpjcmVhdGVkX2F0AGpleHBpcmVzX2F0AGJpZABsbGFzdF9zZWVuX2F0AGhzdGFmZl9pZABlc3RhdGUAjm1jb2RlX3ZlcmlmaWVyAGpjcmVhdGVkX2F0AGpleHBpcmVzX2F0AGJpZABoaXNfcmV0cnkAam5vbmNlX2hhc2gAanN0YXRlX2hhc2gAlmpjcmVhdGVkX2F0AGtlbmRwb2ludF9pZABoZXZlbnRfaWQAYmlkAG9uZXh0X2F0dGVtcHRfYXQAbnBheWxvYWRfc2hhMjU2AGxyZXNwb25kZWRfYXQAcHJlc3BvbnNlX2V4Y2VycHQAZ3NlbnRfYXQAZXN0YXRlAGN1cmwAlGZhY3Rpb24AaGFjdG9yX2lkAGphY3Rvcl9raW5kAGJpZABrb2NjdXJyZWRfYXQAbG9wZXJhdGlvbl9pZABnb3V0Y29tZQBsdGFyZ2V0X2NsYXNzAGt0YXJnZXRfcmVwcgBmdGVuYW50AIRiaWQAb2lkZW1wb3RlbmN5X2tleQCEZGNvZGUAZ21lc3NhZ2UAhGRraW5kAG9yZWRpcmVjdF90b191cmwYHpgaZmFtb3VudABnY3JlYXRlZABoY3VycmVuY3kAaGN1c3RvbWVyAGtkZXNjcmlwdGlvbgBiaWQAcmxhc3RfcGF5bWVudF9lcnJvchgaaGxpdmVtb2RlAGhtZXRhZGF0YYIAGB9rbmV4dF9hY3Rpb24YG3ZwYXltZW50X21ldGhvZF9kZXRhaWxzGB10cGF5bWVudF9tZXRob2RfdHlwZXMAZnN0YXR1cwCEa21ldGhvZF90eXBlAGRyYWlsAIRqcmV0dXJuX3VybABjdXJsAIRja2V5AGV2YWx1ZQCCZGFyZ3MYGYRiaWQAb2lkZW1wb3RlbmN5X2tleQCSZmFtb3VudABnY3JlYXRlZABoY3VycmVuY3kAY2ZlZQBiaWQAaG1ldGFkYXRhggAYI25wYXltZW50X2ludGVudABmcmVhc29uAGZzdGF0dXMAhGNrZXkAZXZhbHVlAIJkYXJncxghimJpZABvaWRlbXBvdGVuY3lfa2V5AGxwYXllcl9maWVsZHOCABgsc3BheW1lbnRfbWV0aG9kX3R5cGUAanJldHVybl91cmwAhGRjb2RlAGdtZXNzYWdlAIRka2luZABvcmVkaXJlY3RfdG9fdXJsGCqYGmZhbW91bnQAZ2NyZWF0ZWQAaGN1cnJlbmN5AGhjdXN0b21lcgBrZGVzY3JpcHRpb24AYmlkAHJsYXN0X3BheW1lbnRfZXJyb3IYJmhsaXZlbW9kZQBobWV0YWRhdGGCABgra25leHRfYWN0aW9uGCd2cGF5bWVudF9tZXRob2RfZGV0YWlscxgpdHBheW1lbnRfbWV0aG9kX3R5cGVzAGZzdGF0dXMAhGttZXRob2RfdHlwZQBkcmFpbACEanJldHVybl91cmwAY3VybACEY2tleQBldmFsdWUAhGRuYW1lAGV2YWx1ZQCCZGFyZ3MYJY5mYW1vdW50AHJkZXN0aW5hdGlvbl9maWVsZHOCABgxcGRlc3RpbmF0aW9uX3R5cGUAb2lkZW1wb3RlbmN5X2tleQBobWV0YWRhdGGCABgwbnBheW1lbnRfaW50ZW50AGZyZWFzb24AkmZhbW91bnQAZ2NyZWF0ZWQAaGN1cnJlbmN5AGNmZWUAYmlkAGhtZXRhZGF0YYIAGDBucGF5bWVudF9pbnRlbnQAZnJlYXNvbgBmc3RhdHVzAIRja2V5AGV2YWx1ZQCEZG5hbWUAZXZhbHVlAIJkYXJncxguhGRjb2RlAGdtZXNzYWdlAIRka2luZABvcmVkaXJlY3RfdG9fdXJsGDeYGmZhbW91bnQAZ2NyZWF0ZWQAaGN1cnJlbmN5AGhjdXN0b21lcgBrZGVzY3JpcHRpb24AYmlkAHJsYXN0X3BheW1lbnRfZXJyb3IYM2hsaXZlbW9kZQBobWV0YWRhdGGCABg4a25leHRfYWN0aW9uGDR2cGF5bWVudF9tZXRob2RfZGV0YWlscxg2dHBheW1lbnRfbWV0aG9kX3R5cGVzAGZzdGF0dXMAhGttZXRob2RfdHlwZQBkcmFpbACEanJldHVybl91cmwAY3VybACEY2tleQBldmFsdWUAjmZhbW91bnQAaGN1cnJlbmN5AGhjdXN0b21lcgBrZGVzY3JpcHRpb24Ab2lkZW1wb3RlbmN5X2tleQBobWV0YWRhdGGCABg4dHBheW1lbnRfbWV0aG9kX3R5cGVzAIJkYXJncxg5gmJpZACIa2NyZWF0ZWRfZ3RlAGtjcmVhdGVkX2x0ZQBucGF5bWVudF9zdGF0dXMAZnN0YXR1cwCWamNyZWF0ZWRfYXQAa2N1c3RvbWVyX2lkAGpleHBpcmVzX2F0AGJpZABobGl2ZW1vZGUAa21lcmNoYW50X2lkAHFwYXltZW50X2ludGVudF9pZABucGF5bWVudF9zdGF0dXMAZnN0YXR1cwBndWlfbW9kZQBqdXBkYXRlZF9hdACEZHBhZ2UAZmZpbHRlchg8iGtjcmVhdGVkX2d0ZQBrY3JlYXRlZF9sdGUAZWVtYWlsAGVwaG9uZQCSamNyZWF0ZWRfYXQAZWVtYWlsAGJpZABsbGFzdF91c2VkX2F0AGhsaXZlbW9kZQBrbWVyY2hhbnRfaWQAZG5hbWUAZXBob25lAGp1cGRhdGVkX2F0AIRkcGFnZQBmZmlsdGVyGD+Ga2NyZWF0ZWRfZ3RlAGtjcmVhdGVkX2x0ZQBmc3RhdHVzAJgcZmFtb3VudABvYW1vdW50X3JlY2VpdmVkAHVhbW91bnRfcmVmdW5kX3BlbmRpbmcAb2Ftb3VudF9yZWZ1bmRlZABqY3JlYXRlZF9hdABtY3VycmVuY3lfY29kZQBrY3VzdG9tZXJfaWQAa2Rlc2NyaXB0aW9uAGJpZAB3bGFzdF9wYXltZW50X2Vycm9yX2NvZGUAaGxpdmVtb2RlAGttZXJjaGFudF9pZABmc3RhdHVzAGp1cGRhdGVkX2F0AIRkcGFnZQBmZmlsdGVyGEKGa2NyZWF0ZWRfZ3RlAGtjcmVhdGVkX2x0ZQBmc3RhdHVzAJgYZmFtb3VudABpY2hhcmdlX2lkAGpjcmVhdGVkX2F0AG1jdXJyZW5jeV9jb2RlAGxmYWlsdXJlX2NvZGUAY2ZlZQBiaWQAcXBheW1lbnRfaW50ZW50X2lkAHVwcm92aWRlcl9yZWZlcmVuY2VfaWQAZnJlYXNvbgBmc3RhdHVzAGp1cGRh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";

export interface CratestackClientOptions {
  basePath?: string;
  fetch?: typeof fetch;
  headers?: HeadersInit | (() => HeadersInit | Promise<HeadersInit>);
  /** Opt in to compact wire keys: schema-declared fields travel as integer
   *  keys (`application/vnd.cratestack.compact+json`). Default `false`, which
   *  leaves the wire byte-identical to a client without the option. The
   *  request body goes compact only after the op has answered compact once,
   *  so an older server is never sent a body it cannot read. Loads the
   *  optional peer `@cratestack/cbor` on first use; without it, or with a
   *  release that predates `createCompactKeys`, the option is silently
   *  inert. Error bodies stay named. */
  compactKeys?: boolean;
}

/** Serializable request prepared before its first network attempt. */
export interface PreparedCall {
  version: 1;
  method: string;
  path: string;
  url: string;
  op: string;
  contract: string;
  keyTables: string;
  headers: [string, string][];
  body: string | null;
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

const RESERVED_ERROR_MEMBERS: ReadonlySet<string> = new Set(["code", "message", "details"]);

/** The members of a decoded error body beyond `code`, `message` and
 *  `details`: what an `AuthProvider` refusal adds (cratestack#1208), such
 *  as `{ error: "attestation_expired" }`. Every member is kept, as decoded;
 *  empty when `body` is not an object. */
function extraErrorMembers(body: unknown): Readonly<Record<string, unknown>> {
  const extra: Record<string, unknown> = {};
  if (typeof body === "object" && body !== null && !Array.isArray(body)) {
    for (const [name, value] of Object.entries(body)) {
      if (!RESERVED_ERROR_MEMBERS.has(name)) {
        extra[name] = value;
      }
    }
  }
  return Object.freeze(extra);
}

/** The answer's `WWW-Authenticate` value, raw and unparsed (cratestack#1243);
 *  empty when there is none. `Headers` joins two challenge lines with `, `,
 *  so they arrive as one entry holding both. */
function challengesOf(headers: Headers | undefined): readonly string[] {
  const value = headers?.get("www-authenticate");
  return Object.freeze(value ? [value] : []);
}

/** The extra member `error` of a step-up refusal (RFC 9470, cratestack#1244). */
export const INSUFFICIENT_USER_AUTHENTICATION = "insufficient_user_authentication";

/** What a step-up refusal asks for (cratestack#1244): an operation that
 *  requires a fresher or stronger authentication than the caller presented.
 *  Each requirement is present only when the refused operation declares it. */
export interface StepUpRequired {
  /** The longest time, in seconds, since the caller last authenticated. */
  readonly maxAge: number | undefined;
  /** The authentication context classes the operation accepts. */
  readonly acrValues: readonly string[];
  /** The authentication methods the operation requires. */
  readonly amr: readonly string[];
  /** Which requirements the caller failed: a subset of `"max_age"`, `"acr"`
   *  and `"amr"`. */
  readonly unmet: readonly string[];
}

/** Reads a step-up refusal once the transport's own test (`refused`: a 401,
 *  or `unauthenticated`) passed: `undefined` unless the extra member `error`
 *  is `insufficient_user_authentication`. Never throws: a missing or
 *  mistyped field reads as absent or empty. */
function readStepUp(
  refused: boolean,
  details: unknown,
  extra: Readonly<Record<string, unknown>>,
): StepUpRequired | undefined {
  if (!refused || extra["error"] !== INSUFFICIENT_USER_AUTHENTICATION) {
    return undefined;
  }
  const map: Record<string, unknown> =
    typeof details === "object" && details !== null && !Array.isArray(details)
      ? (details as Record<string, unknown>)
      : {};
  const strings = (name: string): readonly string[] => {
    const value = map[name];
    return Object.freeze(
      Array.isArray(value) ? value.filter((item): item is string => typeof item === "string") : [],
    );
  };
  const raw = map["max_age"];
  const maxAge = typeof raw === "bigint" ? Number(raw) : raw;
  return Object.freeze({
    maxAge:
      typeof maxAge === "number" && Number.isSafeInteger(maxAge) && maxAge >= 0
        ? maxAge
        : undefined,
    acrValues: strings("acr_values"),
    amr: strings("amr"),
    unmet: strings("unmet"),
  });
}

/** Runs `call`; when it fails with a step-up refusal, hands the refusal to
 *  `reprove` and, when that resolves `true` (the user authenticated again),
 *  runs `call` **once** more and returns what it gives, even a second
 *  refusal. Never loops (cratestack#1244).
 *
 *  `reprove` resolves `false` when the user declined or the re-proof failed;
 *  the first refusal is then rethrown. Any other error, and one `reprove`
 *  itself throws, propagates as it is.
 *
 *  ```ts
 *  const receipt = await retryAfterStepUp(
 *    () => client.procedures.transfer(args),
 *    (stepUp) => session.reauthenticate({ maxAge: stepUp.maxAge }),
 *  );
 *  ``` */
export async function retryAfterStepUp<T>(
  call: () => Promise<T>,
  reprove: (stepUp: StepUpRequired) => Promise<boolean>,
): Promise<T> {
  try {
    return await call();
  } catch (error) {
    const stepUp = stepUpOf(error);
    if (stepUp === undefined || !(await reprove(stepUp))) {
      throw error;
    }
  }
  return call();
}

/** The error envelope a CrateStack server answers with. Mirrors
 *  `cratestack_core::CratestackErrorResponse`; the members an
 *  `AuthProvider` refusal adds beside these three are on
 *  {@link CratestackHttpError.extra}. */
export interface CratestackErrorResponse {
  code: string;
  message: string;
  details?: unknown;
}

function isErrorResponse(payload: unknown): payload is CratestackErrorResponse {
  return (
    typeof payload === "object" &&
    payload !== null &&
    typeof (payload as { code?: unknown }).code === "string"
  );
}

/** Thrown when the server answers with a non-2xx status. `payload` is the
 *  whole decoded body; the envelope's members are typed beside it: `body`
 *  (`code`, `message`, `details`), `extra` (every other member, as an
 *  `AuthProvider` refusal adds them) and `challenges` (the
 *  `WWW-Authenticate` value). */
export class CratestackHttpError extends Error {
  readonly status: number;
  readonly response: Response;
  readonly payload: unknown;
  /** The decoded envelope, or `undefined` when the payload is not one. */
  readonly body: CratestackErrorResponse | undefined;
  /** The envelope's `code` (`UNAUTHORIZED`, `FORBIDDEN`, ...). */
  readonly code: string | undefined;
  readonly details: unknown;
  readonly extra: Readonly<Record<string, unknown>>;
  readonly challenges: readonly string[];
  /** The step-up refusal this answer is (cratestack#1244): a 401 whose
   *  extra member `error` is `insufficient_user_authentication`, read from
   *  `details`; `undefined` for any other error. */
  readonly stepUp: StepUpRequired | undefined;

  constructor(response: Response, payload: unknown) {
    super(`CrateStack request failed with status ${response.status}`);
    this.name = "CratestackHttpError";
    this.status = response.status;
    this.response = response;
    this.payload = payload;
    this.body = isErrorResponse(payload) ? payload : undefined;
    this.code = this.body?.code;
    this.details = this.body?.details;
    this.extra = extraErrorMembers(this.body);
    this.challenges = challengesOf(response.headers);
    this.stepUp = readStepUp(this.status === 401, this.details, this.extra);
  }
}

/** The step-up refusal of a {@link CratestackHttpError} (its `stepUp`);
 *  `undefined` for anything else (cratestack#1244). */
export function stepUpOf(error: unknown): StepUpRequired | undefined {
  return error instanceof CratestackHttpError ? error.stepUp : undefined;
}

export class CratestackRuntime {
  readonly origin: string;
  readonly basePath: string;
  readonly fetchFn: typeof fetch;
  readonly defaultHeaders: HeadersInit | (() => HeadersInit | Promise<HeadersInit>) | undefined;
  private readonly compact: RestCompactSession | undefined;

  constructor(origin: string, options: CratestackClientOptions = {}) {
    this.origin = trimTrailingSlashes(origin);
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
    this.compact =
      options.compactKeys === true
        ? new RestCompactSession(KEY_TABLES, Object.keys(OP_CONTRACTS))
        : undefined;
  }

  /** Prepare a keyed request for caller-owned durable storage before sending. */
  async prepare(method: string, path: string, options: CratestackRequestOptions = {}): Promise<PreparedCall> {
    if (!["POST", "PUT", "PATCH", "DELETE"].includes(method.toUpperCase())) throw new Error("prepare requires a mutation");
    const target = new URL(this.url(path, options.query));
    if (target.username || target.password) throw new Error("prepared URL must not contain userinfo");
    const headers = new Headers(await resolveHeaders(this.defaultHeaders));
    for (const [key, value] of new Headers(options.headers)) headers.set(key, value);
    if (!headers.get("Idempotency-Key")) throw new Error("prepare requires an Idempotency-Key");
    const routes = this.compact ?? new RestCompactSession(KEY_TABLES, Object.keys(OP_CONTRACTS));
    const op = routes.opOf(method, path);
    const contract = op === undefined ? undefined : OP_CONTRACTS[op];
    if (op === undefined || contract === undefined) throw new Error("unknown prepared route");
    let body = options.body === undefined ? null : JSON.stringify(encodeBinaryAsJson(options.body));
    headers.set("Accept", "application/json");
    if (body !== null) headers.set("Content-Type", "application/json");
    const compact = await this.compact?.begin(method, path, headers.get("Idempotency-Key") ?? undefined);
    if (compact !== undefined) {
      compact.applyHeaders(headers, body !== null);
      if (body !== null) body = compact.encodeBody(body);
    }
    if (body !== null && new TextEncoder().encode(body).length > 8 * 1024 * 1024) throw new Error("prepared body too large");
    for (const name of [...headers.keys()]) if (!["idempotency-key", "if-match", "accept", "content-type", "cratestack-contract"].includes(name)) headers.delete(name);
    const prepared: PreparedCall = { version: 1, method: method.toUpperCase(), path, url: target.toString(), op, contract, keyTables: KEY_TABLES, headers: [...headers], body };
    if (new TextEncoder().encode(JSON.stringify(prepared)).length > 16 * 1024 * 1024) throw new Error("prepared call too large");
    return prepared;
  }

  async executePrepared<T>(prepared: PreparedCall, signal?: AbortSignal): Promise<T> {
    const serialized = JSON.stringify(prepared);
    if (new TextEncoder().encode(serialized).length > 16 * 1024 * 1024) throw new Error("prepared call too large");
    prepared = JSON.parse(serialized) as PreparedCall;
    if (Object.keys(prepared).sort().join(",") !== "body,contract,headers,keyTables,method,op,path,url,version" || prepared.version !== 1 ||
      typeof prepared.method !== "string" || typeof prepared.path !== "string" || typeof prepared.url !== "string" ||
      typeof prepared.op !== "string" || typeof prepared.contract !== "string" || prepared.contract !== OP_CONTRACTS[prepared.op] || prepared.keyTables !== KEY_TABLES ||
      !Array.isArray(prepared.headers) || (prepared.body !== null && typeof prepared.body !== "string")) {
      throw new Error("invalid or incompatible prepared call");
    }
    const target = new URL(prepared.url);
    if (target.username || target.password) throw new Error("prepared URL must not contain userinfo");
    if (prepared.body !== null) JSON.parse(prepared.body);
    const names = new Set<string>();
    for (const pair of prepared.headers) {
      if (!Array.isArray(pair) || pair.length !== 2 || typeof pair[0] !== "string" || typeof pair[1] !== "string") throw new Error("invalid prepared header");
      const name = pair[0].toLowerCase();
      if (names.has(name) || !["idempotency-key", "if-match", "accept", "content-type", "cratestack-contract"].includes(name) || new TextEncoder().encode(pair[1]).length > 8192) throw new Error("invalid prepared header");
      names.add(name);
    }
    const routes = this.compact ?? new RestCompactSession(KEY_TABLES, Object.keys(OP_CONTRACTS));
    if (!["POST", "PUT", "PATCH", "DELETE"].includes(prepared.method) || routes.opOf(prepared.method, prepared.path) !== prepared.op ||
      this.url(prepared.path).split("?")[0] !== prepared.url.split("?")[0] ||
      (prepared.body !== null && new TextEncoder().encode(prepared.body).length > 8 * 1024 * 1024)) throw new Error("invalid prepared route or body");
    const saved = new Headers(prepared.headers);
    if (!saved.get("Idempotency-Key")) throw new Error("invalid prepared key");
    const headers = new Headers(await resolveHeaders(this.defaultHeaders));
    if (SCHEMA_SHA256 !== "") headers.set(SCHEMA_SHA_HEADER, SCHEMA_SHA256);
    for (const name of ["idempotency-key", "if-match", "accept", "content-type", "cratestack-contract"]) headers.delete(name);
    for (const [key, value] of saved) headers.set(key, value);
    const compact = await this.compact?.begin(prepared.method, prepared.path, headers.get("Idempotency-Key") ?? undefined);
    if (headers.has("Cratestack-Contract") && compact === undefined) throw new Error("compact decoder unavailable");
    const expected = new Headers();
    if (compact !== undefined) compact.applyHeaders(expected, prepared.body !== null);
    const contentType = saved.get("content-type");
    if (saved.get("accept") !== (compact ? expected.get("accept") : "application/json") ||
        saved.get("cratestack-contract") !== expected.get("cratestack-contract") ||
        (prepared.body !== null && contentType !== "application/json" && contentType !== "application/vnd.cratestack.compact+json") ||
        (contentType === "application/vnd.cratestack.compact+json" && compact === undefined)) throw new Error("invalid prepared media or selector");

    const response = await this.fetchFn(prepared.url, { method: prepared.method, headers, body: prepared.body, signal: signal ?? null });
    const payload = await readResponsePayload(response, compact);
    if (!response.ok) throw new CratestackHttpError(response, payload);
    return payload as T;
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

    const effectiveHeaders = new Headers(headers);
    for (const [key, value] of new Headers(options.headers)) effectiveHeaders.set(key, value);
    const compact = await this.compact?.begin(method, path, effectiveHeaders.get("Idempotency-Key") ?? undefined);
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
    if (compact !== undefined) {
      compact.applyHeaders(headers, body !== undefined);
      if (typeof body === "string") {
        body = compact.encodeBody(body);
      }
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

    if (!response.ok) {
      // A `415` sends the op back to named bodies (see `./compact`).
      compact?.noteFailure(response.status);
    }
    const payload = await readResponsePayload(response, compact);
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
    const normalizedBase = trimTrailingSlashes(this.basePath);
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

async function readResponsePayload(
  response: Response,
  compact?: RestCompactCall,
): Promise<unknown> {
  if (response.status === 204) {
    return undefined;
  }

  const text = await response.text();
  if (text.length === 0) {
    return undefined;
  }

  const contentType = response.headers.get("Content-Type") ?? "";
  // A compact body is transcoded back to plain JSON; an error body never is.
  if (
    compact !== undefined &&
    response.ok &&
    contentType.split(";", 1)[0]?.trim() === COMPACT_JSON_CONTENT_TYPE
  ) {
    return JSON.parse(compact.decodeResponse(text));
  }
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

function trimTrailingSlashes(value: string): string {
  let end = value.length;
  while (end > 0 && value.charCodeAt(end - 1) === 47) {
    end--;
  }
  return value.slice(0, end);
}