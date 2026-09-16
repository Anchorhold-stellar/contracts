use soroban_sdk::{contracttype, Address, String, Vec};

#[contracttype]
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum EscrowStatus {
    Created,
    Active,
    Disputed,
    Completed,
    Cancelled,
}

#[contracttype]
#[derive(Clone, PartialEq, Debug)]
pub enum DisputeOutcome {
    Pending,
    RenterWins,
    HostWins,
    /// Jurors never reached a full vote before the deadline; the disputed
    /// amount was split 50/50 instead of leaving it frozen forever.
    Split,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct Milestone {
    pub description: String,
    pub amount: i128,
    /// Seconds after deposit before this milestone auto-releases.
    pub auto_release_offset: u64,
    /// Absolute ledger timestamp; set when the escrow is funded.
    pub auto_release_at: u64,
    /// True once this milestone is no longer pending - either genuinely
    /// released to the host, or written off via mutual_cancel's refund to
    /// the renter (see `released_at` for telling those apart).
    pub released: bool,
    /// Ledger timestamp of an actual release to the host - left at 0 for
    /// mutual_cancel's refund path, since nothing was "released" there in
    /// the payout-to-host sense even though `released` is set to keep it
    /// out of future refund/completion accounting.
    pub released_at: u64,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct Escrow {
    pub id: u32,
    pub renter: Address,
    pub host: Address,
    pub asset: Address,
    pub total_amount: i128,
    pub funded_amount: i128,
    pub milestones: Vec<Milestone>,
    pub status: EscrowStatus,
    pub dispute_id: Option<u32>,
    /// False only when `requires_host_acceptance` was set at creation and
    /// the host hasn't called `accept_escrow` yet. `deposit` checks this
    /// directly rather than adding a new EscrowStatus, so the common
    /// (unrequested) case stays a plain Created -> Active transition.
    pub host_accepted: bool,
    /// Whether this escrow was created with a host-acceptance gate at all.
    /// Kept separate from `host_accepted` so add_milestone/remove_milestone
    /// know whether an acceptance actually needs revoking when terms
    /// change, versus an ungated escrow where `host_accepted` is just
    /// permanently true and never meant anything.
    pub requires_host_acceptance: bool,
    /// Ledger timestamp at creation - used by `expire_unfunded_escrow` to
    /// clean up escrows nobody ever funded.
    pub created_at: u64,
    /// Set once by raise_dispute and never cleared, even after the dispute
    /// resolves - lets completion logic tell "finished with zero disputes
    /// ever" apart from "finished after resolving one," which
    /// `status`/`dispute_id` alone can't distinguish once a dispute is
    /// resolved and the escrow goes back to Active.
    pub ever_disputed: bool,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct JurorStakeInfo {
    pub asset: Address,
    pub amount: i128,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct JurorParams {
    pub min_stake: i128,
    pub jury_size: u32,
    /// Basis points of a minority juror's stake slashed on a resolved
    /// dispute (not a stale one - see `force_resolve_stale_dispute`).
    pub slash_bps: u32,
    /// Minimum on-chain reputation required to register_juror. Defaults to
    /// i32::MIN (no gate) so existing behavior is unaffected until an admin
    /// opts in.
    pub min_reputation: i32,
    /// Basis points of a resolved (non-stale) dispute's amount paid out to
    /// the jurors who voted on it, split evenly regardless of which side
    /// they voted for. Defaults to 0 (disabled).
    pub arbitration_fee_bps: u32,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct FeeConfig {
    /// Protocol fee in basis points (1 = 0.01%), taken out of every
    /// milestone payout - both normal releases and dispute payouts.
    pub bps: u32,
    pub treasury: Address,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct Dispute {
    pub escrow_id: u32,
    pub milestone_index: u32,
    pub opened_by: Address,
    pub evidence_uri: String,
    pub jurors: Vec<Address>,
    pub votes_for_renter: Vec<Address>,
    pub votes_for_host: Vec<Address>,
    pub resolved: bool,
    /// Ledger timestamp this dispute was resolved (normally or via the
    /// stale-dispute fallback), or 0 if still open.
    pub resolved_at: u64,
    pub outcome: DisputeOutcome,
    /// Ledger timestamp after which anyone can call
    /// `force_resolve_stale_dispute` if jurors haven't finished voting.
    pub voting_deadline: u64,
    /// Follow-up evidence from either party, submitted via
    /// `add_dispute_evidence` after the dispute was opened. `evidence_uri`
    /// above is only ever the opener's initial submission - this is where
    /// the other party (or the opener themselves) can add more before
    /// jurors vote.
    pub additional_evidence: Vec<String>,
    /// The following are snapshotted from JurorParams/StakeWeightedVoting
    /// at raise_dispute time and used as-is at resolution, instead of
    /// re-reading whatever the admin has configured *then*. Without this,
    /// an admin changing slash_bps, arbitration_fee_bps, or
    /// stake_weighted_voting between when jurors vote and when
    /// resolve_dispute is called would retroactively change the stakes a
    /// dispute was decided under - for stake_weighted_voting, toggling it
    /// mid-dispute could even flip who wins.
    pub slash_bps: u32,
    pub arbitration_fee_bps: u32,
    pub stake_weighted: bool,
}
