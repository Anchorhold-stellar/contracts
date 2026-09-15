async function getEscrows(wallet: string) {
  const res = await fetch(`${process.env.NEXT_PUBLIC_API_URL}/escrows?wallet=${wallet}`, {
    cache: "no-store",
  });
  if (!res.ok) return [];
  return res.json();
}

export default async function Dashboard() {
  // TODO: read the connected wallet from a client-side context/cookie
  // instead of hardcoding — left simple here since WalletConnect is a stub.
  const wallet = "REPLACE_WITH_CONNECTED_WALLET";
  const escrows = await getEscrows(wallet);

  return (
    <div>
      <h1>Your escrows</h1>
      {escrows.length === 0 && <p>No escrows yet.</p>}
      <ul>
        {escrows.map((e: any) => (
          <li key={e.escrow_id}>
            <a href={`/escrow/${e.escrow_id}`}>
              Escrow #{e.escrow_id} — {e.status} — {e.total_amount}
            </a>
          </li>
        ))}
      </ul>
    </div>
  );
}
