"use client";

import { Button, useTheme } from "@vaam-apps/ui";
import { Moon, Sun } from "lucide-react";
import { useSyncExternalStore } from "react";

const NEVER = () => () => undefined;

/**
 * One-tap light/dark switch for the top app bar.
 *
 * The three-way `ThemeSwitcher` (System, Light, Dark) stays in the account
 * block, where "follow the OS" can be chosen; this is the shortcut, and it
 * writes the same store, so the two can never disagree.
 *
 * It renders the dark-theme icon until it has mounted: the server cannot
 * know the operator's choice (it lives in `localStorage`), and
 * `themeInitScript` has already painted the right theme before React runs,
 * so only this button's icon could mismatch on hydration — and it does not.
 */
export function ThemeToggle() {
  const { resolvedTheme, setPreference } = useTheme();
  const mounted = useSyncExternalStore(
    NEVER,
    () => true,
    () => false,
  );
  const dark = !mounted || resolvedTheme === "dark";
  return (
    <Button
      type="button"
      variant="ghost"
      size="icon"
      aria-label={dark ? "Use the light theme" : "Use the dark theme"}
      onClick={() => setPreference(dark ? "light" : "dark")}
    >
      {dark ? (
        <Sun size={20} aria-hidden="true" />
      ) : (
        <Moon size={20} aria-hidden="true" />
      )}
    </Button>
  );
}
