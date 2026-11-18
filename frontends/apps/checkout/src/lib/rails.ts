/**
 * Which payment methods this page can drive, and how.
 *
 * **The list of methods on offer is never written here.** It comes from the
 * session read's `rails` member: the server-driven rail spec
 * (`vpay_api::model::RailSpec`, `docs/reference/rails.md`), one entry per
 * code in the intent's `payment_method_types`, in that order. Each entry
 * says what the page has to do — its `flow` (`push`: collect the payer's
 * fields and confirm inline; `redirect`: hand the payer to the rail's own
 * page), the `fields` to collect, and the names to show. An entry may be a
 * rail code (`mtn_momo`, `notchpay`) or a payment-method **type**
 * (`mtn_wallet`, `orange_wallet`; ADR-0029 D12): for a type the server
 * renders the flow of the rail its router ranks first, and confirms by the
 * type, so this page never has to know which rail a deployment runs. That is
 * what lets one hosted checkout serve a deployment whose only rail is an
 * aggregator (Notch Pay) as well as one with the direct MTN and Orange rails.
 *
 * What this module owns is narrower: whether the page **can** collect what an
 * entry asks for. It has two screens: one phone-number form (a Cameroon
 * MSISDN, `normalizeCameroonMsisdn`) and one button for a redirect. An entry
 * it cannot render is not silently dropped and not rendered as a button that
 * fails on click: it is listed as unsupported, with its code shown, and if
 * it is the *only* entry the page refuses outright (D9). A payer who cannot
 * pay must be told so before they try, not after.
 *
 * A server that renders no `rails` member at all (older than the spec,
 * 2026-09-16) falls back to {@link RAIL_PAGE_FLOWS}, this page's own
 * knowledge of the two direct rails, so such a deployment pays as before.
 */
import type { MessageKey, Translate } from "../i18n/index";
import { DICTIONARIES } from "../i18n/index";
import type { PaymentIntent, PublicPaymentIntent, RailSpec } from "./types";

/** What the page must do for a payment method. */
export type RailFlow = "mobile_money_push" | "redirect";

/** A payment method's name in each language the page renders, from the server. */
export interface RailName {
  en?: string | undefined;
  fr?: string | undefined;
}

export interface SupportedRail {
  /** What the confirm names: `payment_method_data[type]`. A rail code or a type. */
  code: string;
  flow: RailFlow;
  /** This page's own message for the method, when it has one. */
  label: MessageKey | null;
  /** The server's name for it, used when the page has no message of its own. */
  name?: RailName | null | undefined;
}

export interface RailChoices {
  /** Methods the intent offers that this page knows how to drive, in the server's order. */
  supported: SupportedRail[];
  /** Codes the intent offers that this page cannot drive (D9). */
  unsupported: string[];
}

/**
 * The fallback for a session read with no `rails` member: rail code → the page
 * flow it needs, and the dictionary key naming it. Adding a rail here without
 * adding its key to both dictionaries does not compile.
 */
export const RAIL_PAGE_FLOWS: Readonly<
  Record<string, { flow: RailFlow; label: MessageKey }>
> = Object.freeze({
  mtn_momo: { flow: "mobile_money_push", label: "rail.mtn_momo" },
  orange_money: { flow: "redirect", label: "rail.orange_money" },
});

/** A message key this page's dictionaries have, or `null`. */
function messageKeyOf(key: string): MessageKey | null {
  return Object.prototype.hasOwnProperty.call(DICTIONARIES.en, key)
    ? (key as MessageKey)
    : null;
}

/**
 * Whether the page can collect what a push entry asks for: exactly one
 * field, `msisdn`, a Cameroon phone number — the one form it has.
 */
function isCameroonMsisdnOnly(fields: RailSpec["fields"]): boolean {
  if (fields.length !== 1) {
    return false;
  }
  const field = fields[0];
  return (
    field !== undefined &&
    field.name === "msisdn" &&
    field.type === "phone" &&
    field.region === "CM"
  );
}

/** One server entry, as something this page can drive, or `null`. */
function supportedFromSpec(spec: RailSpec): SupportedRail | null {
  const name = spec.display_name ?? null;
  const label = messageKeyOf(spec.label_key);
  if (spec.flow === "redirect" && spec.fields.length === 0) {
    return { code: spec.code, flow: "redirect", label, name };
  }
  if (spec.flow === "push" && isCameroonMsisdnOnly(spec.fields)) {
    return { code: spec.code, flow: "mobile_money_push", label, name };
  }
  return null;
}

/**
 * Splits what the intent offers into what this page can and cannot do,
 * narrowed by the deployment's own allow-list.
 *
 * `specs` is the session read's `rails` member, or `null` when the read
 * carried none (an older server; see the module doc). `allowed` is
 * `checkout.allowed_methods` from the operator's `config.yaml`, or `null`
 * for "no opinion". It can only ever **narrow**: a code the operator allows
 * that the intent does not offer adds nothing, and one the intent offers and
 * the operator excludes lands in `unsupported`, so the payer is *told*
 * rather than shown a shorter list with no explanation.
 */
export function railChoices(
  intent: PaymentIntent | PublicPaymentIntent,
  allowed: readonly string[] | null = null,
  specs: readonly RailSpec[] | null = null,
): RailChoices {
  const supported: SupportedRail[] = [];
  const unsupported: string[] = [];
  const excluded = (code: string): boolean =>
    allowed !== null && !allowed.includes(code);
  if (specs !== null) {
    for (const spec of specs) {
      const rail = supportedFromSpec(spec);
      if (rail === null || excluded(spec.code)) {
        unsupported.push(spec.code);
        continue;
      }
      supported.push(rail);
    }
    return { supported, unsupported };
  }
  for (const code of intent.payment_method_types) {
    const entry = RAIL_PAGE_FLOWS[code];
    if (entry === undefined || excluded(code)) {
      unsupported.push(code);
      continue;
    }
    supported.push({ code, flow: entry.flow, label: entry.label });
  }
  return { supported, unsupported };
}

/**
 * What a payer reads for a method: this page's own message when it has one,
 * otherwise the server's name in the page's language (then the other one),
 * otherwise the code itself — the fallback order `docs/reference/rails.md`
 * gives a client.
 */
export function railLabel(rail: SupportedRail, t: Translate): string {
  if (rail.label !== null) {
    return t(rail.label);
  }
  const name = rail.name ?? null;
  const locale = t.locale ?? "fr";
  return (
    (locale === "en" ? name?.en : name?.fr) ?? name?.en ?? name?.fr ?? rail.code
  );
}
