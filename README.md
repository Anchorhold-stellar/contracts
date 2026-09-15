# SafeTrust v2 — Milestone Escrow on Soroban

A self-custodied, milestone-capable rental escrow platform for Stellar/Soroban.
Built as a feature expansion on the SafeTrust concept (decentralized P2P escrow
for rentals) — the core difference here is that escrow logic lives in a
**contract you own**, not a third-party hosted API, and supports multi-release
milestones plus on-chain dispute resolution instead of a single release/refund.

## Monorepo layout

```
safetrust-v2/
├── contracts/escrow/     Soroban smart contract (Rust)
├── backend/              Express API + event indexer + Postgres schema
├── frontend/             Next.js 14 app (renter / host / juror views)
└── docker-compose.yml    Local Postgres + Hasura
```

## How the pieces connect

```
Soroban contract (contracts/escrow)
        │  emits events (deposit, milestone_release, dispute_opened, dispute_resolved)
        ▼
backend/src/indexer  ── polls/subscribes to contract events, writes to Postgres
        │
        ▼
Postgres  ── exposed via Hasura GraphQL (docker-compose.yml)
        │
        ▼
backend/src/routes  ── REST endpoints for actions the frontend can't sign itself
        │                (building unsigned XDR, submitting evidence, juror registration)
        ▼
frontend (Next.js)  ── renter/host/juror dashboards, Freighter wallet signing
```

## Getting started

### 1. Contract

```
cd contracts/escrow
cargo build --target wasm32-unknown-unknown --release
```

Requires the Soroban CLI and Rust wasm32 target:
```
rustup target add wasm32-unknown-unknown
cargo install --locked soroban-cli
```

Deploy to testnet:
```
soroban contract deploy \
  --wasm target/wasm32-unknown-unknown/release/escrow.wasm \
  --source <your-identity> \
  --network testnet
```

### 2. Backend

```
cd backend
cp .env.example .env   # fill in RPC URL, contract ID, DB creds
npm install
npm run dev
```

### 3. Database + Hasura

```
docker compose up -d
```

### 4. Frontend

```
cd frontend
npm install
npm run dev
```

## What's implemented vs. stubbed

This scaffold gives you real structure and working logic for the pieces that
define the project (contract state machine, indexer shape, API surface,
frontend flow) but a few integration points are intentionally left as `TODO`
for you to fill in with your own RPC endpoint, Freighter integration details,
and Hasura metadata — those are config/wiring, not design decisions, so
finishing them shouldn't require changing the architecture.

- [x] Escrow contract: init, deposit, milestone release, auto-release timeout, dispute lifecycle, juror voting
- [x] Backend: REST routes for escrow + dispute actions, Postgres schema, indexer skeleton
- [x] Frontend: dashboard, escrow detail with milestone timeline, dispute filing UI, wallet connect stub
- [ ] TODO: wire indexer to a real Soroban RPC event stream (currently polls a stubbed function)
- [ ] TODO: Freighter signing calls (stubbed — see `frontend/components/WalletConnect.tsx`)
- [ ] TODO: Hasura metadata/permissions (schema.sql gives you tables; track them in Hasura console or via metadata YAML)
