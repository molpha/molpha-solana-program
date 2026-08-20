/* eslint-disable no-console */
/**
 * Local test-validator CU bench for `verify_attestation` and `submit_attestation`.
 *
 * Reports `unitsConsumed` via `Connection.simulateTransaction` against
 * `solana-test-validator` (default http://127.0.0.1:8899).
 *
 * Prereqs:
 *   - `yarn build:molpha` (or `anchor build`)
 *   - Local validator reachable at ANCHOR_PROVIDER_URL
 *
 * On first run the script restarts the local validator once per signer count
 * (3 / 8 / 18) to load bench account snapshots. Set CU_BENCH_NO_RESTART=1 to
 * require preloaded state instead.
 *
 * Manual bootstrap (restart validator yourself):
 *   node scripts/cu-bench-attestation-localnet.js --write-accounts
 *   solana-test-validator -r --bpf-program <MOLPHA_ID> target/deploy/molpha.so \
 *     --account-dir scripts/.cu-bench-localnet-accounts/n3
 *   # repeat for n8 / n18, or use CU_BENCH_BOOTSTRAP=1
 */
const anchor = require("@coral-xyz/anchor");
const {
  Keypair,
  SystemProgram,
  Transaction,
  ComputeBudgetProgram,
} = require("@solana/web3.js");
const { execSync, spawn } = require("child_process");
const fs = require("fs");
const path = require("path");
const os = require("os");

const {
  SIGNER_COUNTS,
  COMPUTE_UNIT_LIMIT,
  CU_CEILINGS,
  ACCOUNTS_ROOT,
  FIXTURE_VALUE,
  FIXTURE_CANONICAL_TIMESTAMP,
  FIXTURE_REGISTRY_VERSION,
  benchConfigHash,
  deriveFeedId,
  loadProgramId,
  loadIdl,
  nodeRemainingAccounts,
  NODE_COUNT,
  generateSignerMaterial,
  buildCoalitionSignature,
  buildSubmitArgs,
  writeAccountDir,
  findRegistryPda,
  findFeedPda,
  attachNodePdas,
  REGISTRY_STATE_SEED,
} = require("./lib/cu-bench-attestation-common");

const WRITE_ACCOUNTS_ONLY = process.argv.includes("--write-accounts");
const NO_RESTART = process.env.CU_BENCH_NO_RESTART === "1";

const SO_PATH = path.join(__dirname, "..", "target", "deploy", "molpha.so");
const RPC_URL = process.env.ANCHOR_PROVIDER_URL || "http://127.0.0.1:8899";

function sleep(ms) {
  return new Promise((r) => setTimeout(r, ms));
}

function loadPayerKeypair() {
  const walletPath =
    process.env.ANCHOR_WALLET ||
    path.join(os.homedir(), ".config", "solana", "id.json");
  const secretKey = Uint8Array.from(
    JSON.parse(fs.readFileSync(walletPath, "utf8"))
  );
  return Keypair.fromSecretKey(secretKey);
}

async function airdropIfNeeded(connection, pubkey, sol = 5) {
  const bal = await connection.getBalance(pubkey, "processed");
  const need = sol * 1e9;
  if (bal >= need) return;
  const sig = await connection.requestAirdrop(pubkey, need - bal);
  await connection.confirmTransaction(sig, "processed");
}

async function waitForRpc(url, attempts = 120) {
  const connection = new anchor.web3.Connection(url, "processed");
  for (let i = 0; i < attempts; i++) {
    try {
      await connection.getVersion();
      return connection;
    } catch (_e) {
      await sleep(500);
    }
  }
  throw new Error(`RPC not reachable at ${url}`);
}

function stopLocalValidator() {
  try {
    execSync('pkill -f "solana-test-validator" || true', { stdio: "ignore" });
  } catch (_e) {
    // ignore
  }
}

function startLocalValidator(programId, accountDir) {
  const logPath = path.join(ACCOUNTS_ROOT, "validator.log");
  fs.mkdirSync(ACCOUNTS_ROOT, { recursive: true });
  const logFd = fs.openSync(logPath, "a");
  const args = [
    "-r",
    "--quiet",
    "--bind-address",
    "127.0.0.1",
    "--rpc-port",
    "8899",
    "--bpf-program",
    programId.toBase58(),
    path.resolve(SO_PATH),
    "--account-dir",
    path.resolve(accountDir),
  ];
  console.log(
    "starting validator:",
    ["solana-test-validator", ...args].join(" ")
  );
  const child = spawn("solana-test-validator", args, {
    detached: true,
    stdio: ["ignore", logFd, logFd],
  });
  child.unref();
}

async function bootstrapValidatorForSignerCount(
  program,
  programId,
  payer,
  signerCount
) {
  const signerMaterial = generateSignerMaterial(NODE_COUNT);

  const accountDir = await writeAccountDir(
    program,
    programId,
    payer,
    signerCount,
    signerMaterial
  );

  console.log("bootstrapping validator", { signerCount, accountDir });
  stopLocalValidator();
  await sleep(2000);
  startLocalValidator(programId, accountDir);
  return waitForRpc(RPC_URL);
}

async function ensureProgramDeployed(connection, programId) {
  const info = await connection.getAccountInfo(programId, "processed");
  if (!info || !info.executable) {
    throw new Error(
      "molpha program not deployed on local validator — run `anchor deploy` or CU_BENCH_BOOTSTRAP=1"
    );
  }
}

async function benchAccountsPresent(connection, programId, payer, signerCount) {
  void payer;
  void signerCount;
  const registryPda = findRegistryPda(programId, FIXTURE_REGISTRY_VERSION);
  const info = await connection.getAccountInfo(registryPda, "processed");
  return info !== null && info.owner.equals(programId);
}

async function simulateIx(connection, payer, ix) {
  const { blockhash } = await connection.getLatestBlockhash("processed");
  const tx = new Transaction().add(
    ComputeBudgetProgram.setComputeUnitLimit({ units: COMPUTE_UNIT_LIMIT }),
    ix
  );
  tx.feePayer = payer.publicKey;
  tx.recentBlockhash = blockhash;
  tx.sign(payer);

  const sim = await connection.simulateTransaction(tx, {
    sigVerify: false,
    commitment: "processed",
  });
  if (sim.value.err) {
    throw new Error(
      `simulation failed: ${JSON.stringify(sim.value.err)}\nlogs:\n${(
        sim.value.logs || []
      ).join("\n")}`
    );
  }
  if (typeof sim.value.unitsConsumed !== "number") {
    throw new Error("simulateTransaction did not return unitsConsumed");
  }
  return sim.value.unitsConsumed;
}

async function measureForSignerCount(
  connection,
  program,
  programId,
  payer,
  signerCount
) {
  const configHash = benchConfigHash(signerCount);
  const sourceId = deriveFeedId(payer.publicKey, configHash, signerCount);

  const material = attachNodePdas(
    programId,
    generateSignerMaterial(NODE_COUNT)
  );

  const signatureMaterial = buildCoalitionSignature({
    signerCount,
    privKeys: material.privKeys,
    compressedPubkeys: material.compressedPubkeys,
    sourceId,
    registryVersion: FIXTURE_REGISTRY_VERSION,
    value: FIXTURE_VALUE,
    canonicalTimestamp: FIXTURE_CANONICAL_TIMESTAMP,
  });

  const [registryStatePda] = anchor.web3.PublicKey.findProgramAddressSync(
    [REGISTRY_STATE_SEED],
    programId
  );
  const registryPda = findRegistryPda(programId, FIXTURE_REGISTRY_VERSION);
  const feedPda = findFeedPda(
    programId,
    sourceId,
    signerCount,
    payer.publicKey
  );

  const args = buildSubmitArgs(signerCount, signatureMaterial, sourceId);
  const remainingAccounts = nodeRemainingAccounts(
    material.nodePdas,
    signatureMaterial.signersBitmap
  );

  const verifyIx = await program.methods
    .verifyAttestation(args)
    .accountsPartial({
      registry: registryPda,
    })
    .remainingAccounts(remainingAccounts)
    .instruction();

  const submitIx = await program.methods
    .submitAttestation(args)
    .accountsPartial({
      submitter: payer.publicKey,
      registryState: registryStatePda,
      registry: registryPda,
      feed: feedPda,
      systemProgram: SystemProgram.programId,
    })
    .remainingAccounts(remainingAccounts)
    .instruction();

  const verifyCu = await simulateIx(connection, payer, verifyIx);
  const submitCu = await simulateIx(connection, payer, submitIx);
  return { verifyCu, submitCu };
}

function makeProgram(idl, connection, payer) {
  const wallet = new anchor.Wallet(payer);
  const provider = new anchor.AnchorProvider(connection, wallet, {
    commitment: "processed",
  });
  return new anchor.Program(idl, provider);
}

function checkCuCeilings(out) {
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
    if (submitCu > submitCeil) {
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

async function main() {
  if (!fs.existsSync(SO_PATH)) {
    throw new Error(
      "target/deploy/molpha.so missing — run `yarn build:molpha` or `anchor build` first"
    );
  }

  const programId = loadProgramId();
  const payer = loadPayerKeypair();
  const { idl, programId: idlProgramId } = loadIdl();

  const providerUrl = RPC_URL;
  let connection = await waitForRpc(providerUrl).catch(() => null);

  if (WRITE_ACCOUNTS_ONLY) {
    const conn =
      connection || new anchor.web3.Connection(providerUrl, "processed");
    const program = makeProgram(idl, conn, payer);
    for (const n of SIGNER_COUNTS) {
      const signerMaterial = generateSignerMaterial(NODE_COUNT);
      const dir = await writeAccountDir(
        program,
        programId,
        payer,
        n,
        signerMaterial
      );
      console.log("wrote accounts", { signerCount: n, dir });
    }
    console.log(
      "Restart validator per signer count, e.g.:\n" +
        `  solana-test-validator -r --bpf-program ${programId.toBase58()} ${SO_PATH} ` +
        `--account-dir ${ACCOUNTS_ROOT}/n3`
    );
    return;
  }

  const out = {
    runtime: "solana-test-validator",
    rpc: providerUrl,
    signerCounts: SIGNER_COUNTS,
    verify_attestation: {},
    submit_attestation: {},
  };

  for (const n of SIGNER_COUNTS) {
    const present =
      connection &&
      (await benchAccountsPresent(connection, programId, payer, n));
    if (!present) {
      if (NO_RESTART && connection) {
        throw new Error(
          `Bench accounts for ${n} signers not on-chain. Remove CU_BENCH_NO_RESTART or restart validator with ${ACCOUNTS_ROOT}/n${n}`
        );
      }
      if (NO_RESTART && !connection) {
        throw new Error(
          `Cannot reach local validator at ${providerUrl}. Start \`solana-test-validator\` first`
        );
      }
      const bootstrapProgram = makeProgram(
        idl,
        connection || new anchor.web3.Connection(providerUrl, "processed"),
        payer
      );
      connection = await bootstrapValidatorForSignerCount(
        bootstrapProgram,
        programId,
        payer,
        n
      );
    } else {
      await ensureProgramDeployed(connection, programId);
    }

    const program = makeProgram(idl, connection, payer);
    if (!program.programId.equals(idlProgramId)) {
      throw new Error("IDL program id mismatch");
    }

    await airdropIfNeeded(connection, payer.publicKey, 2);

    const { verifyCu, submitCu } = await measureForSignerCount(
      connection,
      program,
      programId,
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

  console.log("attestation CU (localnet)", JSON.stringify(out, null, 2));
  checkCuCeilings(out);
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
