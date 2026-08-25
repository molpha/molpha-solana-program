/* eslint-disable no-console */
/**
 * Bankrun / LiteSVM CU bench for `verify_attestation` and `submit_attestation`.
 *
 * Reports `unitsConsumed` for signer counts 3 / 8 / 18 via BankrunProvider.simulate
 * (LiteSVM-backed), without a local validator. Uses v0 transactions + an Address
 * Lookup Table so large remaining-account sets fit under the 1232-byte limit.
 *
 * Prereq: `yarn build:molpha` so `target/deploy/molpha.so` matches current program.
 */
const anchor = require("@coral-xyz/anchor");
const {
  AddressLookupTableAccount,
  AddressLookupTableProgram,
  ComputeBudgetProgram,
  Keypair,
  PublicKey,
  SystemProgram,
  TransactionMessage,
  VersionedTransaction,
} = require("@solana/web3.js");
const { BankrunProvider, startAnchor } = require("anchor-bankrun");
const fs = require("fs");
const path = require("path");

const {
  NODE_COUNT,
  SIGNER_COUNTS,
  COMPUTE_UNIT_LIMIT,
  CU_CEILINGS,
  FIXTURE_SOURCE_ID,
  FIXTURE_VALUE,
  FIXTURE_REGISTRY_VERSION,
  FIXTURE_CANONICAL_TIMESTAMP,
  generateSignerMaterial,
  buildCoalitionSignature,
  buildSubmitArgs,
  nodeRemainingAccounts,
  buildBenchAccountSet,
  loadProgramId,
  loadIdl,
} = require("./lib/cu-bench-attestation-common");

const LAMPORTS = 10_000_000;
const U64_MAX = 0xffffffffffffffffn;

async function seedFromAccountSet(context, accountSet) {
  for (const { pubkey, data, owner } of accountSet.accounts) {
    context.setAccount(pubkey, {
      lamports: LAMPORTS,
      owner,
      executable: false,
      data: Buffer.from(data),
    });
  }
}

function lookupTableData(addresses) {
  const data = Buffer.alloc(56 + addresses.length * 32);
  data.writeUInt32LE(1, 0);
  data.writeBigUInt64LE(U64_MAX, 4);
  data.writeBigUInt64LE(0n, 12);
  addresses.forEach((address, index) =>
    address.toBuffer().copy(data, 56 + index * 32)
  );
  return data;
}

function collectLookupAddresses(ix) {
  const seen = new Set();
  const addresses = [];
  for (const meta of ix.keys) {
    if (meta.isSigner) continue;
    const key = meta.pubkey.toBase58();
    if (seen.has(key)) continue;
    seen.add(key);
    addresses.push(meta.pubkey);
  }
  return addresses;
}

function seedLookupTable(context, addresses) {
  const lookupKey = PublicKey.unique();
  const lookupTable = new AddressLookupTableAccount({
    key: lookupKey,
    state: {
      deactivationSlot: U64_MAX,
      lastExtendedSlot: 0,
      lastExtendedSlotStartIndex: 0,
      authority: undefined,
      addresses,
    },
  });
  context.setAccount(lookupKey, {
    lamports: LAMPORTS,
    owner: AddressLookupTableProgram.programId,
    executable: false,
    data: lookupTableData(addresses),
  });
  return lookupTable;
}

async function simulateV0(provider, payer, ix, lookupTable) {
  const [blockhash] = await provider.context.banksClient.getLatestBlockhash();
  const message = new TransactionMessage({
    payerKey: payer.publicKey,
    recentBlockhash: blockhash,
    instructions: [
      ComputeBudgetProgram.setComputeUnitLimit({ units: COMPUTE_UNIT_LIMIT }),
      ix,
    ],
  }).compileToV0Message([lookupTable]);
  const tx = new VersionedTransaction(message);
  tx.sign([payer]);

  const sim = await provider.context.banksClient.simulateTransaction(tx);
  if (sim.result !== null) {
    throw new Error(
      `simulation failed: ${sim.result}\nlogs:\n${(
        sim.meta?.logMessages || []
      ).join("\n")}`
    );
  }
  if (!sim.meta) {
    throw new Error("simulateTransaction returned no meta");
  }
  return Number(sim.meta.computeUnitsConsumed);
}

async function measureForSignerCount(provider, program, payer, signerCount) {
  const signerMaterial = generateSignerMaterial(NODE_COUNT);

  const signatureMaterial = buildCoalitionSignature({
    signerCount,
    privKeys: signerMaterial.privKeys,
    compressedPubkeys: signerMaterial.compressedPubkeys,
    sourceId: FIXTURE_SOURCE_ID,
    registryVersion: FIXTURE_REGISTRY_VERSION,
    value: FIXTURE_VALUE,
    canonicalTimestamp: FIXTURE_CANONICAL_TIMESTAMP,
  });

  const accountSet = await buildBenchAccountSet(
    program,
    program.programId,
    payer,
    signerCount,
    signerMaterial,
    {
      sourceId: FIXTURE_SOURCE_ID,
      registryVersion: FIXTURE_REGISTRY_VERSION,
    }
  );
  await seedFromAccountSet(provider.context, accountSet);

  const args = buildSubmitArgs(
    signerCount,
    signatureMaterial,
    FIXTURE_SOURCE_ID
  );
  const remainingAccounts = nodeRemainingAccounts(
    accountSet.nodePdas,
    signatureMaterial.signersBitmap
  );

  const verifyIx = await program.methods
    .verifyAttestation(args)
    .accountsPartial({
      registry: accountSet.registryPda,
    })
    .remainingAccounts(remainingAccounts)
    .instruction();

  const submitIx = await program.methods
    .submitAttestation(args)
    .accountsPartial({
      submitter: payer.publicKey,
      registryState: accountSet.registryStatePda,
      registry: accountSet.registryPda,
      feed: accountSet.feedPda,
      systemProgram: SystemProgram.programId,
    })
    .remainingAccounts(remainingAccounts)
    .instruction();

  const verifyLookup = seedLookupTable(
    provider.context,
    collectLookupAddresses(verifyIx)
  );
  const verifyCu = await simulateV0(provider, payer, verifyIx, verifyLookup);

  const submitLookup = seedLookupTable(
    provider.context,
    collectLookupAddresses(submitIx)
  );
  const submitCu = await simulateV0(provider, payer, submitIx, submitLookup);

  return { verifyCu, submitCu };
}

async function main() {
  const repoRoot = path.join(__dirname, "..");
  const soPath = path.join(repoRoot, "target", "deploy", "molpha.so");
  if (!fs.existsSync(soPath)) {
    throw new Error(
      "target/deploy/molpha.so missing — run `yarn build:molpha` first"
    );
  }

  const programId = loadProgramId();
  const payer = Keypair.generate();

  const context = await startAnchor(
    ".",
    [{ name: "molpha", programId }],
    [
      {
        address: payer.publicKey,
        info: {
          lamports: 1_000_000_000_000,
          executable: false,
          owner: SystemProgram.programId,
          data: Buffer.alloc(0),
        },
      },
    ],
    BigInt(COMPUTE_UNIT_LIMIT)
  );

  const provider = new BankrunProvider(context);
  provider.wallet = new anchor.Wallet(payer);
  anchor.setProvider(provider);

  const { idl, programId: idlProgramId } = loadIdl();
  const program = new anchor.Program(idl, provider);
  if (!program.programId.equals(idlProgramId)) {
    throw new Error("IDL program id mismatch");
  }

  const out = {
    runtime: "bankrun/litesvm",
    txFormat: "v0+alt",
    signerCounts: SIGNER_COUNTS,
    verify_attestation: {},
    submit_attestation: {},
  };

  for (const n of SIGNER_COUNTS) {
    const { verifyCu, submitCu } = await measureForSignerCount(
      provider,
      program,
      payer,
      n
    );
    out.verify_attestation[`cu${n}`] = verifyCu;
    out.submit_attestation[`cu${n}`] = submitCu;
    console.log("measured", {
      signerCount: n,
      verify_attestation: verifyCu,
      submit_attestation: submitCu,
    });
  }

  console.log("attestation CU (bankrun)", JSON.stringify(out, null, 2));

  const failures = [];
  for (const n of SIGNER_COUNTS) {
    const key = `cu${n}`;
    const verifyCu = out.verify_attestation[key];
    const submitCu = out.submit_attestation[key];
    const verifyCeil = CU_CEILINGS.verify_attestation[n];
    const submitCeil = CU_CEILINGS.submit_attestation[n];
    if (verifyCu > verifyCeil) {
      failures.push(
        `verify_attestation @ ${n} signers: ${verifyCu} CU > ceiling ${verifyCeil}`
      );
    }
    if (submitCu != null && submitCu > submitCeil) {
      failures.push(
        `submit_attestation @ ${n} signers: ${submitCu} CU > ceiling ${submitCeil}`
      );
    }
  }
  if (failures.length > 0) {
    console.error("CU ceiling check failed:\n" + failures.join("\n"));
    process.exit(1);
  }
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
