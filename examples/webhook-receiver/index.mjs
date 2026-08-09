/**
 * Verifying a vpay webhook.
 *
 * The scheme is Stripe's, so if you already verify Stripe webhooks this is the
 * same code with a different header name.
 *
 * vpay does not send webhooks yet (see ../../docs/STATUS.md), but the
 * verification below is complete and correct — copy it.
 */
import { createHmac, timingSafeEqual } from 'node:crypto';
import { createServer } from 'node:http';

const SECRET = process.env.VPAY_WEBHOOK_SECRET ?? 'whsec_placeholder';
const TOLERANCE_SECONDS = 300;

/** @returns {boolean} */
export function verify(rawBody, signatureHeader, secret, nowSeconds = Date.now() / 1000) {
  const parts = Object.fromEntries(
    signatureHeader.split(',').map((kv) => kv.split('=', 2)),
  );
  const timestamp = Number(parts.t);
  if (!Number.isFinite(timestamp)) return false;

  // Replay protection. Without this, a captured request is valid forever.
  if (Math.abs(nowSeconds - timestamp) > TOLERANCE_SECONDS) return false;

  const expected = createHmac('sha256', secret)
    .update(`${timestamp}.${rawBody}`, 'utf8')
    .digest('hex');

  const a = Buffer.from(expected, 'utf8');
  const b = Buffer.from(parts.v1 ?? '', 'utf8');
  // Length check first: timingSafeEqual throws on a mismatch.
  return a.length === b.length && timingSafeEqual(a, b);
}

const server = createServer((req, res) => {
  let raw = '';
  req.on('data', (c) => (raw += c));
  req.on('end', () => {
    // The RAW body must be used. Parsing and re-stringifying breaks the HMAC.
    if (!verify(raw, req.headers['vpay-signature'] ?? '', SECRET)) {
      res.writeHead(400).end('bad signature');
      return;
    }
    const event = JSON.parse(raw);

    // Delivery is at-least-once: dedupe by event.id before acting.
    console.log('event', event.id, event.type);

    res.writeHead(200).end('ok');
  });
});

if (process.env.NODE_ENV !== 'test') {
  server.listen(4242, () => console.log('listening on :4242'));
}
