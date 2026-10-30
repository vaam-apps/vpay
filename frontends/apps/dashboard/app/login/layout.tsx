import { CenteredShell } from "../../src/components/centered-shell";

/**
 * The signed-out shell: centred, no rail, no nav.
 *
 * The root layout renders none of either, so the sign-in carries no link to a
 * page nobody at that point could open. See {@link CenteredShell}.
 */
export default function LoginLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return <CenteredShell>{children}</CenteredShell>;
}
