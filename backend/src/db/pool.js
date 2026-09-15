import pg from "pg";
import "dotenv/config";

export const pool = new pg.Pool({
  connectionString: process.env.DATABASE_URL,
});

pool.on("error", (err) => {
  // A dropped connection here shouldn't crash the process — log and let
  // the pool reconnect on the next query.
  console.error("[db] unexpected pool error", err);
});
