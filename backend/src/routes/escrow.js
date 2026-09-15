import { Router } from "express";
import { pool } from "../db/pool.js";
import { buildContractCallXdr } from "../soroban/contractCall.js";

export const escrowRouter = Router();

// List escrows for a wallet, either side.
escrowRouter.get("/", async (req, res) => {
  const { wallet } = req.query;
  if (!wallet) {
    return res.status(400).json({ error: "wallet query param required" });
  }
  const { rows } = await pool.query(
    `SELECT * FROM escrows WHERE renter_wallet = $1 OR host_wallet = $1 ORDER BY created_at DESC`,
    [wallet]
  );
  res.json(rows);
});

escrowRouter.get("/:escrowId", async (req, res) => {
  const { escrowId } = req.params;
  const escrow = await pool.query(`SELECT * FROM escrows WHERE escrow_id = $1`, [escrowId]);
  if (escrow.rows.length === 0) {
    return res.status(404).json({ error: "escrow not found" });
  }
  const milestones = await pool.query(
    `SELECT * FROM milestones WHERE escrow_id = $1 ORDER BY milestone_index ASC`,
    [escrowId]
  );
  res.json({ ...escrow.rows[0], milestones: milestones.rows });
});

// Builds an unsigned XDR for `create_escrow`. The frontend signs this with
// Freighter and submits it — the backend never holds a signing key.
escrowRouter.post("/build/create", async (req, res) => {
  try {
    const { renterWallet, hostWallet, assetAddress, milestones } = req.body;
    const xdr = await buildContractCallXdr({
      method: "create_escrow",
      source: renterWallet,
      args: [renterWallet, hostWallet, assetAddress, milestones],
    });
    res.json({ xdr });
  } catch (err) {
    console.error("[escrow] build/create failed", err);
    res.status(500).json({ error: "failed to build transaction" });
  }
});

escrowRouter.post("/build/deposit", async (req, res) => {
  try {
    const { renterWallet, escrowId } = req.body;
    const xdr = await buildContractCallXdr({
      method: "deposit",
      source: renterWallet,
      args: [renterWallet, escrowId],
    });
    res.json({ xdr });
  } catch (err) {
    console.error("[escrow] build/deposit failed", err);
    res.status(500).json({ error: "failed to build transaction" });
  }
});

escrowRouter.post("/build/confirm-milestone", async (req, res) => {
  try {
    const { renterWallet, escrowId, milestoneIndex } = req.body;
    const xdr = await buildContractCallXdr({
      method: "confirm_milestone",
      source: renterWallet,
      args: [renterWallet, escrowId, milestoneIndex],
    });
    res.json({ xdr });
  } catch (err) {
    console.error("[escrow] build/confirm-milestone failed", err);
    res.status(500).json({ error: "failed to build transaction" });
  }
});
