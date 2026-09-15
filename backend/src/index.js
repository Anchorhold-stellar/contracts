import "dotenv/config";
import express from "express";
import cors from "cors";
import { escrowRouter } from "./routes/escrow.js";
import { disputeRouter } from "./routes/dispute.js";
import { webhookRouter } from "./webhook/listener.js";

const app = express();
app.use(cors());
app.use(express.json());

app.get("/health", (_req, res) => res.json({ ok: true }));

app.use("/escrows", escrowRouter);
app.use("/disputes", disputeRouter);
app.use("/webhooks", webhookRouter);

const port = process.env.PORT ?? 3002;
app.listen(port, () => {
  console.log(`[backend] listening on :${port}`);
  console.log("[backend] remember to run `npm run indexer` in a separate process");
});
