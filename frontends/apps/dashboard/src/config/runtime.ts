/**
 * The four settings this container serves, read once at start.
 *
 * The only file under `src/config/` that touches the environment, and it is
 * deliberately thin — `settings.ts` decides what a configuration *means*,
 * and is a unit test.
 *
 * **Bracket notation, and it is load-bearing.** Next inlines
 * `process.env.FOO` written as a static member expression at *build* time, so
 * a value baked into the image is not a value an operator can change. Every
 * read here goes through `process.env[…]` with a variable key, which the
 * compiler cannot inline. The same reason `frontends/apps/checkout`'s own
 * runtime config states at length.
 *
 * **Once, at start.** The result is memoised at module scope: the read
 * happens on the first import in the server process and never again. A
 * configuration change is a pod restart, exactly as it is for `vpay-server`
 * (ADR-0003).
 */
import { assembleConfig, type AssembledConfig } from "./settings";

let memo: AssembledConfig | null = null;

/**
 * What this container loaded — or `config: null` and the reasons.
 *
 * Server-side only. Nothing here is ever passed to a browser: the API base
 * URL names a host only this process can reach, and the client id and
 * redirect URI are needed by the two OAuth legs, both of which run here.
 */
export function dashboardConfig(): AssembledConfig {
  if (memo !== null) {
    return memo;
  }
  const assembled = assembleConfig(process.env);
  for (const problem of assembled.problems) {
    // eslint-disable-next-line no-console -- the loud line this feature exists for. Server-side, at start, naming a variable and nothing else: no session, no staff member, no credential has ever passed through here.
    console.warn(
      `[vpay-dashboard] configuration: ${problem.variable} is not set — ${problem.detail}`,
    );
  }
  memo = assembled;
  return memo;
}
