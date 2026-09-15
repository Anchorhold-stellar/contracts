import "dotenv/config";
import { rpc } from "@stellar/stellar-sdk";
import { pool } from "../db/pool.js";

const server = new rpc.Server(process.env.SOROBAN_RPC_URL);
const CONTRACT_ID = process.env.ESCROW_CONTRACT_ID;
const POLL_INTERVAL_MS = Number(process.env.INDEXER_POLL_INTERVAL_MS ?? 5000);

/**
 * This is a polling indexer, not a subscription — simplest thing that works
 * for a testnet-scale project. For production volume, swap this loop for a
 * proper event-streaming setup (or a hosted indexer) but keep the same
 * `applyEvent` dispatch below; that's the part that encodes your data model.
 *
 * TODO: replace the stubbed `fetchEventsSince` with a real call to
 * `server.getEvents(...)` filtered to CONTRACT_ID, once you've deployed and
 * know the exact topic/event shapes your contract emits.
 */
async function run() {
  console.log(`[indexer] watching contract ${CONTRACT_ID} on ${process.env.SOROBAN_RPC_URL}`);
  for (;;) {
    try {
      const { rows } = await pool.query(`SELECT last_ledger FROM indexer_cursor WHERE id = 1`);
      const lastLedger = rows[0]?.last_ledger ?? 0;

      const events = await fetchEventsSince(lastLedger);
      for (const event of events) {
        await applyEvent(event);
      }

      if (events.length > 0) {
        const newCursor = events[events.length - 1].ledger;
        await pool.query(`UPDATE indexer_cursor SET last_ledger = $1 WHERE id = 1`, [newCursor]);
      }
    } catch (err) {
      console.error("[indexer] poll iteration failed", err);
    }
    await sleep(POLL_INTERVAL_MS);
  }
}

async function fetchEventsSince(lastLedger) {
  // TODO: real implementation, roughly:
  //
  // const latest = await server.getLatestLedger();
  // return server.getEvents({
  //   startLedger: lastLedger + 1,
  //   filters: [{ type: "contract", contractIds: [CONTRACT_ID] }],
  // });
  //
  // then map each raw event's topics/value into the shape `applyEvent`
  // expects below. Left as a stub so this file runs without a live
  // deployment while you build out the rest of the stack.
  return [];
}

async function applyEvent(event) {
  switch (event.type) {
    case "escrow_created":
      return pool.query(
        `INSERT INTO escrows (escrow_id, renter_wallet, host_wallet, asset_address, total_amount, status)
         VALUES ($1, $2, $3, $4, $5, 'created')
         ON CONFLICT (escrow_id) DO NOTHING`,
        [event.escrowId, event.renter, event.host, event.asset, event.totalAmount]
      );
    case "escrow_funded":
      return pool.query(`UPDATE escrows SET status = 'active', updated_at = now() WHERE escrow_id = $1`, [
        event.escrowId,
      ]);
    case "milestone_released":
      await pool.query(
        `UPDATE milestones SET released = true, released_at = now()
         WHERE escrow_id = $1 AND milestone_index = $2`,
        [event.escrowId, event.milestoneIndex]
      );
      if (event.escrowCompleted) {
        await pool.query(`UPDATE escrows SET status = 'completed', updated_at = now() WHERE escrow_id = $1`, [
          event.escrowId,
        ]);
      }
      return;
    case "dispute_opened":
      return pool.query(
        `INSERT INTO disputes (escrow_id, milestone_index, opened_by_wallet, evidence_uri)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (escrow_id) DO NOTHING`,
        [event.escrowId, event.milestoneIndex, event.openedBy, event.evidenceUri]
      );
    case "dispute_resolved":
      return pool.query(
        `UPDATE disputes SET resolved = true, outcome = $2, resolved_at = now() WHERE escrow_id = $1`,
        [event.escrowId, event.outcome]
      );
    default:
      console.warn("[indexer] unrecognized event type", event.type);
  }
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

run();
