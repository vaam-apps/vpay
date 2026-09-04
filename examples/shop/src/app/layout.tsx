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
    <html lang="en">
      <body>
        <div className="banner">
          <div className="wrap">
            Demo shop. Nothing here ships, no money moves, and the rails behind
            vpay are stubs. Do not deploy.
          </div>
        </div>
        <header className="site">
          <div className="wrap">
            <strong>
              <Link href="/">Marché</Link>
            </strong>
            <span style={{ color: "var(--muted)" }}>paid with vpay</span>
            <nav>
              <Link href="/">Catalogue</Link>
              <Link href="/cart">Cart</Link>
            </nav>
          </div>
        </header>
        <main className="wrap">{children}</main>
      </body>
    </html>
  );
}
