# Escrow — Milestone Rental Escrow for Soroban

A self-custodied, milestone-capable escrow contract for Stellar/Soroban. Two
parties — a **renter** and a **host** — agree on an ordered list of
milestones and an asset; the renter deposits the full amount up front, and
funds release per-milestone either by mutual confirmation, by timeout, or
through an on-chain staked-juror dispute process. No third party ever
custodies funds or has unilateral power to move them.

## Contents

- [Design goals](#design-goals)
- [State machine](#state-machine)
- [Core flows](#core-flows)
- [Disputes and jurors](#disputes-and-jurors)
- [Governance / admin surface](#governance--admin-surface)
- [Fees](#fees)
- [Events](#events)
- [Function reference](#function-reference)
- [Error codes](#error-codes)
- [Building, testing, deploying](#building-testing-deploying)
- [Known limitations](#known-limitations)

## Design goals

- **Multi-milestone release** instead of a single deposit/release flow —
  a rental (or any staged agreement) can pay out in installments as it
  progresses, rather than all-or-nothing.
- **Auto-release per milestone** after a timeout if nobody disputes it, so
  funds don't get stuck waiting on a party who's gone silent.
- **On-chain dispute resolution** via a staked-juror vote, with a
  permissionless fallback if jurors go silent too.
- **On-chain reputation** that persists across escrows and feeds back into
  the system itself (juror eligibility, a small bonus for clean
  completions) instead of being purely informational.
- **No custodial admin power.** The admin key configures parameters (fees,
  juror requirements, pause) but can never move a specific escrow's funds,
  pick a dispute's outcome, or otherwise touch money that isn't its own.

## State machine

```
Created ──cancel_escrow──────────────────────────────────────► Cancelled
   │                                                                ▲
   │ (if requires_host_acceptance: accept_escrow / reject_escrow)   │
   │                                                          reject_escrow
   │ deposit()                                                      │
   ▼                                                                │
Active ──raise_dispute()──► Disputed ──resolve_dispute()──────► Active / Completed
   │                            │        or force_resolve_stale_dispute()
   │ confirm_milestone() /      │
   │ check_auto_release() /     └──mutual_cancel()───────────► Cancelled
   │ confirm_all_milestones()
   ▼
Completed (once every milestone is released)
```

`expire_unfunded_escrow` also moves `Created → Cancelled` for escrows
nobody ever funded (or explicitly cancelled), after a configurable window.

## Core flows

1. **`create_escrow(renter, host, asset, milestones, requires_host_acceptance)`**
   — renter proposes terms. No funds move yet. `milestones` is a list of
   `(description, amount, auto_release_offset_seconds)`; offsets must be
   non-decreasing (a later milestone can't auto-release before an earlier
   one). Before funding, the renter can still `add_milestone` /
   `remove_milestone` to adjust terms.
2. **Host acceptance (optional).** If `requires_host_acceptance` was set,
   the host must call `accept_escrow` before the renter can deposit —
   otherwise a renter could lock a host into terms they never agreed to.
   `reject_escrow` declines outright.
3. **`deposit(renter, escrow_id)`** — renter locks the full amount. This is
   the only point real tokens move into the contract; every milestone's
   `auto_release_offset` becomes an absolute deadline from this moment.
4. **Release, one of:**
   - `confirm_milestone` / `confirm_all_milestones` — renter signs off
     early.
   - `check_auto_release` — permissionless; succeeds once a milestone's
     deadline has passed and nothing disputed it.
   - Dispute resolution (below).
5. **Early exits:** `cancel_escrow` (pre-funding, either party),
   `mutual_cancel` (post-funding, both signatures required, refunds
   whatever's unreleased), `expire_unfunded_escrow` (permissionless
   cleanup for abandoned unfunded escrows).

## Disputes and jurors

- Either party calls `raise_dispute(escrow_id, milestone_index, evidence_uri)`
  on a **funded, undisputed, unreleased** milestone. This freezes the whole
  escrow (only one dispute in flight at a time), draws a jury, and
  snapshots the current slash rate, arbitration fee, and stake-weighted-
  voting setting onto the dispute record - later admin changes to any of
  those can't retroactively affect a dispute that's already open.
- `register_juror(asset, stake)` stakes into a pool; `select_jurors`
  excludes the escrow's own renter/host from being drawn on their own
  dispute, and rotates the starting index by escrow ID so consecutive
  disputes don't always draw the same jurors.
- Either party can call `add_dispute_evidence` to submit more evidence
  after the dispute opens — `evidence_uri` on `raise_dispute` is only ever
  the opener's initial submission.
- Jurors call `vote_dispute`. Once everyone assigned has voted,
  `resolve_dispute` tallies the outcome (flat headcount by default, or
  stake-weighted if the dispute was opened with `stake_weighted_voting`
  enabled), pays the winner, adjusts both parties' reputation (+2 winner /
  −1 loser), slashes losing-side juror stakes and redistributes to the
  winning side (`slash_bps`), and pays every voting juror an arbitration fee
  (`arbitration_fee_bps`, off the top, split evenly regardless of side).
- If jurors don't finish voting in time, both parties can jointly
  `extend_dispute_deadline`, or — once `voting_deadline` passes — anyone
  can call `force_resolve_stale_dispute`, which splits the amount 50/50
  and touches neither reputation nor juror stakes (a stalled jury isn't
  either party's fault).
- `withdraw_juror_stake` returns a juror's full stake and drops them from
  the pool, but is blocked while they're assigned to any unresolved
  dispute — tracked via a per-juror active-dispute counter
  (`get_active_dispute_count`).
- Dispute records are keyed by `(escrow_id, milestone_index)`, not just
  `escrow_id` — a multi-milestone escrow that disputes two different
  milestones over its lifetime keeps both dispute histories intact.

## Governance / admin surface

The admin address (set once at `initialize`, rotatable via a two-step
`transfer_admin` / `accept_admin_transfer` handoff so a typo'd address
can't permanently brick it) can:

| Configure | Function |
|---|---|
| Protocol fee | `set_fee_config(bps, treasury)` — capped at 20% |
| Per-address fee exemption | `set_fee_exempt(party, bool)` |
| Juror requirements | `set_juror_params(min_stake, jury_size, slash_bps, min_reputation, arbitration_fee_bps)` |
| Stake-weighted voting | `set_stake_weighted_voting(bool)` |
| Dispute voting window | `set_dispute_voting_window(seconds)` — bounded [1h, 30d] |
| Emergency pause | `set_paused(bool)` — blocks only new `create_escrow`/`deposit`, never touches escrows already active |

None of these let the admin touch a specific escrow's funds, pick a
dispute's winner, or bypass a party's signature on anything.

## Fees

Two independent, additive fees, both computed in basis points
(1 = 0.01%) and both optional (0/unset by default):

- **Protocol fee** (`FeeConfig`) — taken from every payout (normal release
  or dispute resolution) and sent to a treasury address, unless the
  recipient is fee-exempt.
- **Arbitration fee** (`JurorParams.arbitration_fee_bps`) — taken from a
  *disputed* payout only, before the protocol fee applies to what's left,
  and split evenly among the jurors who voted (win or lose).

## Events

| Topic | Emitted when | Payload |
|---|---|---|
| `(escrow, created)` | `create_escrow` | `(id, renter, host, total_amount)` |
| `(escrow, funded)` | `deposit` | `(id, total_amount)` |
| `(escrow, released)` | any milestone release | `(id, milestone_index, amount, host)` |
| `(escrow, complete)` | escrow's last milestone releases | `id` |
| `(escrow, cancelled)` | `cancel_escrow` | `id` |
| `(escrow, expired)` | `expire_unfunded_escrow` | `id` |
| `(escrow, mutual)` | `mutual_cancel` | `(id, refund_amount)` |
| `(escrow, disputed)` | `raise_dispute` | `(id, milestone_index, caller)` |
| `(escrow, resolved)` | `resolve_dispute` | `(id, milestone_index, recipient)` |
| `(escrow, stale)` | `force_resolve_stale_dispute` | `(id, milestone_index)` |
| `(juror, joined)` | `register_juror` | `(juror, stake)` |
| `(juror, left)` | `withdraw_juror_stake` | `(juror, amount)` |

## Function reference

**Admin / governance:** `initialize`, `transfer_admin`,
`accept_admin_transfer`, `get_admin`, `get_pending_admin`,
`set_dispute_voting_window`, `get_dispute_voting_window`,
`set_fee_config`, `get_fee_config`, `set_fee_exempt`, `is_fee_exempt`,
`set_stake_weighted_voting`, `get_stake_weighted_voting`,
`set_juror_params`, `get_juror_params`, `set_paused`, `is_paused`

**Escrow lifecycle:** `create_escrow`, `get_escrows_for_party`,
`accept_escrow`, `reject_escrow`, `add_milestone`, `remove_milestone`,
`update_milestone`, `deposit`, `cancel_escrow`, `expire_unfunded_escrow`,
`mutual_cancel`

**Milestone release:** `confirm_milestone`, `confirm_all_milestones`,
`check_auto_release`, `extend_milestone_deadline`

**Disputes:** `raise_dispute`, `add_dispute_evidence`,
`extend_dispute_deadline`, `vote_dispute`, `resolve_dispute`,
`force_resolve_stale_dispute`

**Jurors:** `register_juror`, `withdraw_juror_stake`, `get_juror_stake`,
`get_active_dispute_count`

**Read-only queries:** `get_escrow`, `get_escrow_status`, `get_milestone`,
`get_dispute`, `get_reputation`, `get_escrow_count`

Full parameter lists and per-function rationale are documented as doc
comments on each function in `src/lib.rs`.

## Error codes

| Code | Name | Meaning |
|---|---|---|
| 1 | `AlreadyInitialized` | `initialize` called twice |
| 2 | `NoMilestones` | Empty milestone list, or removing the last one |
| 3 | `InvalidAmount` | Non-positive amount, or a numeric input out of range |
| 4 | `NotAuthorized` | Caller isn't the required party/admin/juror |
| 5 | `InvalidState` | Action doesn't match the escrow's current status |
| 6 | `EscrowNotFound` | No escrow with that ID |
| 7 | `InvalidMilestone` | Milestone index out of range |
| 8 | `AlreadyReleased` | Milestone already paid out |
| 9 | `TooEarly` | Auto-release/force-resolve deadline hasn't passed yet |
| 10 | `EscrowDisputed` | Action blocked while a dispute is open |
| 11 | `NoDispute` | No dispute exists for that escrow/milestone |
| 12 | `AlreadyResolved` | Dispute already resolved |
| 13 | `NotAJuror` | Caller isn't assigned to this dispute |
| 14 | `AlreadyVoted` | Juror already cast a vote |
| 15 | `VotingIncomplete` | Not every assigned juror has voted yet |
| 16 | `InsufficientStake` | Stake below the configured minimum |
| 17 | `NoJurorsAvailable` | Juror pool is empty (or fully excluded as a party) |
| 18 | `FeeTooHigh` | Protocol fee above `MAX_FEE_BPS` |
| 19 | `InvalidJurySize` | Jury size is zero or even |
| 20 | `AssetMismatch` | Re-registering a juror with a different stake asset |
| 21 | `JurorHasActiveDispute` | Can't withdraw stake while assigned to an open dispute |
| 22 | `SameParty` | Renter and host are the same address |
| 23 | `MissingEvidence` | Empty evidence string |
| 24 | `NonChronologicalMilestones` | Milestone offsets out of order |
| 25 | `ContractPaused` | `create_escrow`/`deposit` blocked by admin pause |
| 26 | `SlashTooHigh` | Slash rate above `MAX_SLASH_BPS` |
| 27 | `ReputationTooLow` | Below `min_reputation` to register as a juror |
| 28 | `HostAcceptancePending` | Deposit blocked until the host accepts |
| 29 | `StringTooLong` | Description/evidence string above `MAX_STRING_LENGTH` |
| 30 | `TooManyMilestones` | Milestone count above `MAX_MILESTONES` |
| 31 | `NotYetExpired` | `expire_unfunded_escrow` called before its window elapsed |
| 32 | `InvalidVotingWindow` | Voting window outside `[1h, 30d]` |
| 33 | `ArbitrationFeeTooHigh` | Arbitration fee above `MAX_ARBITRATION_FEE_BPS` |
| 34 | `TooMuchEvidence` | Additional evidence above `MAX_ADDITIONAL_EVIDENCE` entries |

## Building, testing, deploying

Requires Rust with the `wasm32-unknown-unknown` target and the Soroban CLI:

```
rustup target add wasm32-unknown-unknown
cargo install --locked soroban-cli
```

Run the test suite (unit tests exercise every function against an
in-memory ledger, including full dispute/slashing/timelock flows):

```
cargo test
```

Build the deployable contract:

```
cargo build --target wasm32-unknown-unknown --release
```

Deploy to testnet:

```
soroban contract deploy \
  --wasm target/wasm32-unknown-unknown/release/escrow.wasm \
  --source <your-identity> \
  --network testnet
```

## Known limitations

These are deliberate scope boundaries, not oversights — worth knowing
before a mainnet deployment or an audit:

- **Jurors aren't required to stake a single canonical asset.** Slashing
  and arbitration-fee rewards only move between jurors staked in the
  *same* asset as the one being redistributed; a slashed amount with no
  same-asset majority juror to receive it stays in the contract's balance
  rather than being misdirected or lost.
- **Juror selection is deterministic** ("first N eligible, rotated by
  escrow ID"), not a VRF or commit-reveal scheme — fine for an
  adversarial-light environment, not sybil-resistant against a
  well-resourced attacker who can register many juror identities.
- **No per-escrow fund segregation on-chain.** All deposits and juror
  stakes in a given asset sit in one pooled contract balance; correctness
  relies on every payout path (release, dispute resolution, refunds)
  checking escrow/dispute state properly rather than on any on-chain
  accounting wall between escrows. (`raise_dispute` requiring `Active`
  status is the guard that keeps a never-funded escrow from being disputed
  into a real payout — see the fix history for why that matters.)
