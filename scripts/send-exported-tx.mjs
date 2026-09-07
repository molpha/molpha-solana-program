import fs from "node:fs";
import { Connection, Keypair, VersionedTransaction } from "@solana/web3.js";

const RPC = process.env.ANCHOR_PROVIDER_URL;
const TX_B64 = process.env.TX_B64;
const KEYPAIR_PATH = `${process.env.HOME}/.config/solana/id.json`;

async function main() {
  const secret = Uint8Array.from(
    JSON.parse(fs.readFileSync(KEYPAIR_PATH, "utf8")),
  );
  const payer = Keypair.fromSecretKey(secret);
  const connection = new Connection(RPC, "confirmed");

  const raw = Buffer.from(TX_B64, "base64");
  const tx = VersionedTransaction.deserialize(raw);
  tx.sign([payer]);

  const sim = await connection.simulateTransaction(tx, { sigVerify: true });
  console.log("sim err:", sim.value.err);
  if (sim.value.logs) console.log("logs:\n" + sim.value.logs.join("\n"));

  if (sim.value.err) process.exit(1);

  if (process.env.SEND === "1") {
    const sig = await connection.sendRawTransaction(tx.serialize());
    console.log("sent:", sig);
    const conf = await connection.confirmTransaction(sig, "confirmed");
    console.log("confirmation:", conf.value);
  } else {
    console.log("simulation ok (set SEND=1 to broadcast)");
  }
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
