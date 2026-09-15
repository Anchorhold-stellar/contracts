export default function Disputes() {
  // TODO: fetch open disputes assigned to the connected wallet as juror
  // (GET /disputes?juror=<wallet> — add that filter to backend/routes/dispute.js
  // once you're ready; kept out of the scaffold since juror assignment
  // lives on-chain and needs a read path from the indexer first).
  return (
    <div>
      <h1>Disputes</h1>
      <p>
        Disputes you&apos;ve opened, or ones you&apos;ve been assigned to
        vote on as a juror, will show up here.
      </p>
    </div>
  );
}
