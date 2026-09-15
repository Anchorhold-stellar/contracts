import { MilestoneTimeline } from "../../../components/MilestoneTimeline";

async function getEscrow(id: string) {
  const res = await fetch(`${process.env.NEXT_PUBLIC_API_URL}/escrows/${id}`, {
    cache: "no-store",
  });
  if (!res.ok) return null;
  return res.json();
}

export default async function EscrowDetail({ params }: { params: { id: string } }) {
  const escrow = await getEscrow(params.id);

  if (!escrow) {
    return <p>Escrow not found.</p>;
  }

  return (
    <div>
      <h1>Escrow #{escrow.escrow_id}</h1>
      <p>
        Status: <strong>{escrow.status}</strong> · Total: {escrow.total_amount}
      </p>
      <MilestoneTimeline milestones={escrow.milestones} />

      {escrow.status !== "disputed" && (
        <a href={`/disputes/new?escrowId=${escrow.escrow_id}`}>
          Something wrong with this stay? Open a dispute.
        </a>
      )}
    </div>
  );
}
