import { Router } from "express";
import { pool } from "../db/pool.js";

export const webhookRouter = Router();

/**
 * Generic inbound webhook for off-chain integrations — e.g. a keeper
 * service that calls `check_auto_release` on-chain and then pings this
 * endpoint so the read model updates without waiting for the next indexer
 * poll, or a notification provider confirming delivery.
 *
 * Keep this separate from the indexer: the indexer is the source of truth
 * for on-chain state; this endpoint is only for out-of-band signals that
 * don't come from contract events.
 */
webhookRouter.post("/keeper-ping", async (req, res) => {
  const { escrowId, milestoneIndex } = req.body;
  if (escrowId == null || milestoneIndex == null) {
    return res.status(400).json({ error: "escrowId and milestoneIndex required" });
  }
  await pool.query(
    `UPDATE milestones SET released = true, released_at = now()
     WHERE escrow_id = $1 AND milestone_index = $2`,
    [escrowId, milestoneIndex]
  );
  res.status(204).end();
});
