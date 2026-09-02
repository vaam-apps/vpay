/**
 * The SDK's own version, sent as the `vpay-sdk-node/<version>` User-Agent
 * suffix (docs/flows/merchant-auth.md, "Headers" table).
 *
 * Kept as a hand-written constant rather than read from `package.json` at
 * runtime: this package builds to plain Node ESM via `tsc`
 * (`moduleResolution: "NodeNext"`), and importing JSON across that boundary
 * needs an import attribute whose support has shifted between Node
 * releases. A hand-written constant has no such landmine — it just needs to
 * be bumped alongside `package.json`'s own `version` field.
 */
export const SDK_VERSION = "0.1.0";
