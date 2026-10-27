/**
 * The sealing module itself, driven directly (no HTTP): what a signature binds,
 * and the ways a message that does not match is refused. The server half is
 * `sdks/rust/wasm`'s `responder` build (cratestack's own server envelope), so
 * each refusal here is cratestack's, not a re-implementation's.
 *
 * Each case is a guard shown failing: the tampered, replayed, re-addressed or
 * re-routed message is refused, and the control beside it — the same message
 * untouched — is accepted.
 */
import { describe, expect, it } from "vitest";
import {
  loadResponderModule,
  newTestIdentity,
  type WasmResponder,
} from "./testing/responder.js";
import type { HeaderPairs, WasmSealedCall, WasmSealer } from "./wasm.js";

const AUDIENCE = "https://api.vpay.example";

function setup(audience = AUDIENCE): {
  sealer: WasmSealer;
  responder: WasmResponder;
} {
  const wasm = loadResponderModule();
  const identity = newTestIdentity();
  const sealer = wasm.Sealer.fromSeed(
    identity.merchantSeed,
    identity.serverPublic,
    undefined,
    AUDIENCE,
  );
  const responder = new wasm.Responder(
    identity.serverSeed,
    [identity.merchantPublic],
    audience,
  );
  return { sealer, responder };
}

async function refused(promise: Promise<unknown>): Promise<boolean> {
  try {
    await promise;
    return false;
  } catch {
    return true;
  }
}

function open(
  responder: WasmResponder,
  call: WasmSealedCall,
  method: string,
  path: string,
  query?: string,
  headers: HeaderPairs = call.headers,
  body: Uint8Array = call.body,
) {
  return responder.openRequest(method, path, query, headers, body);
}

describe("what a signature binds", () => {
  it("opens an untouched message (the control)", async () => {
    const { sealer, responder } = setup();
    const call = await sealer.sealCall(
      "POST",
      "/customers",
      "name=Ada",
      undefined,
      "k1",
    );

    const opened = await open(responder, call, "POST", "/customers");

    expect(opened.payload).toBe("name=Ada");
    expect(opened.idempotencyKey).toBe("k1");
  });

  it("refuses the identical bytes a second time: a replay", async () => {
    const { sealer, responder } = setup();
    const call = await sealer.sealCall(
      "POST",
      "/customers",
      "name=Ada",
      undefined,
      "k1",
    );

    await open(responder, call, "POST", "/customers");

    expect(await refused(open(responder, call, "POST", "/customers"))).toBe(
      true,
    );
  });

  it("refuses a message whose bytes were changed", async () => {
    const { sealer, responder } = setup();
    const call = await sealer.sealCall(
      "POST",
      "/customers",
      "name=Ada",
      undefined,
      "k1",
    );
    const tampered = Uint8Array.from(call.body);
    tampered[tampered.length - 1] = (tampered[tampered.length - 1] ?? 0) ^ 1;

    expect(
      await refused(
        open(
          responder,
          call,
          "POST",
          "/customers",
          undefined,
          call.headers,
          tampered,
        ),
      ),
    ).toBe(true);
  });

  it("refuses a message addressed to another audience", async () => {
    const { sealer, responder } = setup("https://other.example");
    const call = await sealer.sealCall(
      "GET",
      "/customers",
      undefined,
      undefined,
      undefined,
    );

    expect(await refused(open(responder, call, "GET", "/customers"))).toBe(
      true,
    );
  });

  it("refuses the same message presented to another route", async () => {
    const { sealer, responder } = setup();
    const call = await sealer.sealCall(
      "GET",
      "/customers/cus_1",
      undefined,
      undefined,
      undefined,
    );

    // A different path parameter, a different template, a different method.
    expect(
      await refused(open(responder, call, "GET", "/customers/cus_2")),
    ).toBe(true);
    expect(
      await refused(open(responder, call, "GET", "/payment_intents/cus_1")),
    ).toBe(true);
    expect(
      await refused(open(responder, call, "DELETE", "/customers/cus_1")),
    ).toBe(true);
    // And the control: the route it was sealed for.
    expect(
      (await open(responder, call, "GET", "/customers/cus_1")).payload,
    ).toBe("");
  });

  it("refuses the same message with another query, however the pairs are ordered", async () => {
    const { sealer, responder } = setup();
    const call = await sealer.sealCall(
      "GET",
      "/payment_intents",
      undefined,
      "limit=2&starting_after=pi_1",
      undefined,
    );

    expect(
      await refused(
        open(
          responder,
          call,
          "GET",
          "/payment_intents",
          "limit=3&starting_after=pi_1",
        ),
      ),
    ).toBe(true);
    // Same pairs, other order: the signature binds the canonical query, so this opens.
    const reordered = await sealer.sealCall(
      "GET",
      "/payment_intents",
      undefined,
      "starting_after=pi_1&limit=2",
      undefined,
    );
    expect(
      (
        await open(
          responder,
          reordered,
          "GET",
          "/payment_intents",
          "limit=2&starting_after=pi_1",
        )
      ).payload,
    ).toBe("");
  });

  it("refuses the same message with another Idempotency-Key", async () => {
    const { sealer, responder } = setup();
    const call = await sealer.sealCall(
      "POST",
      "/customers",
      "name=Ada",
      undefined,
      "k1",
    );
    const swapped = call.headers.map(([name, value]): [string, string] =>
      name === "idempotency-key" ? [name, "k2"] : [name, value],
    );

    expect(
      await refused(
        open(responder, call, "POST", "/customers", undefined, swapped),
      ),
    ).toBe(true);
  });

  it("refuses a message signed by a key the server does not hold", async () => {
    const wasm = loadResponderModule();
    const mine = newTestIdentity();
    const theirs = newTestIdentity();
    const sealer = wasm.Sealer.fromSeed(
      mine.merchantSeed,
      mine.serverPublic,
      undefined,
      AUDIENCE,
    );
    const responder = new wasm.Responder(
      mine.serverSeed,
      [theirs.merchantPublic],
      AUDIENCE,
    );
    const call = await sealer.sealCall(
      "GET",
      "/customers",
      undefined,
      undefined,
      undefined,
    );

    expect(await refused(open(responder, call, "GET", "/customers"))).toBe(
      true,
    );
  });

  it("refuses an operation no table row serves, before anything is signed", async () => {
    const { sealer } = setup();

    await expect(
      sealer.sealCall("GET", "/balance", undefined, undefined, undefined),
    ).rejects.toMatchObject({ kind: "unbound" });
    await expect(
      sealer.sealCall("PUT", "/customers/cus_1", "x=y", undefined, "k"),
    ).rejects.toMatchObject({ kind: "unbound" });
  });
});

describe("what an answer binds", () => {
  it("opens the answer to its own request (the control)", async () => {
    const { sealer, responder } = setup();
    const call = await sealer.sealCall(
      "GET",
      "/customers",
      undefined,
      undefined,
      undefined,
    );
    const request = await open(responder, call, "GET", "/customers");
    const answer = await request.sealResponse(200, '{"object":"list"}');

    expect(await call.openResponse(200, answer.headers, answer.body)).toBe(
      '{"object":"list"}',
    );
  });

  it("refuses the answer to another request", async () => {
    const { sealer, responder } = setup();
    const first = await sealer.sealCall(
      "GET",
      "/customers",
      undefined,
      undefined,
      undefined,
    );
    const second = await sealer.sealCall(
      "GET",
      "/customers",
      undefined,
      undefined,
      undefined,
    );
    const request = await open(responder, first, "GET", "/customers");
    const answer = await request.sealResponse(200, "{}");

    await expect(
      second.openResponse(200, answer.headers, answer.body),
    ).rejects.toMatchObject({ kind: "unverified" });
  });

  it("refuses an answer delivered with another status than it was sealed under", async () => {
    const { sealer, responder } = setup();
    const call = await sealer.sealCall(
      "GET",
      "/customers",
      undefined,
      undefined,
      undefined,
    );
    const request = await open(responder, call, "GET", "/customers");
    const answer = await request.sealResponse(404, '{"error":{}}');

    await expect(
      call.openResponse(200, answer.headers, answer.body),
    ).rejects.toMatchObject({ kind: "unverified" });
  });

  it("refuses an answer that is not a COSE message, whatever its status, as unsigned", async () => {
    const { sealer } = setup();
    const call = await sealer.sealCall(
      "GET",
      "/customers",
      undefined,
      undefined,
      undefined,
    );

    await expect(
      call.openResponse(
        401,
        [["content-type", "application/json"]],
        Buffer.from("{}"),
      ),
    ).rejects.toMatchObject({ kind: "unsigned", status: 401 });
  });

  it("opens an answer only once", async () => {
    const { sealer, responder } = setup();
    const call = await sealer.sealCall(
      "GET",
      "/customers",
      undefined,
      undefined,
      undefined,
    );
    const request = await open(responder, call, "GET", "/customers");
    const answer = await request.sealResponse(200, "{}");

    await call.openResponse(200, answer.headers, answer.body);

    await expect(
      call.openResponse(200, answer.headers, answer.body),
    ).rejects.toMatchObject({ kind: "seal" });
  });
});
