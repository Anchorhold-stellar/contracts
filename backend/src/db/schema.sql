-- SafeTrust v2 schema.
-- These tables are a *read model* materialized from Soroban contract events
-- by the indexer (src/indexer/soroban-indexer.js). The contract is the
-- source of truth for fund custody; Postgres/Hasura exist for fast reads,
-- notifications, and evidence storage that doesn't belong on-chain.

CREATE TABLE IF NOT EXISTS listings (
  id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  host_wallet   TEXT NOT NULL,
  title         TEXT NOT NULL,
  description   TEXT,
  vertical      TEXT NOT NULL DEFAULT 'rental', -- rental | equipment | service
  created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS escrows (
  escrow_id       INTEGER PRIMARY KEY,        -- matches on-chain u32 id
  listing_id      UUID REFERENCES listings(id),
  renter_wallet   TEXT NOT NULL,
  host_wallet     TEXT NOT NULL,
  asset_address   TEXT NOT NULL,
  total_amount    NUMERIC NOT NULL,
  status          TEXT NOT NULL DEFAULT 'created', -- created|active|disputed|completed|cancelled
  created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS milestones (
  id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  escrow_id           INTEGER NOT NULL REFERENCES escrows(escrow_id),
  milestone_index     INTEGER NOT NULL,
  description         TEXT NOT NULL,
  amount              NUMERIC NOT NULL,
  auto_release_at     TIMESTAMPTZ,
  released            BOOLEAN NOT NULL DEFAULT false,
  released_at         TIMESTAMPTZ,
  UNIQUE (escrow_id, milestone_index)
);

CREATE TABLE IF NOT EXISTS disputes (
  escrow_id           INTEGER PRIMARY KEY REFERENCES escrows(escrow_id),
  milestone_index     INTEGER NOT NULL,
  opened_by_wallet    TEXT NOT NULL,
  evidence_uri        TEXT,
  resolved            BOOLEAN NOT NULL DEFAULT false,
  outcome             TEXT NOT NULL DEFAULT 'pending', -- pending|renter_wins|host_wins
  opened_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
  resolved_at         TIMESTAMPTZ
);

CREATE TABLE IF NOT EXISTS dispute_evidence (
  id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  escrow_id     INTEGER NOT NULL REFERENCES disputes(escrow_id),
  submitted_by  TEXT NOT NULL,
  uri           TEXT NOT NULL, -- IPFS/Arweave URI
  note          TEXT,
  created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS jurors (
  wallet        TEXT PRIMARY KEY,
  stake_amount  NUMERIC NOT NULL,
  registered_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS reputation (
  wallet    TEXT PRIMARY KEY,
  score     INTEGER NOT NULL DEFAULT 0,
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS indexer_cursor (
  id            INTEGER PRIMARY KEY DEFAULT 1,
  last_ledger   BIGINT NOT NULL DEFAULT 0,
  CONSTRAINT single_row CHECK (id = 1)
);
INSERT INTO indexer_cursor (id, last_ledger) VALUES (1, 0) ON CONFLICT DO NOTHING;

CREATE INDEX IF NOT EXISTS idx_escrows_renter ON escrows (renter_wallet);
CREATE INDEX IF NOT EXISTS idx_escrows_host ON escrows (host_wallet);
CREATE INDEX IF NOT EXISTS idx_milestones_escrow ON milestones (escrow_id);
