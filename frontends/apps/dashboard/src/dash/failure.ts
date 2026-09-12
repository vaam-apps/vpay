/**
 * A Refine error, read back as this app's own `ApiFailure`.
 *
 * `ReadFailure` renders a status and a request id, and it has done since
 * before Refine existed here. Rather than change that component to take a
 * second error shape, this maps at the boundary — one direction, one place.
 */
import type { ApiFailure } from "../server/api";

export function failureFromError(error: unknown): ApiFailure {
  const status =
    typeof error === "object" &&
    error !== null &&
    typeof (error as { statusCode?: unknown }).statusCode === "number"
      ? (error as { statusCode: number }).statusCode
      : 0;
  const message =
    error instanceof Error ? error.message : "The read did not complete.";
  return { status, message, requestId: null };
}
