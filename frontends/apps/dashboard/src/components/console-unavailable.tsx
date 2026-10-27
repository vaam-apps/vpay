import { Card, Code, InlineBanner, ScreenStack } from "@vaam-apps/ui";

import { Brand } from "./brand";

/**
 * The one screen this app renders until VP-6b (ADR-0036).
 *
 * The console signs every `/dash/v1` request with a P-256 key a person's
 * browser generated and vaam paired through `/svc/v1`. The sealed sign-in and
 * the signed reader that use that key are VP-6b; the password, TOTP, SSO and
 * session routes this app used to call are deleted from vpay. So there is
 * nothing this app can ask vpay for yet, and it says so rather than offering a
 * form that cannot work or a page that cannot load: fail closed.
 *
 * Rendered by `/` and by every path that has no page, which is every path.
 */
export function ConsoleUnavailable() {
  return (
    <main className="grid min-h-dvh place-items-center p-4">
      <div className="flex w-full max-w-lg flex-col gap-6 py-12">
        <header className="flex justify-center">
          <h1 className="sr-only">vpay dashboard</h1>
          <Brand />
        </header>
        <Card>
          <div className="flex flex-col gap-6 p-6 sm:p-8">
            <ScreenStack>
              <h2 className="font-display text-headline-small text-foreground">
                This console signs in with a paired key
              </h2>
              <div role="status">
                <InlineBanner variant="warning">
                  Signing in with a paired key is not available until VP-6b.
                  Nothing is wrong with your account.
                </InlineBanner>
              </div>
              <p className="text-body text-muted-foreground">
                There is no password, code or single sign-on here any more. An
                operator can enrol a key from the command line with{" "}
                <Code>vpay-server staff pair-key</Code>, and a client that signs
                its requests with it can read <Code>/dash/v1</Code>.
              </p>
            </ScreenStack>
          </div>
        </Card>
      </div>
    </main>
  );
}
