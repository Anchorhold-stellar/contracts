import type { ReactNode } from "react";

export const metadata = {
  title: "SafeTrust v2",
  description: "Self-custodied milestone escrow for rentals, on Soroban.",
};

export default function RootLayout({ children }: { children: ReactNode }) {
  return (
    <html lang="en">
      <body style={{ fontFamily: "system-ui, sans-serif", margin: 0 }}>
        <nav
          style={{
            display: "flex",
            gap: 24,
            padding: "16px 24px",
            borderBottom: "1px solid #e5e5e5",
          }}
        >
          <strong>SafeTrust v2</strong>
          <a href="/dashboard">Dashboard</a>
          <a href="/disputes">Disputes</a>
        </nav>
        <main style={{ padding: 24, maxWidth: 960, margin: "0 auto" }}>{children}</main>
      </body>
    </html>
  );
}
