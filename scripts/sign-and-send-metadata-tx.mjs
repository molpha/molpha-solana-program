import fs from "node:fs";
import * as kit from "@solana/kit";

const RPC = process.env.ANCHOR_PROVIDER_URL;
const KEYPAIR_PATH = `${process.env.HOME}/.config/solana/id.json`;
const TX_B64 =
  process.env.TX_B64 ||
  fs.readFileSync("/tmp/idl-export.txt", "utf8").trim().split("\n").pop();

async function confirmSignature(rpc, signature) {
  for (let i = 0; i < 60; i++) {
    const { value } = await rpc
      .getSignatureStatuses([signature], { searchTransactionHistory: true })
      .send();
    const status = value[0];
    if (status?.err)
      throw new Error(`On-chain error: ${JSON.stringify(status.err)}`);
    if (
      status?.confirmationStatus === "confirmed" ||
      status?.confirmationStatus === "finalized"
    ) {
      return status;
    }
    await new Promise((r) => setTimeout(r, 1000));
  }
  throw new Error(`Timed out confirming ${signature}`);
}

async function main() {
  const secret = Uint8Array.from(
    JSON.parse(fs.readFileSync(KEYPAIR_PATH, "utf8")),
  );
  const signer = await kit.createKeyPairSignerFromBytes(secret);
  const rpc = kit.createSolanaRpc(RPC);

  const txBytes = Buffer.from(TX_B64, "base64");
  const tx = kit.getTransactionDecoder().decode(txBytes);
  const signed = await kit.signTransaction([signer.keyPair], tx);

  const sim = await rpc
    .simulateTransaction(kit.getBase64EncodedWireTransaction(signed), {
      encoding: "base64",
      sigVerify: true,
    })
    .send();
  if (sim.value.err) {
    console.error("simulation failed:", sim.value.err);
    console.error((sim.value.logs || []).join("\n"));
    process.exit(1);
  }
  console.log("simulation ok");

  if (process.env.SIMULATE_ONLY === "1") return;

  const signature = await rpc
    .sendTransaction(kit.getBase64EncodedWireTransaction(signed), {
      encoding: "base64",
      preflightCommitment: "confirmed",
      skipPreflight: false,
    })
    .send();

  console.log("sent:", signature);
  const status = await confirmSignature(rpc, signature);
  console.log("confirmed:", status.confirmationStatus);
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
