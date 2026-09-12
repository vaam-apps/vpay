import { ScreenStack } from "@vaam-apps/ui";

/**
 * The signed-out shell: centred, no rail.
 *
 * The root layout used to wrap every route in this container and render the
 * nav inside it, which put a link to `/payments` on the sign-in form — a
 * page nobody at that point could open. Signed-out routes get their own
 * wrapper instead, and the signed-in rail belongs to `app/(dash)`.
 */
export default function LoginLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <main>
      <ScreenStack className="mx-auto w-full max-w-2xl p-6">
        {children}
      </ScreenStack>
    </main>
  );
}
