import { Code, InlineBanner, ScreenStack } from "@vaam-apps/ui";

import type { ConfigProblem } from "../config/settings";
import type { ApiFailure } from "../server/api";
import { ReadFailure } from "./read-failure";

/**
 * What `/login` says when it is not a form.
 *
 * Three different reasons, three different components, and the difference is
 * the point: each one is a different person's problem, and the page that
 * conflates them sends somebody to the wrong door.
 *
 * | notice                | whose problem                                            |
 * | --------------------- | -------------------------------------------------------- |
 * | {@link PairedKeyOnly}   | nobody's — this deployment does not serve a password sign-in |
 * | {@link NotConfigured}   | the operator's — vpay serves it, this container is not set up for it |
 * | {@link SignInUnreachable } | vpay's — it could not be asked, and may be back in a minute |
 *
 * None of them renders a field. A page that asks for a password it cannot use
 * invites somebody to retype it.
 */

/** This deployment serves no staff sign-in (`human_login` is not a served method). */
export function PairedKeyOnly() {
  return (
    <ScreenStack>
      <h2 className="font-display text-headline-small text-foreground">
        This console signs in with a paired key
      </h2>
      <div role="status">
        <InlineBanner variant="warning">
          This deployment does not offer a password sign-in, and a paired key
          cannot sign in here until VP-6b. Nothing is wrong with your account.
        </InlineBanner>
      </div>
      <p className="text-body text-muted-foreground">
        An operator can enrol a key from the command line with{" "}
        <Code>vpay-server staff pair-key</Code>, and a client that signs its
        requests with it can read <Code>/dash/v1</Code>. A deployment that
        serves <Code>human_login</Code> gives this page a sign-in.
      </p>
    </ScreenStack>
  );
}

/** vpay serves a sign-in and this container has not been told how to use it. */
export function NotConfigured({
  problems,
}: {
  readonly problems: readonly ConfigProblem[];
}) {
  return (
    <ScreenStack>
      <h2 className="font-display text-headline-small text-foreground">
        No sign-in is configured
      </h2>
      <div role="status">
        <InlineBanner variant="warning">
          This container has no dashboard client registration, so it can sign
          nobody in. Nothing is wrong with your account.
        </InlineBanner>
      </div>
      <p className="text-body text-muted-foreground">
        The operator of this deployment has to set:
      </p>
      <ul className="flex list-disc flex-col gap-1 pl-5 text-body text-foreground">
        {problems.map((problem) => (
          <li key={problem.variable}>
            <Code>{problem.variable}</Code> — {problem.detail}
          </li>
        ))}
      </ul>
    </ScreenStack>
  );
}

/** vpay could not be asked which sign-in it serves. Never "paired key only". */
export function SignInUnreachable({
  failure,
}: {
  readonly failure: ApiFailure;
}) {
  return (
    <ScreenStack>
      <h2 className="font-display text-headline-small text-foreground">
        Sign-in is not reachable
      </h2>
      <p className="text-body text-muted-foreground">
        This dashboard could not ask vpay how to sign you in. Nothing is wrong
        with your account; try again in a moment.
      </p>
      <ReadFailure failure={failure} />
    </ScreenStack>
  );
}
