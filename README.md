# SafeTrust v2 — Soroban Escrow Contracts

Self-custodied, milestone-capable rental escrow contracts for Stellar/Soroban.
Escrow logic lives in a contract you own: multi-release milestones, per-milestone
auto-release timeouts, and on-chain staked-juror dispute resolution. No third party
ever custodies funds or has unilateral power to move them.

This repo holds the contracts only.

## Layout

```
safetrust-v2/
└── contracts/escrow/     Soroban smart contract (Rust)
```

Full design docs, function reference, events, and error codes:
[`contracts/escrow/README.md`](contracts/escrow/README.md).

## Getting started

Requires Rust with the `wasm32-unknown-unknown` target and the Soroban CLI:

```
rustup target add wasm32-unknown-unknown
cargo install --locked soroban-cli
```

Run the test suite (unit tests exercise every function against an in-memory
ledger, including full dispute/slashing/timelock flows):

```
cd contracts/escrow
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
