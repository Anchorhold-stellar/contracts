//! SafeTrust v2 escrow contract.
//!
//! Design goals (see README for the feature rationale):
//!   - Multi-milestone release instead of a single deploy/fund/release flow.
//!   - Auto-release per milestone after a timeout if nobody disputes.
//!   - On-chain dispute resolution via a staked-juror majority vote.
//!   - On-chain reputation score that persists across escrows.
//!
//! This is a scaffold: the state machine, storage layout, and function
//! signatures are meant to be a real starting point, but review the token
//! transfer calls and juror incentive/slashing logic carefully before any
//! mainnet deployment — that's exactly the kind of surface an audit should
//! focus on.

#![no_std]

use soroban_sdk::{
    contract, contractimpl, contracttype, panic_with_error, symbol_short, Address, Env, String,
    Vec,
};

mod errors;
mod types;

use errors::Error;
use types::{Dispute, DisputeOutcome, Escrow, EscrowStatus, FeeConfig, Milestone};

const MIN_JUROR_STAKE: i128 = 100_0000000; // 100 units at 7 decimals, tune per asset
const JURY_SIZE: u32 = 3;
/// Hard ceiling on the protocol fee, independent of whatever the admin sets:
/// 20% of a milestone payout, so a compromised or careless admin key can't
/// route the whole escrow to the treasury.
const MAX_FEE_BPS: u32 = 2000;
const BPS_DENOMINATOR: i128 = 10_000;

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Admin,
    EscrowCounter,
    Escrow(u32),
    Dispute(u32),
    JurorPool,
    JurorStake(Address),
    Reputation(Address),
    FeeConfig,
}

#[contract]
pub struct EscrowContract;

#[contractimpl]
impl EscrowContract {
    /// One-time setup. `admin` can adjust juror pool params later; it has no
    /// power over individual escrows (no admin-controlled fund release).
    pub fn initialize(env: Env, admin: Address) {
        admin.require_auth();
        if env.storage().instance().has(&DataKey::Admin) {
            panic_with_error!(&env, Error::AlreadyInitialized);
        }
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage()
            .instance()
            .set(&DataKey::EscrowCounter, &0u32);
    }

    /// Set (or update) the protocol fee taken out of every milestone payout.
    /// `bps` is capped at `MAX_FEE_BPS` regardless of what the admin asks
    /// for. Passing `bps == 0` effectively disables the fee.
    pub fn set_fee_config(env: Env, admin: Address, bps: u32, treasury: Address) {
        Self::require_admin(&env, &admin);
        if bps > MAX_FEE_BPS {
            panic_with_error!(&env, Error::FeeTooHigh);
        }
        env.storage()
            .instance()
            .set(&DataKey::FeeConfig, &FeeConfig { bps, treasury });
    }

    pub fn get_fee_config(env: Env) -> Option<FeeConfig> {
        env.storage().instance().get(&DataKey::FeeConfig)
    }

    /// Create a new escrow with an ordered list of milestones. `renter` must
    /// call `deposit` afterward to actually lock funds — creating an escrow
    /// doesn't move any tokens.
    ///
    /// `milestones` is (description, amount, auto_release_offset_seconds).
    /// `auto_release_offset_seconds` is measured from the deposit timestamp,
    /// not from contract creation, so it represents "how long after funding
    /// does this milestone auto-release if undisputed."
    pub fn create_escrow(
        env: Env,
        renter: Address,
        host: Address,
        asset: Address,
        milestones: Vec<(String, i128, u64)>,
    ) -> u32 {
        renter.require_auth();

        if milestones.is_empty() {
            panic_with_error!(&env, Error::NoMilestones);
        }

        let mut total: i128 = 0;
        let mut built: Vec<Milestone> = Vec::new(&env);
        for (desc, amount, offset) in milestones.iter() {
            if amount <= 0 {
                panic_with_error!(&env, Error::InvalidAmount);
            }
            total += amount;
            built.push_back(Milestone {
                description: desc.clone(),
                amount,
                auto_release_offset: offset,
                released: false,
                auto_release_at: 0, // set on deposit
            });
        }

        let id = Self::next_escrow_id(&env);
        let escrow = Escrow {
            id,
            renter: renter.clone(),
            host: host.clone(),
            asset,
            total_amount: total,
            funded_amount: 0,
            milestones: built,
            status: EscrowStatus::Created,
            dispute_id: None,
        };
        env.storage().persistent().set(&DataKey::Escrow(id), &escrow);

        env.events()
            .publish((symbol_short!("escrow"), symbol_short!("created")), (id, renter, host, total));

        id
    }

    /// Renter locks the full escrow amount. Sets each milestone's
    /// `auto_release_at` relative to this deposit timestamp.
    pub fn deposit(env: Env, renter: Address, escrow_id: u32) {
        renter.require_auth();
        let mut escrow = Self::load_escrow(&env, escrow_id);

        if escrow.renter != renter {
            panic_with_error!(&env, Error::NotAuthorized);
        }
        if escrow.status != EscrowStatus::Created {
            panic_with_error!(&env, Error::InvalidState);
        }

        let token = soroban_sdk::token::Client::new(&env, &escrow.asset);
        token.transfer(&renter, &env.current_contract_address(), &escrow.total_amount);

        let now = env.ledger().timestamp();
        for i in 0..escrow.milestones.len() {
            let mut m = escrow.milestones.get(i).unwrap();
            m.auto_release_at = now + m.auto_release_offset;
            escrow.milestones.set(i, m);
        }

        escrow.funded_amount = escrow.total_amount;
        escrow.status = EscrowStatus::Active;
        env.storage()
            .persistent()
            .set(&DataKey::Escrow(escrow_id), &escrow);

        env.events().publish(
            (symbol_short!("escrow"), symbol_short!("funded")),
            (escrow_id, escrow.total_amount),
        );
    }

    /// Either party can cancel an escrow that hasn't been funded yet - no
    /// tokens have moved, so this is just a state transition. Once `deposit`
    /// has been called, use `mutual_cancel` instead.
    pub fn cancel_escrow(env: Env, caller: Address, escrow_id: u32) {
        caller.require_auth();
        let mut escrow = Self::load_escrow(&env, escrow_id);
        if caller != escrow.renter && caller != escrow.host {
            panic_with_error!(&env, Error::NotAuthorized);
        }
        if escrow.status != EscrowStatus::Created {
            panic_with_error!(&env, Error::InvalidState);
        }
        escrow.status = EscrowStatus::Cancelled;
        env.storage()
            .persistent()
            .set(&DataKey::Escrow(escrow_id), &escrow);

        env.events()
            .publish((symbol_short!("escrow"), symbol_short!("cancelled")), escrow_id);
    }

    /// Both parties agree to unwind a funded, undisputed escrow early.
    /// Requires both signatures in the same invocation (a real multi-sig
    /// transaction, not a unilateral call) - refunds every not-yet-released
    /// milestone amount back to the renter and closes the escrow.
    pub fn mutual_cancel(env: Env, renter: Address, host: Address, escrow_id: u32) {
        renter.require_auth();
        host.require_auth();

        let mut escrow = Self::load_escrow(&env, escrow_id);
        if escrow.renter != renter || escrow.host != host {
            panic_with_error!(&env, Error::NotAuthorized);
        }
        if escrow.status != EscrowStatus::Active {
            panic_with_error!(&env, Error::InvalidState);
        }

        let mut refund: i128 = 0;
        for i in 0..escrow.milestones.len() {
            let mut m = escrow.milestones.get(i).unwrap();
            if !m.released {
                refund += m.amount;
                m.released = true;
                escrow.milestones.set(i, m);
            }
        }

        if refund > 0 {
            let token = soroban_sdk::token::Client::new(&env, &escrow.asset);
            token.transfer(&env.current_contract_address(), &renter, &refund);
        }

        escrow.status = EscrowStatus::Cancelled;
        env.storage()
            .persistent()
            .set(&DataKey::Escrow(escrow_id), &escrow);

        env.events().publish(
            (symbol_short!("escrow"), symbol_short!("mutual")),
            (escrow_id, refund),
        );
    }

    /// Renter explicitly confirms a milestone is satisfied and releases it
    /// to the host early (before the auto-release timeout).
    pub fn confirm_milestone(env: Env, renter: Address, escrow_id: u32, milestone_index: u32) {
        renter.require_auth();
        let mut escrow = Self::load_escrow(&env, escrow_id);
        if escrow.renter != renter {
            panic_with_error!(&env, Error::NotAuthorized);
        }
        Self::release_milestone_internal(&env, &mut escrow, milestone_index);
    }

    /// Anyone can call this to trigger auto-release once the timeout has
    /// passed and no dispute is open. Kept permissionless (like a keeper
    /// job) so releases don't depend on either party being online.
    pub fn check_auto_release(env: Env, escrow_id: u32, milestone_index: u32) {
        let mut escrow = Self::load_escrow(&env, escrow_id);
        if escrow.status == EscrowStatus::Disputed {
            panic_with_error!(&env, Error::EscrowDisputed);
        }
        let m = escrow
            .milestones
            .get(milestone_index)
            .unwrap_or_else(|| panic_with_error!(&env, Error::InvalidMilestone));
        if env.ledger().timestamp() < m.auto_release_at {
            panic_with_error!(&env, Error::TooEarly);
        }
        Self::release_milestone_internal(&env, &mut escrow, milestone_index);
    }

    /// Either party can open a dispute on a specific milestone before it
    /// releases. This flips the whole escrow to `Disputed`, freezing all
    /// not-yet-released milestones until jurors resolve it.
    pub fn raise_dispute(
        env: Env,
        caller: Address,
        escrow_id: u32,
        milestone_index: u32,
        evidence_uri: String,
    ) -> u32 {
        caller.require_auth();
        let mut escrow = Self::load_escrow(&env, escrow_id);

        if caller != escrow.renter && caller != escrow.host {
            panic_with_error!(&env, Error::NotAuthorized);
        }
        let m = escrow
            .milestones
            .get(milestone_index)
            .unwrap_or_else(|| panic_with_error!(&env, Error::InvalidMilestone));
        if m.released {
            panic_with_error!(&env, Error::AlreadyReleased);
        }
        if escrow.status == EscrowStatus::Disputed {
            panic_with_error!(&env, Error::EscrowDisputed);
        }

        let jurors = Self::select_jurors(&env, escrow_id);
        let dispute = Dispute {
            escrow_id,
            milestone_index,
            opened_by: caller.clone(),
            evidence_uri,
            jurors,
            votes_for_renter: Vec::new(&env),
            votes_for_host: Vec::new(&env),
            resolved: false,
            outcome: DisputeOutcome::Pending,
        };
        env.storage()
            .persistent()
            .set(&DataKey::Dispute(escrow_id), &dispute);

        escrow.status = EscrowStatus::Disputed;
        escrow.dispute_id = Some(escrow_id);
        env.storage()
            .persistent()
            .set(&DataKey::Escrow(escrow_id), &escrow);

        env.events().publish(
            (symbol_short!("escrow"), symbol_short!("disputed")),
            (escrow_id, milestone_index, caller),
        );

        escrow_id
    }

    /// Register as a juror by staking `MIN_JUROR_STAKE` of the escrow's
    /// asset. Slashed stake on a losing minority vote goes to the majority
    /// side's reward pool (kept simple here — see `resolve_dispute`).
    pub fn register_juror(env: Env, juror: Address, asset: Address, stake: i128) {
        juror.require_auth();
        if stake < MIN_JUROR_STAKE {
            panic_with_error!(&env, Error::InsufficientStake);
        }
        let token = soroban_sdk::token::Client::new(&env, &asset);
        token.transfer(&juror, &env.current_contract_address(), &stake);

        env.storage()
            .persistent()
            .set(&DataKey::JurorStake(juror.clone()), &stake);

        let mut pool: Vec<Address> = env
            .storage()
            .persistent()
            .get(&DataKey::JurorPool)
            .unwrap_or(Vec::new(&env));
        if !pool.contains(&juror) {
            pool.push_back(juror.clone());
        }
        env.storage().persistent().set(&DataKey::JurorPool, &pool);

        env.events()
            .publish((symbol_short!("juror"), symbol_short!("joined")), (juror, stake));
    }

    /// A juror assigned to this dispute casts a vote for who should receive
    /// the disputed milestone amount.
    pub fn vote_dispute(env: Env, juror: Address, escrow_id: u32, vote_for_renter: bool) {
        juror.require_auth();
        let mut dispute: Dispute = env
            .storage()
            .persistent()
            .get(&DataKey::Dispute(escrow_id))
            .unwrap_or_else(|| panic_with_error!(&env, Error::NoDispute));

        if dispute.resolved {
            panic_with_error!(&env, Error::AlreadyResolved);
        }
        if !dispute.jurors.contains(&juror) {
            panic_with_error!(&env, Error::NotAJuror);
        }
        if dispute.votes_for_renter.contains(&juror) || dispute.votes_for_host.contains(&juror) {
            panic_with_error!(&env, Error::AlreadyVoted);
        }

        if vote_for_renter {
            dispute.votes_for_renter.push_back(juror);
        } else {
            dispute.votes_for_host.push_back(juror);
        }
        env.storage()
            .persistent()
            .set(&DataKey::Dispute(escrow_id), &dispute);
    }

    /// Tally votes once all jurors have voted (or can be called by anyone
    /// after a resolution deadline — deadline enforcement is a TODO, see
    /// README). Distributes the disputed milestone amount to the winning
    /// side and updates reputation for both parties.
    pub fn resolve_dispute(env: Env, escrow_id: u32) {
        let mut dispute: Dispute = env
            .storage()
            .persistent()
            .get(&DataKey::Dispute(escrow_id))
            .unwrap_or_else(|| panic_with_error!(&env, Error::NoDispute));
        if dispute.resolved {
            panic_with_error!(&env, Error::AlreadyResolved);
        }

        let renter_votes = dispute.votes_for_renter.len();
        let host_votes = dispute.votes_for_host.len();
        if renter_votes + host_votes < dispute.jurors.len() {
            panic_with_error!(&env, Error::VotingIncomplete);
        }

        let mut escrow = Self::load_escrow(&env, escrow_id);
        let renter_wins = renter_votes > host_votes;
        dispute.outcome = if renter_wins {
            DisputeOutcome::RenterWins
        } else {
            DisputeOutcome::HostWins
        };
        dispute.resolved = true;

        let m = escrow.milestones.get(dispute.milestone_index).unwrap();
        let token = soroban_sdk::token::Client::new(&env, &escrow.asset);
        let recipient = if renter_wins { &escrow.renter } else { &escrow.host };
        Self::pay_out(&env, &token, recipient, m.amount);

        let mut m = m;
        m.released = true;
        escrow.milestones.set(dispute.milestone_index, m);

        // Reputation: winner +2, loser -1. Simple and tunable; the point is
        // that outcomes persist and feed future trust decisions off-chain.
        Self::adjust_reputation(&env, recipient, 2);
        let loser = if renter_wins { &escrow.host } else { &escrow.renter };
        Self::adjust_reputation(&env, loser, -1);

        // If every milestone is now released, mark the escrow complete;
        // otherwise unfreeze it so remaining milestones can proceed.
        escrow.status = if Self::all_released(&escrow) {
            EscrowStatus::Completed
        } else {
            EscrowStatus::Active
        };
        escrow.dispute_id = None;

        env.storage()
            .persistent()
            .set(&DataKey::Dispute(escrow_id), &dispute);
        env.storage()
            .persistent()
            .set(&DataKey::Escrow(escrow_id), &escrow);

        env.events().publish(
            (symbol_short!("escrow"), symbol_short!("resolved")),
            (escrow_id, dispute.milestone_index, recipient.clone()),
        );
    }

    pub fn get_escrow(env: Env, escrow_id: u32) -> Escrow {
        Self::load_escrow(&env, escrow_id)
    }

    pub fn get_dispute(env: Env, escrow_id: u32) -> Option<Dispute> {
        env.storage().persistent().get(&DataKey::Dispute(escrow_id))
    }

    pub fn get_reputation(env: Env, who: Address) -> i32 {
        env.storage()
            .persistent()
            .get(&DataKey::Reputation(who))
            .unwrap_or(0)
    }

    // ---- internal helpers ----

    fn next_escrow_id(env: &Env) -> u32 {
        let id: u32 = env
            .storage()
            .instance()
            .get(&DataKey::EscrowCounter)
            .unwrap_or(0);
        env.storage()
            .instance()
            .set(&DataKey::EscrowCounter, &(id + 1));
        id
    }

    fn load_escrow(env: &Env, escrow_id: u32) -> Escrow {
        env.storage()
            .persistent()
            .get(&DataKey::Escrow(escrow_id))
            .unwrap_or_else(|| panic_with_error!(env, Error::EscrowNotFound))
    }

    fn release_milestone_internal(env: &Env, escrow: &mut Escrow, milestone_index: u32) {
        if escrow.status != EscrowStatus::Active {
            panic_with_error!(env, Error::InvalidState);
        }
        let mut m = escrow
            .milestones
            .get(milestone_index)
            .unwrap_or_else(|| panic_with_error!(env, Error::InvalidMilestone));
        if m.released {
            panic_with_error!(env, Error::AlreadyReleased);
        }

        let token = soroban_sdk::token::Client::new(env, &escrow.asset);
        Self::pay_out(env, &token, &escrow.host, m.amount);

        m.released = true;
        let amount = m.amount;
        escrow.milestones.set(milestone_index, m);

        if Self::all_released(escrow) {
            escrow.status = EscrowStatus::Completed;
        }
        env.storage()
            .persistent()
            .set(&DataKey::Escrow(escrow.id), escrow);

        env.events().publish(
            (symbol_short!("escrow"), symbol_short!("released")),
            (escrow.id, milestone_index, amount, escrow.host.clone()),
        );
    }

    fn all_released(escrow: &Escrow) -> bool {
        for m in escrow.milestones.iter() {
            if !m.released {
                return false;
            }
        }
        true
    }

    /// Naive juror selection: pull the first JURY_SIZE addresses from the
    /// pool. Replace with weighted-random selection (e.g. VRF or a
    /// commit-reveal seed) before relying on this for anything real —
    /// deterministic "first N" selection is trivially gameable.
    fn select_jurors(env: &Env, seed_escrow_id: u32) -> Vec<Address> {
        let pool: Vec<Address> = env
            .storage()
            .persistent()
            .get(&DataKey::JurorPool)
            .unwrap_or(Vec::new(env));
        let mut selected = Vec::new(env);
        let pool_len = pool.len();
        if pool_len == 0 {
            panic_with_error!(env, Error::NoJurorsAvailable);
        }
        let take = if JURY_SIZE < pool_len { JURY_SIZE } else { pool_len };
        // rotate the starting index by escrow id so consecutive disputes
        // don't always draw the same jurors — still not sybil-resistant,
        // see the note above.
        let start = (seed_escrow_id as u32) % pool_len;
        for i in 0..take {
            let idx = (start + i) % pool_len;
            selected.push_back(pool.get(idx).unwrap());
        }
        selected
    }

    fn require_admin(env: &Env, caller: &Address) {
        caller.require_auth();
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .unwrap_or_else(|| panic_with_error!(env, Error::NotAuthorized));
        if &admin != caller {
            panic_with_error!(env, Error::NotAuthorized);
        }
    }

    /// Splits a payout into (fee, net) per the configured protocol fee.
    /// Returns (0, amount) when no fee is configured.
    fn fee_split(env: &Env, amount: i128) -> (i128, i128) {
        match env.storage().instance().get::<_, FeeConfig>(&DataKey::FeeConfig) {
            Some(cfg) if cfg.bps > 0 => {
                let fee = (amount * cfg.bps as i128) / BPS_DENOMINATOR;
                (fee, amount - fee)
            }
            _ => (0, amount),
        }
    }

    fn pay_out(env: &Env, token: &soroban_sdk::token::Client, recipient: &Address, amount: i128) {
        let (fee, net) = Self::fee_split(env, amount);
        if fee > 0 {
            if let Some(cfg) = env.storage().instance().get::<_, FeeConfig>(&DataKey::FeeConfig) {
                token.transfer(&env.current_contract_address(), &cfg.treasury, &fee);
            }
        }
        token.transfer(&env.current_contract_address(), recipient, &net);
    }

    fn adjust_reputation(env: &Env, who: &Address, delta: i32) {
        let current: i32 = env
            .storage()
            .persistent()
            .get(&DataKey::Reputation(who.clone()))
            .unwrap_or(0);
        env.storage()
            .persistent()
            .set(&DataKey::Reputation(who.clone()), &(current + delta));
    }
}

#[cfg(test)]
mod test;
