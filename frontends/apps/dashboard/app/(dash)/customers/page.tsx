import { CUSTOMERS_SCREEN } from "../../../src/dash/list-screens";
import { ProcedureListScreen } from "../../../src/components/procedure-list-screen";
import { CUSTOMERS } from "../../../src/dash/resource-name";
import { CustomersList } from "./customers-list";

/**
 * `/customers` — this merchant's customers.
 *
 * # The identifiers are UNMASKED, and that is a decision rather than an
 * oversight
 *
 * `name`, `email` and `phone` are rendered in full. They carry `@sensitive`
 * in `schemas/vpay.cstack`, and **that attribute does not protect them**:
 * `cratestack-macros`' `is_sensitive_field` is read in exactly one place, to
 * redact a value from the *audit log*. It excludes nothing from a response
 * and never has. The maintainer's call (2026-09-13) is that an operator who
 * already sees a payer's details on the payment detail screen is not
 * protected by hiding them here. Anyone reading `@sensitive` and inferring
 * masking is reading it wrong, which is why this paragraph exists.
 *
 * # Erased customers do not appear
 *
 * The procedure's `WHERE` carries `anonymized_at IS NULL`, unconditionally.
 * Erasure (issue #111, closed in #143) already answered for those records
 * and this surface does not re-expose them. Its container test removes that
 * clause to prove the exclusion fires.
 *
 * Note this differs from `GET /v1/customers`, the merchant-facing list,
 * which does **not** exclude anonymised rows — an asymmetry worth knowing
 * about rather than discovering.
 *
 * # No address is shown
 *
 * The six `address_*` columns and the two coordinate columns are not on the
 * summary type at all. A list is not where a payer's postal address or GPS
 * point is read.
 */
export const dynamic = "force-dynamic";

export default async function CustomersPage({
  searchParams,
}: {
  searchParams: Promise<Record<string, string | string[] | undefined>>;
}) {
  return (
    <ProcedureListScreen
      {...CUSTOMERS_SCREEN}
      resource={CUSTOMERS}
      basePath="/customers"
      params={await searchParams}
      List={CustomersList}
      emptyMessage="No customers. This merchant has no customers yet."
    />
  );
}
