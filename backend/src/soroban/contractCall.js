import "dotenv/config";
import { rpc, Contract, TransactionBuilder, Networks, nativeToScVal } from "@stellar/stellar-sdk";

const server = new rpc.Server(process.env.SOROBAN_RPC_URL);
const contract = new Contract(process.env.ESCROW_CONTRACT_ID);

/**
 * Builds an unsigned transaction XDR that invokes `method` on the escrow
 * contract with `args`. The frontend is responsible for signing this with
 * Freighter and submitting via `helper/send-transaction`-style flow — the
 * backend never touches a private key.
 *
 * `source` is the account that pays the fee and provides auth for the call.
 * Some calls (e.g. resolve_dispute) are permissionless and don't require a
 * specific source's auth beyond paying the fee — pass any funded account
 * for those, or run them from a keeper/relayer identity.
 */
export async function buildContractCallXdr({ method, source, args }) {
  if (!source) {
    throw new Error("a fee-paying source account is required to build a transaction");
  }

  const account = await server.getAccount(source);
  const scArgs = args.map(toScVal);

  const tx = new TransactionBuilder(account, {
    fee: "100000",
    networkPassphrase: process.env.SOROBAN_NETWORK_PASSPHRASE ?? Networks.TESTNET,
  })
    .addOperation(contract.call(method, ...scArgs))
    .setTimeout(60)
    .build();

  const prepared = await server.prepareTransaction(tx);
  return prepared.toXDR();
}

// Very small type-inference helper for the demo endpoints. In practice
// you'll want explicit per-method arg schemas (matching the contract's
// signatures 1:1) rather than guessing from JS types.
function toScVal(value) {
  if (typeof value === "string" && value.length === 56 && value.startsWith("G")) {
    return nativeToScVal(value, { type: "address" });
  }
  if (typeof value === "number") {
    return nativeToScVal(value, { type: "u32" });
  }
  if (typeof value === "boolean") {
    return nativeToScVal(value, { type: "bool" });
  }
  if (Array.isArray(value)) {
    return nativeToScVal(value);
  }
  return nativeToScVal(value, { type: "string" });
}
