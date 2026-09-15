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
use types::{
    Dispute, DisputeOutcome, Escrow, EscrowStatus, FeeConfig, JurorParams, JurorStakeInfo,
    Milestone,
};

const DEFAULT_MIN_JUROR_STAKE: i128 = 100_0000000; // 100 units at 7 decimals, tune per asset
const DEFAULT_JURY_SIZE: u32 = 3;
/// Default (admin-configurable, see set_dispute_voting_window) length of
/// time jurors have to finish voting before anyone can force-resolve the
/// dispute with a 50/50 split. Keeps a milestone from being frozen forever
/// if jurors go silent.
const DEFAULT_DISPUTE_VOTING_WINDOW: u64 = 3 * 24 * 60 * 60;
const MIN_DISPUTE_VOTING_WINDOW: u64 = 60 * 60; // 1 hour
const MAX_DISPUTE_VOTING_WINDOW: u64 = 30 * 24 * 60 * 60; // 30 days
/// Fraction of a minority juror's stake slashed on a resolved (non-stale)
/// dispute. Redistributed to majority jurors staked in the same asset.
const DEFAULT_JUROR_SLASH_BPS: u32 = 1000; // 10%
/// Hard ceiling on the protocol fee, independent of whatever the admin sets:
/// 20% of a milestone payout, so a compromised or careless admin key can't
/// route the whole escrow to the treasury.
const MAX_FEE_BPS: u32 = 2000;
/// Hard ceiling on the juror slash rate, same rationale as MAX_FEE_BPS.
const MAX_SLASH_BPS: u32 = 5000; // 50%
/// Hard ceiling on the juror arbitration fee - it comes out of the
/// disputed amount before either party sees any of it, so it needs its
/// own (tighter) cap independent of the protocol fee.
const MAX_ARBITRATION_FEE_BPS: u32 = 1000; // 10%
/// Per-call cap on extend_milestone_deadline: 1 year.
const MAX_DEADLINE_EXTENSION_SECONDS: u64 = 365 * 24 * 60 * 60;
/// Upper bound on milestone descriptions and dispute evidence URIs. These
/// land in persistent storage, so an unbounded string is an unbounded and
/// permanent storage-cost griefing vector, not just a UX nuisance.
const MAX_STRING_LENGTH: u32 = 512;
/// Same storage/gas-cost rationale as MAX_STRING_LENGTH, applied to the
/// milestone list's length instead of a string's.
const MAX_MILESTONES: u32 = 50;
/// Same storage-cost rationale as MAX_MILESTONES/MAX_STRING_LENGTH, applied
/// to how many follow-up evidence entries a single dispute can accumulate.
const MAX_ADDITIONAL_EVIDENCE: u32 = 10;
/// How long an escrow can sit in `Created` (never funded, never cancelled)
/// before anyone can permissionlessly expire it - same "someone has to be
/// able to clean this up" rationale as check_auto_release and
/// force_resolve_stale_dispute.
const UNFUNDED_EXPIRY_WINDOW: u64 = 30 * 24 * 60 * 60;
const BPS_DENOMINATOR: i128 = 10_000;

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Admin,
    EscrowCounter,
    Escrow(u32),
    Dispute(u32, u32),
    JurorPool,
    JurorStake(Address),
    Reputation(Address),
    FeeConfig,
    JurorParams,
    ActiveDisputeCount(Address),
    Paused,
    PartyEscrows(Address),
    PendingAdmin,
    DisputeVotingWindow,
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

    /// Rotate the admin key. The admin has no power over individual
    /// escrows (see `initialize`'s doc comment) - this only matters for
    /// fee config, juror params, and pause/unpause - but a compromised or
    /// lost admin key should still be replaceable without redeploying.
    ///
    /// This only proposes the handoff - `new_admin` must independently call
    /// `accept_admin_transfer` to finalize it. A one-step transfer means a
    /// typo'd address permanently bricks every admin-gated function with no
    /// recovery path; requiring the new admin's own signature to accept
    /// closes that off.
    pub fn transfer_admin(env: Env, current_admin: Address, new_admin: Address) {
        Self::require_admin(&env, &current_admin);
        env.storage()
            .instance()
            .set(&DataKey::PendingAdmin, &new_admin);
    }

    /// Finalizes a transfer proposed by `transfer_admin`. Must be called by
    /// the proposed address itself.
    pub fn accept_admin_transfer(env: Env, new_admin: Address) {
        new_admin.require_auth();
        let pending: Address = env
            .storage()
            .instance()
            .get(&DataKey::PendingAdmin)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotAuthorized));
        if pending != new_admin {
            panic_with_error!(&env, Error::NotAuthorized);
        }
        env.storage().instance().set(&DataKey::Admin, &new_admin);
        env.storage().instance().remove(&DataKey::PendingAdmin);
    }

    pub fn get_admin(env: Env) -> Address {
        env.storage()
            .instance()
            .get(&DataKey::Admin)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotAuthorized))
    }

    pub fn get_pending_admin(env: Env) -> Option<Address> {
        env.storage().instance().get(&DataKey::PendingAdmin)
    }

    /// How long jurors get to vote before a dispute can be force-resolved.
    /// Bounded to [MIN_DISPUTE_VOTING_WINDOW, MAX_DISPUTE_VOTING_WINDOW] so
    /// it can't be set to something that either resolves disputes before
    /// jurors can plausibly react or never resolves them at all.
    pub fn set_dispute_voting_window(env: Env, admin: Address, seconds: u64) {
        Self::require_admin(&env, &admin);
        if !(MIN_DISPUTE_VOTING_WINDOW..=MAX_DISPUTE_VOTING_WINDOW).contains(&seconds) {
            panic_with_error!(&env, Error::InvalidVotingWindow);
        }
        env.storage()
            .instance()
            .set(&DataKey::DisputeVotingWindow, &seconds);
    }

    pub fn get_dispute_voting_window(env: Env) -> u64 {
        Self::dispute_voting_window(&env)
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

    /// Update the juror pool requirements. `jury_size` must be odd (so
    /// `resolve_dispute`'s majority check can't tie) and at least 1.
    pub fn set_juror_params(
        env: Env,
        admin: Address,
        min_stake: i128,
        jury_size: u32,
        slash_bps: u32,
        min_reputation: i32,
        arbitration_fee_bps: u32,
    ) {
        Self::require_admin(&env, &admin);
        if min_stake <= 0 {
            panic_with_error!(&env, Error::InvalidAmount);
        }
        if jury_size == 0 || jury_size.is_multiple_of(2) {
            panic_with_error!(&env, Error::InvalidJurySize);
        }
        if slash_bps > MAX_SLASH_BPS {
            panic_with_error!(&env, Error::SlashTooHigh);
        }
        if arbitration_fee_bps > MAX_ARBITRATION_FEE_BPS {
            panic_with_error!(&env, Error::ArbitrationFeeTooHigh);
        }
        env.storage().instance().set(
            &DataKey::JurorParams,
            &JurorParams {
                min_stake,
                jury_size,
                slash_bps,
                min_reputation,
                arbitration_fee_bps,
            },
        );
    }

    pub fn get_juror_params(env: Env) -> JurorParams {
        Self::juror_params(&env)
    }

    /// Emergency circuit breaker: stops new escrows from being created or
    /// funded. Deliberately does *not* touch confirm_milestone,
    /// check_auto_release, disputes, or juror actions on escrows that are
    /// already active - pausing is meant to stop new exposure during an
    /// incident, not strand funds that are already locked up.
    pub fn set_paused(env: Env, admin: Address, paused: bool) {
        Self::require_admin(&env, &admin);
        env.storage().instance().set(&DataKey::Paused, &paused);
    }

    pub fn is_paused(env: Env) -> bool {
        Self::paused(&env)
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
        requires_host_acceptance: bool,
    ) -> u32 {
        renter.require_auth();

        if Self::paused(&env) {
            panic_with_error!(&env, Error::ContractPaused);
        }
        if renter == host {
            panic_with_error!(&env, Error::SameParty);
        }
        if milestones.is_empty() {
            panic_with_error!(&env, Error::NoMilestones);
        }
        if milestones.len() > MAX_MILESTONES {
            panic_with_error!(&env, Error::TooManyMilestones);
        }

        let mut total: i128 = 0;
        let mut built: Vec<Milestone> = Vec::new(&env);
        let mut last_offset: u64 = 0;
        for (desc, amount, offset) in milestones.iter() {
            if amount <= 0 {
                panic_with_error!(&env, Error::InvalidAmount);
            }
            if desc.len() > MAX_STRING_LENGTH {
                panic_with_error!(&env, Error::StringTooLong);
            }
            if offset < last_offset {
                panic_with_error!(&env, Error::NonChronologicalMilestones);
            }
            last_offset = offset;
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
            host_accepted: !requires_host_acceptance,
            created_at: env.ledger().timestamp(),
        };
        env.storage().persistent().set(&DataKey::Escrow(id), &escrow);
        Self::index_party_escrow(&env, &renter, id);
        Self::index_party_escrow(&env, &host, id);

        env.events()
            .publish((symbol_short!("escrow"), symbol_short!("created")), (id, renter, host, total));

        id
    }

    /// All escrow IDs where `party` is either the renter or the host,
    /// newest first isn't guaranteed - just insertion order. Meant for the
    /// indexer/frontend to avoid scanning every escrow ID to find a user's
    /// history.
    pub fn get_escrows_for_party(env: Env, party: Address) -> Vec<u32> {
        env.storage()
            .persistent()
            .get(&DataKey::PartyEscrows(party))
            .unwrap_or(Vec::new(&env))
    }

    /// When `create_escrow` was called with `requires_host_acceptance =
    /// true`, the host must call this before the renter can `deposit` -
    /// otherwise a renter could lock a host into rental terms they never
    /// agreed to. A no-op requirement for the (default) non-gated case,
    /// where `deposit` never checks `host_accepted` in the first place.
    pub fn accept_escrow(env: Env, host: Address, escrow_id: u32) {
        host.require_auth();
        let mut escrow = Self::load_escrow(&env, escrow_id);
        if escrow.host != host {
            panic_with_error!(&env, Error::NotAuthorized);
        }
        if escrow.status != EscrowStatus::Created {
            panic_with_error!(&env, Error::InvalidState);
        }
        escrow.host_accepted = true;
        env.storage()
            .persistent()
            .set(&DataKey::Escrow(escrow_id), &escrow);
    }

    /// Host declines proposed terms outright, cancelling the escrow before
    /// any funds move.
    pub fn reject_escrow(env: Env, host: Address, escrow_id: u32) {
        host.require_auth();
        let mut escrow = Self::load_escrow(&env, escrow_id);
        if escrow.host != host {
            panic_with_error!(&env, Error::NotAuthorized);
        }
        if escrow.status != EscrowStatus::Created {
            panic_with_error!(&env, Error::InvalidState);
        }
        escrow.status = EscrowStatus::Cancelled;
        env.storage()
            .persistent()
            .set(&DataKey::Escrow(escrow_id), &escrow);
    }

    /// Append a milestone to an escrow that hasn't been funded yet. Only
    /// the renter can edit the milestone list, and only before `deposit`
    /// locks it in.
    pub fn add_milestone(
        env: Env,
        renter: Address,
        escrow_id: u32,
        description: String,
        amount: i128,
        auto_release_offset: u64,
    ) {
        renter.require_auth();
        let mut escrow = Self::load_escrow(&env, escrow_id);
        if escrow.renter != renter {
            panic_with_error!(&env, Error::NotAuthorized);
        }
        if escrow.status != EscrowStatus::Created {
            panic_with_error!(&env, Error::InvalidState);
        }
        if amount <= 0 {
            panic_with_error!(&env, Error::InvalidAmount);
        }
        if description.len() > MAX_STRING_LENGTH {
            panic_with_error!(&env, Error::StringTooLong);
        }
        if escrow.milestones.len() >= MAX_MILESTONES {
            panic_with_error!(&env, Error::TooManyMilestones);
        }
        if let Some(last) = escrow.milestones.last() {
            if auto_release_offset < last.auto_release_offset {
                panic_with_error!(&env, Error::NonChronologicalMilestones);
            }
        }

        escrow.milestones.push_back(Milestone {
            description,
            amount,
            auto_release_offset,
            released: false,
            auto_release_at: 0,
        });
        escrow.total_amount += amount;
        env.storage()
            .persistent()
            .set(&DataKey::Escrow(escrow_id), &escrow);
    }

    /// Remove a milestone from an unfunded escrow. At least one milestone
    /// must always remain - use `cancel_escrow` to abandon the whole thing.
    pub fn remove_milestone(env: Env, renter: Address, escrow_id: u32, milestone_index: u32) {
        renter.require_auth();
        let mut escrow = Self::load_escrow(&env, escrow_id);
        if escrow.renter != renter {
            panic_with_error!(&env, Error::NotAuthorized);
        }
        if escrow.status != EscrowStatus::Created {
            panic_with_error!(&env, Error::InvalidState);
        }
        if escrow.milestones.len() <= 1 {
            panic_with_error!(&env, Error::NoMilestones);
        }
        let m = escrow
            .milestones
            .get(milestone_index)
            .unwrap_or_else(|| panic_with_error!(&env, Error::InvalidMilestone));

        escrow.total_amount -= m.amount;
        escrow.milestones.remove(milestone_index);
        env.storage()
            .persistent()
            .set(&DataKey::Escrow(escrow_id), &escrow);
    }

    /// Renter locks the full escrow amount. Sets each milestone's
    /// `auto_release_at` relative to this deposit timestamp.
    pub fn deposit(env: Env, renter: Address, escrow_id: u32) {
        renter.require_auth();
        if Self::paused(&env) {
            panic_with_error!(&env, Error::ContractPaused);
        }
        let mut escrow = Self::load_escrow(&env, escrow_id);

        if escrow.renter != renter {
            panic_with_error!(&env, Error::NotAuthorized);
        }
        if escrow.status != EscrowStatus::Created {
            panic_with_error!(&env, Error::InvalidState);
        }
        if !escrow.host_accepted {
            panic_with_error!(&env, Error::HostAcceptancePending);
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

    /// Permissionless cleanup for an escrow nobody ever funded or
    /// explicitly cancelled - anyone can call this once
    /// `UNFUNDED_EXPIRY_WINDOW` has passed since creation, same keeper
    /// pattern as `check_auto_release`. No funds move (none were ever
    /// deposited); this only clears out storage that would otherwise sit
    /// abandoned indefinitely.
    pub fn expire_unfunded_escrow(env: Env, escrow_id: u32) {
        let mut escrow = Self::load_escrow(&env, escrow_id);
        if escrow.status != EscrowStatus::Created {
            panic_with_error!(&env, Error::InvalidState);
        }
        if env.ledger().timestamp() < escrow.created_at + UNFUNDED_EXPIRY_WINDOW {
            panic_with_error!(&env, Error::NotYetExpired);
        }
        escrow.status = EscrowStatus::Cancelled;
        env.storage()
            .persistent()
            .set(&DataKey::Escrow(escrow_id), &escrow);

        env.events()
            .publish((symbol_short!("escrow"), symbol_short!("expired")), escrow_id);
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

    /// Convenience wrapper around `confirm_milestone` that releases every
    /// remaining unreleased milestone in one call, instead of requiring one
    /// transaction per milestone when the renter is happy to sign off on
    /// all of them at once.
    pub fn confirm_all_milestones(env: Env, renter: Address, escrow_id: u32) {
        renter.require_auth();
        let mut escrow = Self::load_escrow(&env, escrow_id);
        if escrow.renter != renter {
            panic_with_error!(&env, Error::NotAuthorized);
        }
        for i in 0..escrow.milestones.len() {
            if !escrow.milestones.get(i).unwrap().released {
                Self::release_milestone_internal(&env, &mut escrow, i);
            }
        }
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

    /// Renter grants the host extra time before a milestone's auto-release
    /// deadline - a unilateral, host-favorable action (like an early
    /// confirm, just in the other direction), so only the renter's
    /// signature is required. Capped per-call to keep a fat-fingered value
    /// from parking a milestone in limbo for centuries.
    pub fn extend_milestone_deadline(
        env: Env,
        renter: Address,
        escrow_id: u32,
        milestone_index: u32,
        additional_seconds: u64,
    ) {
        renter.require_auth();
        let mut escrow = Self::load_escrow(&env, escrow_id);
        if escrow.renter != renter {
            panic_with_error!(&env, Error::NotAuthorized);
        }
        if escrow.status != EscrowStatus::Active {
            panic_with_error!(&env, Error::InvalidState);
        }
        if additional_seconds == 0 || additional_seconds > MAX_DEADLINE_EXTENSION_SECONDS {
            panic_with_error!(&env, Error::InvalidAmount);
        }

        let mut m = escrow
            .milestones
            .get(milestone_index)
            .unwrap_or_else(|| panic_with_error!(&env, Error::InvalidMilestone));
        if m.released {
            panic_with_error!(&env, Error::AlreadyReleased);
        }
        m.auto_release_at = m
            .auto_release_at
            .checked_add(additional_seconds)
            .unwrap_or_else(|| panic_with_error!(&env, Error::InvalidAmount));
        escrow.milestones.set(milestone_index, m);
        env.storage()
            .persistent()
            .set(&DataKey::Escrow(escrow_id), &escrow);
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
        if evidence_uri.is_empty() {
            panic_with_error!(&env, Error::MissingEvidence);
        }
        if evidence_uri.len() > MAX_STRING_LENGTH {
            panic_with_error!(&env, Error::StringTooLong);
        }
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

        let jurors = Self::select_jurors(&env, escrow_id, &escrow.renter, &escrow.host);
        for j in jurors.iter() {
            Self::inc_active_dispute_count(&env, &j);
        }
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
            voting_deadline: env.ledger().timestamp() + Self::dispute_voting_window(&env),
            additional_evidence: Vec::new(&env),
        };
        env.storage()
            .persistent()
            .set(&DataKey::Dispute(escrow_id, milestone_index), &dispute);

        escrow.status = EscrowStatus::Disputed;
        escrow.dispute_id = Some(milestone_index);
        env.storage()
            .persistent()
            .set(&DataKey::Escrow(escrow_id), &escrow);

        env.events().publish(
            (symbol_short!("escrow"), symbol_short!("disputed")),
            (escrow_id, milestone_index, caller),
        );

        escrow_id
    }

    /// Register as a juror by staking at least the configured minimum of
    /// `asset`. Calling this again with more of the *same* asset tops up
    /// the existing stake; switching assets is rejected outright since
    /// stake, slashing, and rewards are all tracked in a single asset per
    /// juror (see `JurorStakeInfo`).
    pub fn register_juror(env: Env, juror: Address, asset: Address, stake: i128) {
        juror.require_auth();
        let params = Self::juror_params(&env);
        if stake < params.min_stake {
            panic_with_error!(&env, Error::InsufficientStake);
        }
        if Self::get_reputation(env.clone(), juror.clone()) < params.min_reputation {
            panic_with_error!(&env, Error::ReputationTooLow);
        }
        let token = soroban_sdk::token::Client::new(&env, &asset);
        token.transfer(&juror, &env.current_contract_address(), &stake);

        let existing: Option<JurorStakeInfo> = env
            .storage()
            .persistent()
            .get(&DataKey::JurorStake(juror.clone()));
        let total_stake = match existing {
            Some(info) => {
                if info.asset != asset {
                    panic_with_error!(&env, Error::AssetMismatch);
                }
                info.amount + stake
            }
            None => stake,
        };
        env.storage().persistent().set(
            &DataKey::JurorStake(juror.clone()),
            &JurorStakeInfo { asset, amount: total_stake },
        );

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

    /// Withdraw the caller's full juror stake and leave the pool. Blocked
    /// while assigned to any dispute that hasn't been resolved yet, so a
    /// juror can't dodge an unfavorable vote by pulling their stake out
    /// from under it.
    pub fn withdraw_juror_stake(env: Env, juror: Address) {
        juror.require_auth();

        let active: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::ActiveDisputeCount(juror.clone()))
            .unwrap_or(0);
        if active > 0 {
            panic_with_error!(&env, Error::JurorHasActiveDispute);
        }

        let info: JurorStakeInfo = env
            .storage()
            .persistent()
            .get(&DataKey::JurorStake(juror.clone()))
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotAJuror));

        let token = soroban_sdk::token::Client::new(&env, &info.asset);
        token.transfer(&env.current_contract_address(), &juror, &info.amount);

        env.storage()
            .persistent()
            .remove(&DataKey::JurorStake(juror.clone()));

        let mut pool: Vec<Address> = env
            .storage()
            .persistent()
            .get(&DataKey::JurorPool)
            .unwrap_or(Vec::new(&env));
        if let Some(idx) = pool.first_index_of(&juror) {
            pool.remove(idx);
        }
        env.storage().persistent().set(&DataKey::JurorPool, &pool);

        env.events().publish(
            (symbol_short!("juror"), symbol_short!("left")),
            (juror, info.amount),
        );
    }

    pub fn get_juror_stake(env: Env, juror: Address) -> Option<JurorStakeInfo> {
        env.storage().persistent().get(&DataKey::JurorStake(juror))
    }

    /// A juror assigned to this dispute casts a vote for who should receive
    /// the disputed milestone amount.
    pub fn vote_dispute(
        env: Env,
        juror: Address,
        escrow_id: u32,
        milestone_index: u32,
        vote_for_renter: bool,
    ) {
        juror.require_auth();
        let mut dispute: Dispute = env
            .storage()
            .persistent()
            .get(&DataKey::Dispute(escrow_id, milestone_index))
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
            .set(&DataKey::Dispute(escrow_id, milestone_index), &dispute);
    }

    /// Either party (not just whoever opened the dispute) can attach more
    /// evidence for jurors to consider before voting closes. `evidence_uri`
    /// on raise_dispute is only ever the opener's initial submission -
    /// this is how the other side gets to respond on-chain instead of only
    /// off-chain.
    pub fn add_dispute_evidence(
        env: Env,
        caller: Address,
        escrow_id: u32,
        milestone_index: u32,
        evidence_uri: String,
    ) {
        caller.require_auth();
        if evidence_uri.is_empty() {
            panic_with_error!(&env, Error::MissingEvidence);
        }
        if evidence_uri.len() > MAX_STRING_LENGTH {
            panic_with_error!(&env, Error::StringTooLong);
        }

        let escrow = Self::load_escrow(&env, escrow_id);
        if caller != escrow.renter && caller != escrow.host {
            panic_with_error!(&env, Error::NotAuthorized);
        }

        let mut dispute: Dispute = env
            .storage()
            .persistent()
            .get(&DataKey::Dispute(escrow_id, milestone_index))
            .unwrap_or_else(|| panic_with_error!(&env, Error::NoDispute));
        if dispute.resolved {
            panic_with_error!(&env, Error::AlreadyResolved);
        }
        if dispute.additional_evidence.len() >= MAX_ADDITIONAL_EVIDENCE {
            panic_with_error!(&env, Error::TooMuchEvidence);
        }

        dispute.additional_evidence.push_back(evidence_uri);
        env.storage()
            .persistent()
            .set(&DataKey::Dispute(escrow_id, milestone_index), &dispute);
    }

    /// Tally votes once all jurors have voted (see
    /// `force_resolve_stale_dispute` for the case where they don't).
    /// Distributes the disputed milestone amount to the winning side,
    /// updates reputation for both parties, and slashes/rewards jurors
    /// per `apply_juror_incentives`.
    pub fn resolve_dispute(env: Env, escrow_id: u32, milestone_index: u32) {
        let mut dispute: Dispute = env
            .storage()
            .persistent()
            .get(&DataKey::Dispute(escrow_id, milestone_index))
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

        let arbitration_fee_bps = Self::juror_params(&env).arbitration_fee_bps;
        let arbitration_fee = (m.amount * arbitration_fee_bps as i128) / BPS_DENOMINATOR;
        if arbitration_fee > 0 && !dispute.jurors.is_empty() {
            let share = arbitration_fee / dispute.jurors.len() as i128;
            if share > 0 {
                for juror in dispute.jurors.iter() {
                    token.transfer(&env.current_contract_address(), &juror, &share);
                }
            }
        }

        let recipient = if renter_wins { &escrow.renter } else { &escrow.host };
        Self::pay_out(&env, &token, recipient, m.amount - arbitration_fee);

        let mut m = m;
        m.released = true;
        escrow.milestones.set(dispute.milestone_index, m);

        // Reputation: winner +2, loser -1. Simple and tunable; the point is
        // that outcomes persist and feed future trust decisions off-chain.
        Self::adjust_reputation(&env, recipient, 2);
        let loser = if renter_wins { &escrow.host } else { &escrow.renter };
        Self::adjust_reputation(&env, loser, -1);

        Self::apply_juror_incentives(&env, &dispute, renter_wins);

        // If every milestone is now released, mark the escrow complete;
        // otherwise unfreeze it so remaining milestones can proceed.
        escrow.status = if Self::all_released(&escrow) {
            EscrowStatus::Completed
        } else {
            EscrowStatus::Active
        };
        escrow.dispute_id = None;

        for j in dispute.jurors.iter() {
            Self::dec_active_dispute_count(&env, &j);
        }

        env.storage()
            .persistent()
            .set(&DataKey::Dispute(escrow_id, milestone_index), &dispute);
        env.storage()
            .persistent()
            .set(&DataKey::Escrow(escrow_id), &escrow);

        env.events().publish(
            (symbol_short!("escrow"), symbol_short!("resolved")),
            (escrow_id, dispute.milestone_index, recipient.clone()),
        );
        Self::maybe_emit_completed(&env, &escrow);
    }

    /// Anyone can call this once `voting_deadline` has passed if jurors
    /// still haven't cast enough votes for `resolve_dispute` to settle the
    /// dispute normally. Splits the disputed milestone amount 50/50 between
    /// renter and host instead of leaving it frozen indefinitely - neither
    /// party's reputation is adjusted, since a stalled jury isn't either
    /// party's fault.
    pub fn force_resolve_stale_dispute(env: Env, escrow_id: u32, milestone_index: u32) {
        let mut dispute: Dispute = env
            .storage()
            .persistent()
            .get(&DataKey::Dispute(escrow_id, milestone_index))
            .unwrap_or_else(|| panic_with_error!(&env, Error::NoDispute));
        if dispute.resolved {
            panic_with_error!(&env, Error::AlreadyResolved);
        }
        if env.ledger().timestamp() < dispute.voting_deadline {
            panic_with_error!(&env, Error::TooEarly);
        }

        let mut escrow = Self::load_escrow(&env, escrow_id);
        let m = escrow.milestones.get(dispute.milestone_index).unwrap();
        let token = soroban_sdk::token::Client::new(&env, &escrow.asset);
        let renter_share = m.amount / 2;
        let host_share = m.amount - renter_share;
        Self::pay_out(&env, &token, &escrow.renter, renter_share);
        Self::pay_out(&env, &token, &escrow.host, host_share);

        let mut m = m;
        m.released = true;
        escrow.milestones.set(dispute.milestone_index, m);

        dispute.outcome = DisputeOutcome::Split;
        dispute.resolved = true;
        for j in dispute.jurors.iter() {
            Self::dec_active_dispute_count(&env, &j);
        }

        escrow.status = if Self::all_released(&escrow) {
            EscrowStatus::Completed
        } else {
            EscrowStatus::Active
        };
        escrow.dispute_id = None;

        env.storage()
            .persistent()
            .set(&DataKey::Dispute(escrow_id, milestone_index), &dispute);
        env.storage()
            .persistent()
            .set(&DataKey::Escrow(escrow_id), &escrow);

        env.events().publish(
            (symbol_short!("escrow"), symbol_short!("stale")),
            (escrow_id, dispute.milestone_index),
        );
        Self::maybe_emit_completed(&env, &escrow);
    }

    pub fn get_escrow(env: Env, escrow_id: u32) -> Escrow {
        Self::load_escrow(&env, escrow_id)
    }

    pub fn get_dispute(env: Env, escrow_id: u32, milestone_index: u32) -> Option<Dispute> {
        env.storage()
            .persistent()
            .get(&DataKey::Dispute(escrow_id, milestone_index))
    }

    pub fn get_reputation(env: Env, who: Address) -> i32 {
        env.storage()
            .persistent()
            .get(&DataKey::Reputation(who))
            .unwrap_or(0)
    }

    /// Total number of escrows ever created (equivalently, one past the
    /// highest valid escrow ID) - lets the indexer know the upper bound to
    /// scan without tracking it separately off-chain.
    pub fn get_escrow_count(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&DataKey::EscrowCounter)
            .unwrap_or(0)
    }

    /// How many unresolved disputes `juror` is currently assigned to.
    /// Lets a frontend show *why* withdraw_juror_stake would fail instead
    /// of the juror finding out from a failed transaction.
    pub fn get_active_dispute_count(env: Env, juror: Address) -> u32 {
        env.storage()
            .persistent()
            .get(&DataKey::ActiveDisputeCount(juror))
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
        Self::maybe_emit_completed(env, escrow);
    }

    /// Emits a dedicated completion event so the indexer doesn't have to
    /// re-fetch and scan every milestone after each `released`/`resolved`
    /// event just to find out whether that was the last one.
    fn maybe_emit_completed(env: &Env, escrow: &Escrow) {
        if escrow.status == EscrowStatus::Completed {
            env.events()
                .publish((symbol_short!("escrow"), symbol_short!("complete")), escrow.id);
        }
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
    /// pool (excluding the two parties to this escrow - a renter or host
    /// who also registered as a juror must not be able to sit on their own
    /// dispute). Replace with weighted-random selection (e.g. VRF or a
    /// commit-reveal seed) before relying on this for anything real —
    /// deterministic "first N" selection is trivially gameable.
    fn select_jurors(env: &Env, seed_escrow_id: u32, renter: &Address, host: &Address) -> Vec<Address> {
        let pool: Vec<Address> = env
            .storage()
            .persistent()
            .get(&DataKey::JurorPool)
            .unwrap_or(Vec::new(env));

        let mut eligible = Vec::new(env);
        for candidate in pool.iter() {
            if &candidate != renter && &candidate != host {
                eligible.push_back(candidate);
            }
        }

        let mut selected = Vec::new(env);
        let pool_len = eligible.len();
        if pool_len == 0 {
            panic_with_error!(env, Error::NoJurorsAvailable);
        }
        let jury_size = Self::juror_params(env).jury_size;
        let take = if jury_size < pool_len { jury_size } else { pool_len };
        // rotate the starting index by escrow id so consecutive disputes
        // don't always draw the same jurors — still not sybil-resistant,
        // see the note above.
        let start = seed_escrow_id % pool_len;
        for i in 0..take {
            let idx = (start + i) % pool_len;
            selected.push_back(eligible.get(idx).unwrap());
        }
        selected
    }

    fn index_party_escrow(env: &Env, party: &Address, escrow_id: u32) {
        let mut ids: Vec<u32> = env
            .storage()
            .persistent()
            .get(&DataKey::PartyEscrows(party.clone()))
            .unwrap_or(Vec::new(env));
        ids.push_back(escrow_id);
        env.storage()
            .persistent()
            .set(&DataKey::PartyEscrows(party.clone()), &ids);
    }

    fn paused(env: &Env) -> bool {
        env.storage().instance().get(&DataKey::Paused).unwrap_or(false)
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

    /// Slashes the configured `slash_bps` of each minority-side juror's stake and
    /// redistributes it evenly among majority-side jurors staked in the
    /// same asset as the slashed stake. A minority juror's stake asset
    /// might not match any majority juror's (jurors aren't required to
    /// all stake the same asset) - in that case the slashed amount simply
    /// stays put as part of the contract's balance rather than being lost
    /// or misdirected to an unrelated asset's jurors.
    fn apply_juror_incentives(env: &Env, dispute: &Dispute, renter_wins: bool) {
        let (majority, minority) = if renter_wins {
            (&dispute.votes_for_renter, &dispute.votes_for_host)
        } else {
            (&dispute.votes_for_host, &dispute.votes_for_renter)
        };

        let slash_bps = Self::juror_params(env).slash_bps;
        let mut slash_assets: Vec<Address> = Vec::new(env);
        let mut slash_amounts: Vec<i128> = Vec::new(env);

        for juror in minority.iter() {
            let mut info: JurorStakeInfo = match env
                .storage()
                .persistent()
                .get(&DataKey::JurorStake(juror.clone()))
            {
                Some(info) => info,
                // juror already withdrew everything between voting and
                // resolution - nothing left to slash.
                None => continue,
            };
            let slash = (info.amount * slash_bps as i128) / BPS_DENOMINATOR;
            info.amount -= slash;
            env.storage()
                .persistent()
                .set(&DataKey::JurorStake(juror.clone()), &info);

            match slash_assets.first_index_of(&info.asset) {
                Some(idx) => slash_amounts.set(idx, slash_amounts.get(idx).unwrap() + slash),
                None => {
                    slash_assets.push_back(info.asset.clone());
                    slash_amounts.push_back(slash);
                }
            }
        }

        for i in 0..slash_assets.len() {
            let asset = slash_assets.get(i).unwrap();
            let pool = slash_amounts.get(i).unwrap();
            if pool == 0 {
                continue;
            }

            let mut eligible: Vec<Address> = Vec::new(env);
            for juror in majority.iter() {
                if let Some(info) = env
                    .storage()
                    .persistent()
                    .get::<_, JurorStakeInfo>(&DataKey::JurorStake(juror.clone()))
                {
                    if info.asset == asset {
                        eligible.push_back(juror);
                    }
                }
            }
            if eligible.is_empty() {
                continue;
            }

            let share = pool / eligible.len() as i128;
            if share == 0 {
                continue;
            }
            for juror in eligible.iter() {
                let mut info: JurorStakeInfo = env
                    .storage()
                    .persistent()
                    .get(&DataKey::JurorStake(juror.clone()))
                    .unwrap();
                info.amount += share;
                env.storage()
                    .persistent()
                    .set(&DataKey::JurorStake(juror.clone()), &info);
            }
        }
    }

    fn inc_active_dispute_count(env: &Env, juror: &Address) {
        let count: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::ActiveDisputeCount(juror.clone()))
            .unwrap_or(0);
        env.storage()
            .persistent()
            .set(&DataKey::ActiveDisputeCount(juror.clone()), &(count + 1));
    }

    fn dec_active_dispute_count(env: &Env, juror: &Address) {
        let count: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::ActiveDisputeCount(juror.clone()))
            .unwrap_or(0);
        env.storage()
            .persistent()
            .set(&DataKey::ActiveDisputeCount(juror.clone()), &count.saturating_sub(1));
    }

    fn dispute_voting_window(env: &Env) -> u64 {
        env.storage()
            .instance()
            .get(&DataKey::DisputeVotingWindow)
            .unwrap_or(DEFAULT_DISPUTE_VOTING_WINDOW)
    }

    fn juror_params(env: &Env) -> JurorParams {
        env.storage()
            .instance()
            .get(&DataKey::JurorParams)
            .unwrap_or(JurorParams {
                min_stake: DEFAULT_MIN_JUROR_STAKE,
                jury_size: DEFAULT_JURY_SIZE,
                slash_bps: DEFAULT_JUROR_SLASH_BPS,
                min_reputation: i32::MIN,
                arbitration_fee_bps: 0,
            })
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
