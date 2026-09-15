"use client";

import { useState } from "react";

/**
 * Minimal Freighter wallet connect. `signAndSubmit` is the piece every
 * other page in this app calls after fetching an unsigned XDR from the
 * backend (`/escrows/build/*`, `/disputes/build/*`) — fill in the actual
 * `@stellar/freighter-api` calls (`isConnected`, `getPublicKey`,
 * `signTransaction`) plus submitting the signed XDR to Soroban RPC.
 */
export function WalletConnect() {
  const [publicKey, setPublicKey] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  async function connect() {
    setError(null);
    try {
      // TODO: replace with real Freighter calls, e.g.
      // const freighter = await import("@stellar/freighter-api");
      // const connected = await freighter.isConnected();
      // if (!connected) throw new Error("Freighter not installed");
      // const { address } = await freighter.getAddress();
      // setPublicKey(address);
      throw new Error("Freighter integration not wired up yet — see WalletConnect.tsx");
    } catch (err) {
      setError(err instanceof Error ? err.message : "failed to connect wallet");
    }
  }

  if (publicKey) {
    return <p>Connected: {publicKey.slice(0, 6)}…{publicKey.slice(-4)}</p>;
  }

  return (
    <div>
      <button onClick={connect}>Connect Freighter</button>
      {error && <p style={{ color: "#b00020" }}>{error}</p>}
    </div>
  );
}

/**
 * Given an unsigned XDR string from the backend, sign it with Freighter and
 * submit it to Soroban RPC. Every "build/*" backend endpoint returns exactly
 * this shape, so this is the one function the rest of the frontend needs.
 */
export async function signAndSubmit(_xdr: string): Promise<{ hash: string }> {
  throw new Error("signAndSubmit not implemented — wire up Freighter signing + RPC submit here");
}
