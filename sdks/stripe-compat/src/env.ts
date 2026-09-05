/**
 * How this suite is told which vpay to talk to, and as whom.
 *
 * Five variables, and no defaults for the two that are credentials-shaped. A
 * default `clientId` or key path would let the suite start against a stack
 * it was not configured for and fail somewhere deep inside a token exchange;
 * a missing one should say so in its first line. `VPAY_BASE_URL` and
 * `VPAY_RECEIVER_URL` do have defaults because they name *locations*, not
 * identities, and getting either wrong is immediately visible.
 *
 * `MERCHANT_WEBHOOK_SECRET` is the exception that proves the rule: it is
 * credential-shaped and still defaulted, because the value in question is a
 * placeholder written in plain sight in `compose.e2e.yml` for a stub receiver
 * on a `livemode: false` stack. It is the same default, spelled the same way,
 * that `examples/merchant-demo` uses — see that file's
 * `DEFAULT_WEBHOOK_SECRET` for the full argument.
 */
import { readFileSync } from "node:fs";

/** Everything the suite needs to build a `Stripe` client. */
export interface CompatEnv {
  /** e.g. `http://localhost:18080`. What `createStripeAuthenticator` derives the OP endpoints from. */
  readonly baseUrl: string;
  /** `baseUrl` split up, because stripe-node takes the three separately. */
  readonly host: string;
  readonly port: string;
  readonly protocol: "http" | "https";
  /** The registered merchant `client_id` (`just gen-demo-keys` writes `demo-merchant`). */
  readonly clientId: string;
  /** PEM text of that merchant's private key. Read once, never logged. */
  readonly privateKeyPem: string;
  /** Where it was read from — for error messages only. */
  readonly privateKeyPath: string;
  /**
   * The WireMock receiver `.e2e/application-demo.yml` points this merchant's
   * one webhook endpoint at, as published on the host.
   *
   * The webhook case reads its request *journal* (`GET /__admin/requests`),
   * which is the merchant-side view: what actually arrived, headers and body,
   * rather than what vpay believes it sent.
   */
  readonly receiverUrl: string;
  /** The secret that endpoint's deliveries are signed with. */
  readonly webhookSecret: string;
}

/** The default base URL, matching `just stripe-compat`'s published port. */
export const DEFAULT_BASE_URL = "http://localhost:18080";

/**
 * The default receiver URL, matching the justfile's `demo_receiver_port`
 * (which `just stripe-compat` does not move, unlike `demo_port`).
 */
export const DEFAULT_RECEIVER_URL = "http://localhost:8083";

/**
 * The stub secret `compose.e2e.yml` hands both vpay processes as
 * `MERCHANT_WEBHOOK_SECRET`, which the demo overlay's
 * `webhooks[0].secrets` resolves to.
 *
 * Over `vpay_config`'s 32-byte livemode floor even though this stack is not
 * livemode, so the value most likely to be copied is not one a livemode boot
 * guard would then refuse for a reason unrelated to why it is wrong.
 */
export const DEFAULT_WEBHOOK_SECRET = "wiremock-stub-webhook-secret-32-bytes";

function required(name: string): string {
  const value = process.env[name];
  if (value === undefined || value.trim() === "") {
    throw new Error(
      `@vaam-apps/vpay-stripe-compat: ${name} is not set. This suite runs against a REAL vpay stack; ` +
        `bring one up with \`just stripe-compat\`, which sets all three of VPAY_BASE_URL, ` +
        `VPAY_MERCHANT_CLIENT_ID and VPAY_MERCHANT_PRIVATE_KEY_PATH for you.`,
    );
  }
  return value;
}

/**
 * Reads the environment, or throws.
 *
 * Deliberately throws rather than returning `undefined` for anything: a
 * conformance suite that quietly degrades to "no stack, nothing to check" is
 * the failure mode AGENTS.md rule 2 exists to prevent.
 */
export function readCompatEnv(): CompatEnv {
  const baseUrl = process.env["VPAY_BASE_URL"] ?? DEFAULT_BASE_URL;
  const url = new URL(baseUrl);
  if (url.protocol !== "http:" && url.protocol !== "https:") {
    throw new Error(
      `@vaam-apps/vpay-stripe-compat: VPAY_BASE_URL must be http or https, got ${baseUrl}`,
    );
  }
  const protocol = url.protocol === "https:" ? "https" : "http";
  const port = url.port !== "" ? url.port : protocol === "https" ? "443" : "80";

  const clientId = required("VPAY_MERCHANT_CLIENT_ID");
  const privateKeyPath = required("VPAY_MERCHANT_PRIVATE_KEY_PATH");
  let privateKeyPem: string;
  try {
    privateKeyPem = readFileSync(privateKeyPath, "utf8");
  } catch (cause) {
    throw new Error(
      `@vaam-apps/vpay-stripe-compat: cannot read the merchant private key at ${privateKeyPath}. ` +
        `\`just gen-demo-keys\` writes one to .e2e/demo-merchant/oauth-signing-key.pem.`,
      { cause },
    );
  }

  const receiverUrl = new URL(
    process.env["VPAY_RECEIVER_URL"] ?? DEFAULT_RECEIVER_URL,
  ).origin;
  const webhookSecret =
    process.env["MERCHANT_WEBHOOK_SECRET"] ?? DEFAULT_WEBHOOK_SECRET;

  return {
    baseUrl: url.origin,
    host: url.hostname,
    port,
    protocol,
    clientId,
    privateKeyPem,
    privateKeyPath,
    receiverUrl,
    webhookSecret,
  };
}
