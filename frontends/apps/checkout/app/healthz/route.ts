/**
 * `GET /healthz` — "this process is serving", and deliberately nothing more.
 *
 * Added by Step 9's lane 4 (build/image/deploy), not by the lane that wrote
 * this app: it exists for the compose healthcheck and the chart's probes, and
 * it is the one thing an operator can poll that is not a payment page.
 *
 * **It takes no dependency and reports on none.** This app's only upstream is
 * `VPAY_API_URL`, read per request by `middleware.ts` for the embedded page's
 * `frame-ancestors` lookup — and that lookup already fails *closed*
 * (`frame-ancestors 'none'`) rather than erroring. Probing vpay from here
 * would therefore trade a page that degrades safely for a pod the kubelet
 * restarts, and a rolling vpay-server deploy would take the checkout app down
 * with it. Readiness on this app means "Next is answering"; whether vpay is
 * reachable is vpay's own probe's answer.
 *
 * `force-dynamic` because the answer must come from the running process. A
 * statically rendered 200 would be served by the filesystem and would keep
 * answering after the server had stopped doing anything else useful.
 */
export const dynamic = 'force-dynamic';

export function GET(): Response {
  // `no-store` and the other security headers are added by `middleware.ts`,
  // whose matcher is every path; this body is all that is needed here.
  return new Response('ok\n', {
    status: 200,
    headers: { 'content-type': 'text/plain; charset=utf-8' },
  });
}
