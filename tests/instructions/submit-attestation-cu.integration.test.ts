/**
 * Compute-unit and transaction-size benchmarks for `submit_attestation`.
 *
 * Measures CU at 3, 9, and 18 signers against a 256-node registry snapshot, and
 * discovers the largest signer coalition that fits in a legacy transaction.
 *
 * Run:
 *   yarn test:submit-attestation-cu
 *
 * Env:
 *   REGISTRY_NODE_COUNT — nodes to register (default 256)
 *   BENCH_SIGNER_COUNTS — comma-separated signer counts (default 3,9,18)
 */
import * as anchor from "@coral-xyz/anchor";
import { BN, Program } from "@coral-xyz/anchor";
import { assert } from "chai";
import {
  ComputeBudgetProgram,
  Keypair,
  PublicKey,
  SystemProgram,
  Transaction,
} from "@solana/web3.js";
import {
  ASSOCIATED_TOKEN_PROGRAM_ID,
  TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import { setupTestContext, TestContext } from "../setup";
import { buildInitializeArgs, rpcInitialize } from "../helpers/settlement-access";
import { generateE2eScenario } from "../helpers/e2e-fixture-export";
import {
  deriveSignersBitmap,
  findFeedPda,
  findRegistryPda,
  generateE2eSignerMaterialFromSeed,
  getProtocolTreasury,
  registerNodesForE2e,
  signE2eAttestation,
  signerRemainingAccounts,
} from "../helpers/submit-attestation";
import { bitsOfBitmap } from "../helpers/node-punishment";

const PROTOCOL_CONFIG_SEED = Buffer.from("molpha_config");
const REGISTRY_STATE_SEED = Buffer.from("molpha_registry");
const BENCH_SEED = "submit-attestation-cu-bench-v1";
const REDUNDANCY_BUFFER = 2;

const REGISTRY_NODE_COUNT = parseInt(
  process.env.REGISTRY_NODE_COUNT ?? "256",
  10
);
const BENCH_SIGNER_COUNTS = (process.env.BENCH_SIGNER_COUNTS ?? "3,9,18")
  .split(",")
  .map((s) => parseInt(s.trim(), 10))
  .filter((n) => Number.isFinite(n) && n > 0);

/** Solana legacy transaction packet size limit. */
const LEGACY_TX_SIZE_LIMIT = 1232;
/** Default per-transaction account lock cap on mainnet. */
const MAX_ACCOUNTS_PER_TX = 64;
const MAX_CU_PER_TX = 1_400_000;

type CuBenchRow = {
  signerCount: number;
  registryNodeCount: number;
  computeUnits: number;
  txSizeBytes: number;
  accountCount: number;
  fitsTxSize: boolean;
  fitsAccountLimit: boolean;
  ok: boolean;
};

function ensureNodeOwners(ctx: TestContext, count: number): void {
  while (ctx.nodes.length < count) {
    ctx.nodes.push(Keypair.generate());
  }
}

function parseSignerCounts(raw: string | undefined, fallback: number[]): number[] {
  if (!raw) {
    return fallback;
  }
  const parsed = raw
    .split(",")
    .map((s) => parseInt(s.trim(), 10))
    .filter((n) => Number.isFinite(n) && n > 0);
  return parsed.length > 0 ? parsed : fallback;
}

function countTransactionAccounts(tx: Transaction): number {
  const message = tx.compileMessage();
  return message.accountKeys.length;
}

function measureSerializedTx(tx: Transaction): {
  txSizeBytes: number;
  accountCount: number;
  fitsTxSize: boolean;
} {
  const accountCount = countTransactionAccounts(tx);
  try {
    const serialized = tx.serialize({
      requireAllSignatures: false,
      verifySignatures: false,
    });
    return {
      txSizeBytes: serialized.length,
      accountCount,
      fitsTxSize: serialized.length <= LEGACY_TX_SIZE_LIMIT,
    };
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    if (message.includes("Transaction too large")) {
      return {
        txSizeBytes: LEGACY_TX_SIZE_LIMIT + 1,
        accountCount,
        fitsTxSize: false,
      };
    }
    throw error;
  }
}

function bitsToBitmap(bits: number[]): Uint8Array {
  const bitmap = new Uint8Array(32);
  for (const bit of bits) {
    const byteIndex = 31 - (bit >> 3);
    bitmap[byteIndex] |= 1 << (bit & 7);
  }
  return bitmap;
}

/** Valid coalition of exactly `signerCount` nodes from the duty selection set. */
function signAttestationWithSignerCount(
  params: {
    registryVersion: number;
    registryNodeCount: number;
    signerMaterial: ReturnType<typeof generateE2eSignerMaterialFromSeed>;
    signerCount: number;
    sourceId: Uint8Array;
    value: Uint8Array;
    canonicalTimestamp: number;
  }
) {
  const selection = deriveSignersBitmap(
    params.sourceId,
    params.registryVersion,
    params.registryNodeCount,
    params.signerCount,
    REDUNDANCY_BUFFER,
    params.canonicalTimestamp
  );
  const selectionBits = bitsOfBitmap(selection);
  if (selectionBits.length < params.signerCount) {
    throw new Error(
      `selection size ${selectionBits.length} < signerCount ${params.signerCount}`
    );
  }
  const signersBitmap = bitsToBitmap(
    selectionBits.slice(0, params.signerCount)
  );
  return signE2eAttestation(
    params.registryVersion,
    params.signerMaterial,
    signersBitmap,
    params.sourceId,
    {
      value: params.value,
      canonicalTimestamp: params.canonicalTimestamp,
    },
    params.signerCount
  );
}

async function buildSubmitAttestationTx(
  ctx: TestContext,
  program: Program,
  params: {
    registryPda: PublicKey;
    registryVersion: number;
    registryNodeCount: number;
    signerMaterial: ReturnType<typeof generateE2eSignerMaterialFromSeed>;
    signerCount: number;
    sourceId: Uint8Array;
    canonicalTimestamp: number;
    computeUnitLimit?: number;
  }
): Promise<Transaction> {
  const signed = signAttestationWithSignerCount({
    registryVersion: params.registryVersion,
    registryNodeCount: params.registryNodeCount,
    signerMaterial: params.signerMaterial,
    signerCount: params.signerCount,
    sourceId: params.sourceId,
    value: generateE2eScenario(BENCH_SEED, params.signerCount).value,
    canonicalTimestamp: params.canonicalTimestamp,
  });

  const feedPda = findFeedPda(
    program.programId,
    params.sourceId,
    params.signerCount,
    ctx.authority.publicKey
  );

  const submitIx = await program.methods
    .submitAttestation(signed.submitArgs)
    .accountsPartial({
      submitter: ctx.authority.publicKey,
      registry: params.registryPda,
      feed: feedPda,
      systemProgram: SystemProgram.programId,
    })
    .remainingAccounts(
      signerRemainingAccounts(program.programId, signed.submitArgs.signersBitmap)
    )
    .instruction();

  const [blockhash] =
    await ctx.provider.context.banksClient.getLatestBlockhash();
  const tx = new Transaction().add(
    ComputeBudgetProgram.setComputeUnitLimit({
      units: params.computeUnitLimit ?? MAX_CU_PER_TX,
    }),
    submitIx
  );
  tx.feePayer = ctx.authority.publicKey;
  tx.recentBlockhash = blockhash;
  return tx;
}

function signSubmitAttestationTx(
  ctx: TestContext,
  tx: Transaction
): { ok: true } | { ok: false; reason: "tx_too_large" | "sign_failed" } {
  try {
    tx.sign(ctx.authority.payer);
    return { ok: true };
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    if (
      message.includes("Transaction too large") ||
      message.includes("encoding overruns Uint8Array")
    ) {
      return { ok: false, reason: "tx_too_large" };
    }
    return { ok: false, reason: "sign_failed" };
  }
}

async function measureSubmitAttestation(
  ctx: TestContext,
  program: Program,
  params: {
    registryPda: PublicKey;
    registryVersion: number;
    registryNodeCount: number;
    signerMaterial: ReturnType<typeof generateE2eSignerMaterialFromSeed>;
    signerCount: number;
    sourceId: Uint8Array;
    canonicalTimestamp: number;
  }
): Promise<CuBenchRow> {
  const tx = await buildSubmitAttestationTx(ctx, program, params);
  const signed = signSubmitAttestationTx(ctx, tx);
  if (!signed.ok) {
    throw new Error(
      `failed to sign submit_attestation tx for ${params.signerCount} signers: ${signed.reason}`
    );
  }
  const measured = measureSerializedTx(tx);
  const { txSizeBytes, accountCount, fitsTxSize } = measured;
  const fitsAccountLimit = accountCount <= MAX_ACCOUNTS_PER_TX;

  const result =
    await ctx.provider.context.banksClient.tryProcessTransaction(tx);
  if (result.result !== null) {
    const logs = result.meta?.logMessages?.join("\n") ?? "";
    throw new Error(
      `submit_attestation failed for ${params.signerCount} signers: ${result.result}\n${logs}`
    );
  }

  const computeUnits = Number(result.meta?.computeUnitsConsumed ?? 0n);
  return {
    signerCount: params.signerCount,
    registryNodeCount: params.registryVersion,
    computeUnits,
    txSizeBytes,
    accountCount,
    fitsTxSize,
    fitsAccountLimit,
    ok: true,
  };
}

async function txFitsOnChain(
  ctx: TestContext,
  program: Program,
  params: {
    registryPda: PublicKey;
    registryVersion: number;
    registryNodeCount: number;
    signerMaterial: ReturnType<typeof generateE2eSignerMaterialFromSeed>;
    signerCount: number;
    sourceId: Uint8Array;
    canonicalTimestamp: number;
  }
): Promise<{
  fitsTxSize: boolean;
  fitsAccountLimit: boolean;
  txSizeBytes: number;
  accountCount: number;
  ok: boolean;
  computeUnits: number;
}> {
  const tx = await buildSubmitAttestationTx(ctx, program, params);
  const accountCount = countTransactionAccounts(tx);
  const signed = signSubmitAttestationTx(ctx, tx);
  if (!signed.ok) {
    return {
      fitsTxSize: false,
      fitsAccountLimit: accountCount <= MAX_ACCOUNTS_PER_TX,
      txSizeBytes: LEGACY_TX_SIZE_LIMIT + 1,
      accountCount,
      ok: false,
      computeUnits: 0,
    };
  }

  const measured = measureSerializedTx(tx);
  const { txSizeBytes, fitsTxSize } = measured;
  const fitsAccountLimit = accountCount <= MAX_ACCOUNTS_PER_TX;

  if (!fitsTxSize || !fitsAccountLimit) {
    return {
      fitsTxSize,
      fitsAccountLimit,
      txSizeBytes,
      accountCount,
      ok: false,
      computeUnits: 0,
    };
  }

  const result =
    await ctx.provider.context.banksClient.tryProcessTransaction(tx);
  const computeUnits = Number(result.meta?.computeUnitsConsumed ?? 0n);
  return {
    fitsTxSize,
    fitsAccountLimit,
    txSizeBytes,
    accountCount,
    ok: result.result === null,
    computeUnits,
  };
}

async function findMaxSignersInTransaction(
  ctx: TestContext,
  program: Program,
  params: {
    registryPda: PublicKey;
    registryVersion: number;
    registryNodeCount: number;
    signerMaterial: ReturnType<typeof generateE2eSignerMaterialFromSeed>;
    sourceId: Uint8Array;
    canonicalTimestamp: number;
    upperBound: number;
  }
): Promise<{
  maxSigners: number;
  probe: CuBenchRow | null;
}> {
  let lo = 1;
  let hi = Math.min(params.upperBound, REGISTRY_NODE_COUNT);
  let best = 0;
  let bestRow: CuBenchRow | null = null;

  while (lo <= hi) {
    const mid = Math.floor((lo + hi) / 2);
    const probe = await txFitsOnChain(ctx, program, {
      ...params,
      signerCount: mid,
      canonicalTimestamp: params.canonicalTimestamp + mid,
    });

    if (probe.ok) {
      best = mid;
      bestRow = {
        signerCount: mid,
        registryNodeCount: params.registryVersion,
        computeUnits: probe.computeUnits,
        txSizeBytes: probe.txSizeBytes,
        accountCount: probe.accountCount,
        fitsTxSize: probe.fitsTxSize,
        fitsAccountLimit: probe.fitsAccountLimit,
        ok: true,
      };
      lo = mid + 1;
    } else {
      hi = mid - 1;
    }
  }

  return { maxSigners: best, probe: bestRow };
}

function printBenchTable(title: string, rows: CuBenchRow[]): void {
  // eslint-disable-next-line no-console
  console.log(`\n${title}`);
  // eslint-disable-next-line no-console
  console.table(
    rows.map((row) => ({
      signers: row.signerCount,
      registryNodes: row.registryNodeCount,
      computeUnits: row.computeUnits,
      txBytes: row.txSizeBytes,
      accounts: row.accountCount,
      txSizeOk: row.fitsTxSize,
      accountLimitOk: row.fitsAccountLimit,
    }))
  );
}

describe("submit_attestation CU benchmark", () => {
  jest.setTimeout(900_000);

  it("reports CU usage and max signers per transaction", async () => {
    const signerCounts = parseSignerCounts(
      process.env.BENCH_SIGNER_COUNTS,
      BENCH_SIGNER_COUNTS
    );

    const ctx = await setupTestContext();
    const program = ctx.molphaProgram as Program;
    ensureNodeOwners(ctx, REGISTRY_NODE_COUNT);

    const [protocolConfigPda] = PublicKey.findProgramAddressSync(
      [PROTOCOL_CONFIG_SEED],
      program.programId
    );
    const [registryStatePda] = PublicKey.findProgramAddressSync(
      [REGISTRY_STATE_SEED],
      program.programId
    );
    const treasury = getProtocolTreasury(
      protocolConfigPda,
      ctx.underlyingTokenMint
    );

    await rpcInitialize(
      program,
      {
        protocolConfig: protocolConfigPda,
        registryState: registryStatePda,
        usdcMint: ctx.underlyingTokenMint,
        treasury,
        authority: ctx.authority.publicKey,
      },
      ctx.underlyingTokenMint
    );

    const signerMaterial = generateE2eSignerMaterialFromSeed(
      BENCH_SEED,
      REGISTRY_NODE_COUNT
    );

    // eslint-disable-next-line no-console
    console.log(
      `Registering ${REGISTRY_NODE_COUNT} nodes (this may take several minutes)...`
    );
    await registerNodesForE2e(
      ctx,
      program,
      {
        protocolConfigPda,
        registryStatePda,
        usdcMint: ctx.underlyingTokenMint,
        ownerUsdc: ctx.userTokenAccount,
        treasury,
      },
      signerMaterial,
      async () => undefined
    );

    const registryState = await program.account.registryState.fetch(
      registryStatePda
    );
    assert.equal(registryState.currentVersion, REGISTRY_NODE_COUNT);

    const registryPda = findRegistryPda(
      program.programId,
      registryState.currentVersion
    );
    const registry = await program.account.registry.fetch(registryPda);
    assert.equal(registry.nodeCount, REGISTRY_NODE_COUNT);

    const scenario = generateE2eScenario(BENCH_SEED, 18);
    const baseTimestamp = scenario.canonicalTimestamp;

    const benchRows: CuBenchRow[] = [];
    for (const signerCount of signerCounts) {
      assert.isAtMost(
        signerCount,
        REGISTRY_NODE_COUNT,
        "signer count must fit in registry"
      );
      benchRows.push(
        await measureSubmitAttestation(ctx, program, {
          registryPda,
          registryVersion: registry.version,
          registryNodeCount: REGISTRY_NODE_COUNT,
          signerMaterial,
          signerCount,
          sourceId: scenario.sourceId,
          canonicalTimestamp: baseTimestamp + signerCount,
        })
      );
    }

    printBenchTable(
      `submit_attestation CU (${signerCounts.join(", ")} signers / ${REGISTRY_NODE_COUNT}-node registry)`,
      benchRows
    );

    const { maxSigners, probe } = await findMaxSignersInTransaction(ctx, program, {
      registryPda,
      registryVersion: registry.version,
      registryNodeCount: REGISTRY_NODE_COUNT,
      signerMaterial,
      sourceId: scenario.sourceId,
      canonicalTimestamp: baseTimestamp + 10_000,
      upperBound: REGISTRY_NODE_COUNT,
    });

    // eslint-disable-next-line no-console
    console.log("\nTransaction limits (legacy packet):");
    // eslint-disable-next-line no-console
    console.log(`  max serialized size: ${LEGACY_TX_SIZE_LIMIT} bytes`);
    // eslint-disable-next-line no-console
    console.log(`  max account locks:   ${MAX_ACCOUNTS_PER_TX}`);
    // eslint-disable-next-line no-console
    console.log(`  max compute units:   ${MAX_CU_PER_TX}`);
    // eslint-disable-next-line no-console
    console.log(`\nMax signers that fit in one submit_attestation tx: ${maxSigners}`);
    if (probe) {
      // eslint-disable-next-line no-console
      console.table([
        {
          signers: probe.signerCount,
          computeUnits: probe.computeUnits,
          txBytes: probe.txSizeBytes,
          accounts: probe.accountCount,
        },
      ]);
    }

    assert.isAtLeast(maxSigners, Math.max(...signerCounts));
    for (const row of benchRows) {
      assert.isAtMost(row.computeUnits, MAX_CU_PER_TX);
      assert.isTrue(row.fitsTxSize);
      assert.isTrue(row.fitsAccountLimit);
    }
  });
});
