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
    pub released: bool,
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
    pub outcome: DisputeOutcome,
    /// Ledger timestamp after which anyone can call
    /// `force_resolve_stale_dispute` if jurors haven't finished voting.
    pub voting_deadline: u64,
}
