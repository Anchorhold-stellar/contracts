type Milestone = {
  milestone_index: number;
  description: string;
  amount: string;
  released: boolean;
  auto_release_at: string | null;
};

export function MilestoneTimeline({ milestones }: { milestones: Milestone[] }) {
  return (
    <ol style={{ listStyle: "none", padding: 0 }}>
      {milestones.map((m) => (
        <li
          key={m.milestone_index}
          style={{
            display: "flex",
            justifyContent: "space-between",
            padding: "12px 0",
            borderBottom: "1px solid #eee",
          }}
        >
          <div>
            <strong>{m.description}</strong>
            <div style={{ fontSize: 13, color: "#666" }}>
              {m.released
                ? "Released"
                : m.auto_release_at
                ? `Auto-releases ${new Date(m.auto_release_at).toLocaleString()}`
                : "Pending deposit"}
            </div>
          </div>
          <div>{m.amount}</div>
        </li>
      ))}
    </ol>
  );
}
