/**
 * What this deployment needs to serve a sign-in, and what it does when a
 * piece of it is missing.
 *
 * Pure: no `process.env`, no filesystem, no `next/*`. Everything that decides
 * what a configuration *means* is here, where it is a unit test;
 * `runtime.ts` is the thin half that reads the environment once at start.
 *
 * # Fail closed, and say so on screen
 *
 * A dashboard whose `client_id` is wrong cannot sign anybody in — the
 * authorization request is refused by `handle_authorize`'s client lookup, and
 * the staff member reads a `400` that names `redirect_uri`. So a missing or
 * blank value is **not** defaulted to a plausible one: `assembleConfig`
 * answers `null`, `/login` renders the reason instead of a form, and nobody
 * types a password into a page that could not have used it.
 *
 * That is the opposite of `frontends/apps/checkout`'s runtime config, which
 * defaults everything and warns — deliberately. A checkout page with no
 * branding still takes a payment; a dashboard with no client registration is
 * a login form that cannot log anyone in.
 */

/** Every value the sign-in and the `/dash/v1` reads need. */
export interface DashboardConfig {
  /**
   * vpay's base URL **as this server reaches it** — a compose service name
   * inside the stack, a cluster-internal name in Kubernetes. It is never
   * sent to a browser: every call that uses it is made server-side.
   */
  readonly apiBaseUrl: string;
  /**
   * The registered dashboard client. `aud` on every token this app obtains
   * (ADR-0017 decision 3), so a value that is not the registration's makes
   * `/dash/v1` refuse every read.
   */
  readonly clientId: string;
  /**
   * The redirect URI, matched **byte for byte** against the registration
   * (`ClientRegistration::allows_redirect_uri` — no prefix, no wildcard).
   *
   * No browser ever visits it. ADR-0017 decision 4 has this app's own server
   * follow the `302`, so this string is an identifier the two OAuth legs
   * must spell identically and not a page. `docs/flows/dashboard.md` says so
   * where a reader would otherwise go looking for the route.
   */
  readonly redirectUri: string;
  /**
   * The single read-only scope the registration grants
   * (`docs/flows/dashboard-auth.md` § Scope). Requested explicitly: an
   * authorization request with no `scope` is granted none, and a token with
   * no scope is refused by `/dash/v1` — a `403` that looks nothing like a
   * configuration mistake.
   */
  readonly scope: string;
}

/** One missing or unusable setting, in the words an operator needs. */
export interface ConfigProblem {
  /** The environment variable, so the fix is copy-pasteable. */
  readonly variable: string;
  /** What is wrong with it. */
  readonly detail: string;
}

/** What `assembleConfig` answers. `config` is `null` if `problems` is non-empty. */
export interface AssembledConfig {
  readonly config: DashboardConfig | null;
  readonly problems: readonly ConfigProblem[];
}

/** vpay's base URL, as this server reaches it. */
export const API_BASE_URL_VAR = 'VPAY_DASH_API';
/** The registered dashboard client id. */
export const CLIENT_ID_VAR = 'VPAY_DASHBOARD_CLIENT_ID';
/** The registered redirect URI, byte for byte. */
export const REDIRECT_URI_VAR = 'VPAY_DASHBOARD_REDIRECT_URI';
/** The registered read-only scope. */
export const SCOPE_VAR = 'VPAY_DASHBOARD_SCOPE';

/**
 * A value that is present and not whitespace, or `null`.
 *
 * An empty string is treated as absent rather than as a value: a
 * `VPAY_DASHBOARD_CLIENT_ID=` line in a compose file is a variable somebody
 * meant to set, and honouring it would send `client_id=` to `/authorize`.
 */
function present(value: string | undefined): string | null {
  if (typeof value !== 'string') {
    return null;
  }
  const trimmed = value.trim();
  return trimmed.length === 0 ? null : trimmed;
}

/**
 * Reads the four settings out of an environment-shaped record.
 *
 * The API base URL is additionally required to parse as an absolute
 * `http`/`https` URL, because everything else in this app is built by
 * appending a path to it — a relative or malformed value would fail at the
 * first `fetch`, inside a page render, as an opaque `TypeError`.
 *
 * The redirect URI is **not** parsed. It is compared byte for byte against a
 * registration this app cannot see, so normalising it here (a trailing
 * slash, a lower-cased host) would make this app disagree with the server
 * about a string whose whole job is to be identical.
 */
export function assembleConfig(env: Readonly<Record<string, string | undefined>>): AssembledConfig {
  const problems: ConfigProblem[] = [];

  const apiBaseUrl = present(env[API_BASE_URL_VAR]);
  if (apiBaseUrl === null) {
    problems.push({
      variable: API_BASE_URL_VAR,
      detail: "vpay's base URL, as this server reaches it. Nothing can be read without it.",
    });
  } else if (!isAbsoluteHttpUrl(apiBaseUrl)) {
    problems.push({
      variable: API_BASE_URL_VAR,
      detail: 'must be an absolute http:// or https:// URL.',
    });
  }

  const clientId = present(env[CLIENT_ID_VAR]);
  if (clientId === null) {
    problems.push({
      variable: CLIENT_ID_VAR,
      detail: "this deployment's registered dashboard_client.client_id.",
    });
  }

  const redirectUri = present(env[REDIRECT_URI_VAR]);
  if (redirectUri === null) {
    problems.push({
      variable: REDIRECT_URI_VAR,
      detail:
        'one of dashboard_client.redirect_uris, byte for byte. It is matched exactly and is never visited.',
    });
  }

  const scope = present(env[SCOPE_VAR]);
  if (scope === null) {
    problems.push({
      variable: SCOPE_VAR,
      detail: "dashboard_client.scope — the one read-only scope this deployment's registration grants.",
    });
  }

  if (
    problems.length > 0 ||
    apiBaseUrl === null ||
    clientId === null ||
    redirectUri === null ||
    scope === null
  ) {
    return { config: null, problems };
  }

  return {
    // The base URL loses a trailing slash so that every path this app
    // appends is one `/` and not two. `//dash/v1/...` is a different path to
    // axum, and it answers this crate's 404 envelope.
    config: {
      apiBaseUrl: apiBaseUrl.replace(/\/+$/, ''),
      clientId,
      redirectUri,
      scope,
    },
    problems: [],
  };
}

/** Absolute, and `http`/`https` — not `file:`, not a bare host. */
function isAbsoluteHttpUrl(value: string): boolean {
  try {
    const url = new URL(value);
    return url.protocol === 'http:' || url.protocol === 'https:';
  } catch {
    return false;
  }
}
