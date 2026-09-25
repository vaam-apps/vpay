import { InstrumentPanel, ScreenStack, Skeleton } from "@vaam-apps/ui";
import type { ReactNode } from "react";

import { formatAmount } from "../format";
import { PaymentStatusPill } from "../payment-status";
import { AT_A_GLANCE_CAPTION } from "./payment-detail";

/**
 * A four-digit XAF amount, printed the way the panel prints one. The two
 * amount placeholders are exactly its width, because on a phone that
 * width decides how the panel's three figures wrap.
 */
const SAMPLE_AMOUNT = formatAmount(5000, "xaf");

/**
 * One payment's page while its read is in flight, shaped so that the
 * header, the "At a glance" panel and the Summary heading land where their
 * placeholders were.
 *
 * **Why not `RouteSkeleton rows={10}` (2026-09-24, on vaam-apps/vpay#258's
 * review).** It drew a filter bar this page does not have and
 * ten flat rows where this page has an instrument panel, so everything
 * under the header moved when the payment arrived. Measured in the
 * Storybook suite's Chromium, drawn above the loaded header and view: its
 * header matched the page's 48px, but its 36px filter bar stood where the
 * 172px panel lands (80px of bar, wrapped, where a 292px panel lands in a
 * 375×812 story), so its rows began 136px above the Summary section they
 * stood in for at 1280×800 and 212px above it at 375×812 — 142 and 215px
 * at a 20px root, where on the phone it also pushed the page 25px
 * sideways. With this component every box it claims, below, measured
 * the same as the loaded page's to the pixel: at 1280 and 375 and in
 * columns of 956, 496, 327 and 272px, at both roots, with nothing
 * scrolling sideways.
 *
 * What each placeholder copies, and from where:
 *
 * - **The header**, `PaymentDetailHeader`'s `<h2>` and back link: two
 *   lines of body text, `h-lh` each, with a shorter bar inside so the two
 *   read as two.
 * - **The panel** is a real `InstrumentPanel`, so its padding and its
 *   mesh are the library's, not a copy. Its title and caption are drawn
 *   by `InstrumentPanel` itself as an `<h3>` and a `<p>`, which a
 *   placeholder cannot be handed (a bar inside a `<p>` is invalid markup,
 *   and an empty `<h3>` is an axe violation), so they are rebuilt here:
 *   the header block's `mb-4 flex flex-col gap-1`, one `text-title-sm`
 *   line, and the caption itself, invisible, in its own `text-caption` —
 *   one line or two exactly when the caption is.
 * - **The three figures** are a caption line, a value and a caption line
 *   each, like the loaded ones. The amounts are `SAMPLE_AMOUNT`, invisible,
 *   in the loaded value's own face (`font-mono text-metric font-semibold`);
 *   the status is an invisible `PaymentStatusPill`, the pill the loaded
 *   figure draws. Their widths decide where the row wraps on a phone, so
 *   the placeholders line up for a payment of this shape — a four-digit
 *   amount, a one-word status, a charge — and a longer amount can wrap one
 *   figure more than they did.
 * - **The Summary heading**, one line of body text. The eight rows under
 *   it are `DetailRow`'s divided shape, but their values are data and
 *   their heights follow it (the id's `Code` is a pixel taller than a
 *   line), so nothing below the heading is claimed to line up.
 *
 * No `<h2>` or `<h3>` is rendered, and nothing carries
 * `data-testid="detail-id"`: `dashboard.cy.ts` waits on that as the sign
 * the payment has loaded. Everything but the "Loading…" text is
 * `aria-hidden`, through `Skeleton`, and the invisible copies are
 * `visibility: hidden` as well.
 *
 * Held by the `PaymentLoading` and `PaymentLoadingPhone` stories, which
 * compare it box by box with the loaded header and view at 1280 and 375px,
 * squeezed to the shell's columns, and at a 20px root. Each of these fails
 * them on its own: the caption's placeholder a fixed `h-lh w-80` (18px, on
 * the phone, where the caption wraps); the title block without `mb-4`
 * (16px); the header one line instead of two (24px); the status
 * placeholder a fixed `h-4 w-16` instead of the invisible pill (14px); an
 * amount's a fixed `w-32` instead of the invisible sample (22px); the
 * panel's padding taken away (40px); and a footnote bar a fixed `w-20`
 * (2px — the regression `Figure`'s doc records). `screens.test.tsx` fails
 * if the page's loading branch renders `RouteSkeleton` again.
 */
export function PaymentSkeleton() {
  return (
    <ScreenStack role="status" aria-busy="true">
      <span className="sr-only">Loading…</span>
      <div>
        <div className="flex h-lh items-center">
          <Skeleton className="h-4 w-20" />
        </div>
        <div className="flex h-lh items-center">
          <Skeleton className="h-4 w-36" />
        </div>
      </div>
      <ScreenStack>
        <InstrumentPanel>
          <div className="mb-4 flex flex-col gap-1">
            <div className="flex h-lh items-center text-title-sm">
              <Skeleton instrument className="h-4 w-28" />
            </div>
            <Skeleton instrument className="line-clamp-2 w-fit text-caption">
              <span className="invisible">{AT_A_GLANCE_CAPTION}</span>
            </Skeleton>
          </div>
          <div className="flex flex-wrap gap-8">
            <Figure>
              <Amount />
            </Figure>
            <Figure>
              <div className="flex h-lh items-center">
                <Skeleton instrument className="flex w-fit *:invisible">
                  <PaymentStatusPill state="succeeded" />
                </Skeleton>
              </div>
            </Figure>
            <Figure>
              <Amount />
            </Figure>
          </div>
        </InstrumentPanel>
        <div>
          <div className="flex h-lh items-center">
            <Skeleton className="h-4 w-24" />
          </div>
          <div className="flex flex-col divide-y divide-edge-subtle">
            {Array.from({ length: 8 }, (_, row) => (
              <div key={row} className="flex flex-col gap-0.5 py-2">
                <div className="flex h-lh items-center text-caption">
                  <Skeleton className="h-3 w-24" />
                </div>
                <div className="flex h-lh items-center text-body">
                  <Skeleton className="h-4 w-48" />
                </div>
              </div>
            ))}
          </div>
        </div>
      </ScreenStack>
    </ScreenStack>
  );
}

/**
 * A figure's caption line, its value, and its footnote line.
 *
 * The two caption bars are `w-full` under a cap rather than a width of
 * their own, so they never make a figure wider than its value: the
 * loaded status figure is as wide as its pill (78px), and a `w-20` bar
 * made the placeholder 80px and moved the third figure 2px.
 */
function Figure({ children }: { readonly children: ReactNode }) {
  return (
    <div>
      <div className="flex h-lh items-center text-caption">
        <Skeleton instrument className="h-3 w-full max-w-12" />
      </div>
      {children}
      <div className="flex h-lh items-center text-caption">
        <Skeleton instrument className="h-3 w-full max-w-16" />
      </div>
    </div>
  );
}

/** An amount's value line, as wide as `SAMPLE_AMOUNT` in the figure's face. */
function Amount() {
  return (
    <Skeleton instrument className="w-fit font-mono text-metric font-semibold">
      <span className="invisible">{SAMPLE_AMOUNT}</span>
    </Skeleton>
  );
}
