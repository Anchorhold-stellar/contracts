import { WalletConnect } from "../components/WalletConnect";

export default function Home() {
  return (
    <div>
      <h1>Rental escrow that doesn&apos;t need a third party to hold the keys</h1>
      <p>
        Deposits are locked in a contract you can read yourself, released in
        milestones as a stay progresses, and backed by an on-chain dispute
        process if something goes wrong.
      </p>
      <WalletConnect />
    </div>
  );
}
