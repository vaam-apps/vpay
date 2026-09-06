/**
 * `POST /api/vpay/webhook` — the endpoint vpay's config points at for this
 * merchant.
 *
 * Everything that decides anything lives in `src/server/webhook.ts`, which is
 * where the tests are. This file does three things and no more: read the raw
 * bytes (a parsed-and-reserialised body would break the HMAC), hand them to
 * the handler, and answer with what the handler decided — after the write,
 * never before.
 */
import { db } from "@/server/db";
import { shopConfig } from "@/server/config";
import { ZenStackShopStore } from "@/server/store/zenstack-store";
import { SIGNATURE_HEADER, handleWebhook } from "@/server/webhook";

export const dynamic = "force-dynamic";

export async function POST(request: Request): Promise<Response> {
  const rawBody = await request.text();
  const result = await handleWebhook(
    {
      store: new ZenStackShopStore(db()),
      secret: shopConfig().vpayWebhookSecret,
    },
    {
      rawBody,
      signatureHeader: request.headers.get(SIGNATURE_HEADER),
    },
  );

  // The only log line this endpoint writes, and a demo merchant has no
  // logger to write it to instead. The event id and type are identifiers;
  // the body, the signature header and the secret are not printed, here or
  // anywhere.
  // eslint-disable-next-line no-console
  console.info(
    `vpay webhook: ${result.eventType ?? "<unverified>"} ${result.eventId ?? ""} -> ` +
      `${result.status} ${"outcome" in result.body ? result.body.outcome : result.body.error}`,
  );

  return Response.json(result.body, { status: result.status });
}
