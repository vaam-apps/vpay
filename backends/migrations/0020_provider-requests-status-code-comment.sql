-- 0020: document the `status_code = 0` sentinel on provider_requests.
--
-- `vpay_db::provider_requests::STATUS_CODE_NOT_CARRIED_BY_THE_PORT` records
-- 0 when the rail *did* answer but the provider port carries no HTTP status
-- (`Submitted` and `ProviderError::Rejected` have none). NULL keeps its
-- meaning from 0016 — no answer was received — because `response_is_paired`
-- ties NULL `status_code` to NULL `responded_at`, and the crash-safety
-- recovery table reads that pair as "go and poll". 0 is not an HTTP status,
-- so nobody can mistake it for one. This migration changes no data and no
-- constraint; it only makes the sentinel visible to an operator reading
-- `\d+ provider_requests` (review finding, 2026-09-03).
COMMENT ON COLUMN provider_requests.status_code IS
  'The HTTP status the rail answered with, or 0 when the port carried an answer without a status (Submitted / Rejected). NULL means no answer was received (paired with NULL responded_at by response_is_paired) — the state the recovery table polls.';
