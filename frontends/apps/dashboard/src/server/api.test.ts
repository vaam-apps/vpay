import { afterEach, describe, expect, it, vi } from 'vitest';

import { authorizeRedirect, getJson, postForm, SESSION_HEADER } from './api';

const BASE = 'http://vpay-server:8080';

/**
 * A `fetch` stub that keeps its own signature.
 *
 * `vi.fn(async () => …)` records calls as a zero-length tuple, so
 * `mock.calls[0][1]` — the `RequestInit` every assertion below is about — is
 * not typed at all. The parameters are declared so the recorded call is.
 */
function stub(answer: () => Response) {
  return vi.fn((_input: string | URL | Request, _init?: RequestInit) => Promise.resolve(answer()));
}

/** The `RequestInit` of the nth recorded call. */
function initOf(fetchStub: ReturnType<typeof stub>, index = 0): RequestInit {
  const init = fetchStub.mock.calls[index]?.[1];
  expect(init).toBeDefined();
  return init as RequestInit;
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('a refusal', () => {
  it('takes its sentence from vpay error envelope', async () => {
    vi.stubGlobal(
      'fetch',
      stub(
        () =>
          new Response(
            JSON.stringify({
              error: {
                type: 'invalid_request_error',
                code: 'authentication_error',
                message: 'We could not sign you in.',
              },
            }),
            { status: 401, headers: { 'content-type': 'application/json' } },
          ),
      ),
    );
    const result = await postForm(BASE, '/dash/v1/staff/login', { email: 'a', password: 'b' });
    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.failure.message).toBe('We could not sign you in.');
      expect(result.failure.status).toBe(401);
    }
  });

  it('carries the request id, from either spelling vpay emits', async () => {
    // vpay sends both `request-id` (Stripe's spelling) and `x-request-id`
    // with one value; a deployment behind something that strips one must
    // still show an id, because every refusal on this path is the same
    // sentence and the id is the only thing that tells two apart.
    for (const header of ['request-id', 'x-request-id']) {
      vi.stubGlobal(
        'fetch',
        stub(() => new Response('{}', { status: 500, headers: { [header]: 'req_01J8' } })),
      );
      const result = await getJson(BASE, '/dash/v1/payment_intents');
      expect(result.ok).toBe(false);
      if (!result.ok) {
        expect(result.failure.requestId).toBe('req_01J8');
      }
    }
  });

  it('never renders an unparseable body — a proxy error page is not a message', async () => {
    vi.stubGlobal(
      'fetch',
      stub(() => new Response('<html><body>502 from the ingress</body></html>', { status: 502 })),
    );
    const result = await getJson(BASE, '/dash/v1/payment_intents');
    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.failure.message).not.toContain('<html>');
      expect(result.failure.message).toContain('502');
    }
  });

  it('turns an unreachable vpay into a failure rather than a thrown error', async () => {
    // A rejected fetch inside a server component renders Next own error
    // page, which tells a staff member nothing and an operator less.
    vi.stubGlobal(
      'fetch',
      stub(() => {
        throw new Error('ECONNREFUSED');
      }),
    );
    const result = await getJson(BASE, '/dash/v1/payment_intents');
    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.failure.status).toBe(0);
      expect(result.failure.message).toContain('ECONNREFUSED');
    }
  });
});

describe('the credentials each call presents', () => {
  it('sends a staff session in the header vpay reads, never as a cookie', async () => {
    const fetchStub = stub(() => new Response('{}', { status: 200 }));
    vi.stubGlobal('fetch', fetchStub);
    await getJson(BASE, '/dash/v1/staff/session', { sessionToken: 'tok' });

    const headers = initOf(fetchStub).headers as Record<string, string>;
    expect(headers[SESSION_HEADER]).toBe('tok');
    expect(headers['cookie']).toBeUndefined();
  });

  it('sends the dash token as a bearer', async () => {
    const fetchStub = stub(() => new Response('{}', { status: 200 }));
    vi.stubGlobal('fetch', fetchStub);
    await getJson(BASE, '/dash/v1/payment_intents', { bearer: 'jwt' });

    const headers = initOf(fetchStub).headers as Record<string, string>;
    expect(headers['authorization']).toBe('Bearer jwt');
  });

  it('posts form-encoded, because every staff route extracts axum Form', async () => {
    const fetchStub = stub(() => new Response('{}', { status: 200 }));
    vi.stubGlobal('fetch', fetchStub);
    await postForm(BASE, '/dash/v1/staff/login', { email: 'a@b.test', password: 'p a s s' });

    const init = initOf(fetchStub);
    expect((init.headers as Record<string, string>)['content-type']).toBe(
      'application/x-www-form-urlencoded',
    );
    expect(typeof init.body === 'string' ? init.body : '').toBe(
      'email=a%40b.test&password=p+a+s+s',
    );
  });

  it('never caches a credential step or a payments read', async () => {
    const fetchStub = stub(() => new Response('{}', { status: 200 }));
    vi.stubGlobal('fetch', fetchStub);
    await getJson(BASE, '/dash/v1/payment_intents', { bearer: 'jwt' });
    expect(initOf(fetchStub).cache).toBe('no-store');
  });
});

describe('the authorize leg', () => {
  it('returns the Location of a 302 without following it', async () => {
    vi.stubGlobal(
      'fetch',
      stub(
        () =>
          new Response(null, {
            status: 302,
            headers: { location: 'http://localhost:3000/cb?code=x' },
          }),
      ),
    );
    const result = await authorizeRedirect(BASE, { client_id: 'c' }, 'tok');
    expect(result).toEqual({ ok: true, value: 'http://localhost:3000/cb?code=x' });
  });

  it('refuses a 200 — an authorization endpoint that did not redirect', async () => {
    vi.stubGlobal('fetch', stub(() => new Response('{}', { status: 200 })));
    const result = await authorizeRedirect(BASE, { client_id: 'c' }, 'tok');
    expect(result.ok).toBe(false);
  });
});
