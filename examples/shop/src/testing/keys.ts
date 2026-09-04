/** An RSA key pair for the tests' `VpayClient`. Test-only. */
import { generateKeyPairSync } from "node:crypto";

let cached: string | undefined;

/**
 * A 2048-bit PEM, generated once per process. Generation is the slowest thing
 * in this suite; the tests care that the SDK *signs*, not which key it signs
 * with.
 */
export function testPrivateKeyPem(): string {
  cached ??= generateKeyPairSync("rsa", { modulusLength: 2048 })
    .privateKey.export({ type: "pkcs1", format: "pem" })
    .toString();
  return cached;
}
