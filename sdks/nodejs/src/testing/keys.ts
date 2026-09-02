/** RSA keypair generation shared by tests. Not shipped — see tsconfig.build.json. */
import { generateKeyPairSync, type KeyObject } from "node:crypto";

export interface TestKeyPair {
  privateKey: KeyObject;
  publicKey: KeyObject;
  privateKeyPem: string;
}

export function generateTestRsaKeyPair(): TestKeyPair {
  const { privateKey, publicKey } = generateKeyPairSync("rsa", {
    modulusLength: 2048,
  });
  const privateKeyPem = privateKey
    .export({ type: "pkcs1", format: "pem" })
    .toString();
  return { privateKey, publicKey, privateKeyPem };
}
