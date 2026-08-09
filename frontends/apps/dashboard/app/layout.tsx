import type { Metadata } from 'next';

import './globals.css';

export const metadata: Metadata = {
  title: 'vpay dashboard',
  description: 'Observability for vpay. Administration is YAML.',
};

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en" data-theme="corporate">
      <body className="min-h-screen bg-base-200">{children}</body>
    </html>
  );
}
