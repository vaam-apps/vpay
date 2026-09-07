import type { Metadata } from "next";
import Link from "next/link";
import "./globals.css";

export const metadata: Metadata = {
  title: "Marché — a vpay demo shop",
  description:
    "A fixed five-product shop that pays through vpay's hosted and embedded checkout.",
};

export default function RootLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <html lang="en" data-theme="bumblebee">
      <body className="min-h-screen bg-base-200">
        <div className="alert alert-warning justify-center rounded-none text-sm">
          Demo shop. Nothing here ships, no money moves, and the rails behind
          vpay are stubs. Do not deploy.
        </div>
        <header className="navbar border-b border-base-300 bg-base-100">
          <div className="mx-auto flex w-full max-w-5xl items-baseline gap-5 px-4">
            <Link href="/" className="text-lg font-semibold">
              Marché
            </Link>
            <span className="text-sm text-base-content/60">paid with vpay</span>
            <nav className="ml-auto flex gap-4">
              <Link href="/" className="link link-hover">
                Catalogue
              </Link>
              <Link href="/cart" className="link link-hover">
                Cart
              </Link>
            </nav>
          </div>
        </header>
        <main className="mx-auto max-w-5xl px-4 py-6 pb-16">{children}</main>
      </body>
    </html>
  );
}
