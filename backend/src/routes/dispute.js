import { Router } from "express";
import { pool } from "../db/pool.js";
import { buildContractCallXdr } from "../soroban/contractCall.js";

export const disputeRouter = Router();

disputeRouter.get("/:escrowId", async (req, res) => {
  const { escrowId } = req.params;
  const dispute = await pool.query(`SELECT * FROM disputes WHERE escrow_id = $1`, [escrowId]);
  if (dispute.rows.length === 0) {
    return res.status(404).json({ error: "no dispute for this escrow" });
  }
  const evidence = await pool.query(
    `SELECT * FROM dispute_evidence WHERE escrow_id = $1 ORDER BY created_at ASC`,
    [escrowId]
  );
  res.json({ ...dispute.rows[0], evidence: evidence.rows });
});

// Off-chain evidence upload (photos, messages) referenced by URI. Storing
// the actual bytes belongs on IPFS/Arweave — this just records the pointer
// so jurors and the frontend can pull it up next to the dispute.
disputeRouter.post("/:escrowId/evidence", async (req, res) => {
  const { escrowId } = req.params;
  const { submittedBy, uri, note } = req.body;
  if (!submittedBy || !uri) {
    return res.status(400).json({ error: "submittedBy and uri are required" });
  }
  const { rows } = await pool.query(
    `INSERT INTO dispute_evidence (escrow_id, submitted_by, uri, note)
     VALUES ($1, $2, $3, $4) RETURNING *`,
    [escrowId, submittedBy, uri, note ?? null]
  );
  res.status(201).json(rows[0]);
});

disputeRouter.post("/build/raise", async (req, res) => {
  try {
    const { callerWallet, escrowId, milestoneIndex, evidenceUri } = req.body;
    const xdr = await buildContractCallXdr({
      method: "raise_dispute",
      source: callerWallet,
      args: [callerWallet, escrowId, milestoneIndex, evidenceUri],
    });
    res.json({ xdr });
  } catch (err) {
    console.error("[dispute] build/raise failed", err);
    res.status(500).json({ error: "failed to build transaction" });
  }
});

disputeRouter.post("/build/vote", async (req, res) => {
  try {
    const { jurorWallet, escrowId, voteForRenter } = req.body;
    const xdr = await buildContractCallXdr({
      method: "vote_dispute",
      source: jurorWallet,
      args: [jurorWallet, escrowId, voteForRenter],
    });
    res.json({ xdr });
  } catch (err) {
    console.error("[dispute] build/vote failed", err);
    res.status(500).json({ error: "failed to build transaction" });
  }
});

disputeRouter.post("/build/resolve", async (req, res) => {
  try {
    const { escrowId } = req.body;
    const xdr = await buildContractCallXdr({
      method: "resolve_dispute",
      source: null, // permissionless — anyone can submit once voting is complete
      args: [escrowId],
    });
    res.json({ xdr });
  } catch (err) {
    console.error("[dispute] build/resolve failed", err);
    res.status(500).json({ error: "failed to build transaction" });
  }
});
