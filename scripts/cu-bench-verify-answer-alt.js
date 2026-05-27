/* eslint-disable no-console */
const anchor = require("@coral-xyz/anchor");
const { PublicKey, Keypair, SystemProgram, ComputeBudgetProgram } = require("@solana/web3.js");
const {
  AddressLookupTableProgram,
  TransactionMessage,
  VersionedTransaction,
} = require("@solana/web3.js");
const {
  createMint,
  getOrCreateAssociatedTokenAccount,
  getAssociatedTokenAddressSync,
  mintTo,
  TOKEN_PROGRAM_ID,
  ASSOCIATED_TOKEN_PROGRAM_ID,
} = require("@solana/spl-token");
const secp256k1 = require("@noble/secp256k1");
const { keccak_256 } = require("@noble/hashes/sha3");
const { sha256 } = require("@noble/hashes/sha2");
const fs = require("fs");
const path = require("path");

// Must match `programs/molpha/src/instructions/verify_answer.rs`.
const EVM_ROUND_ID_PREFIX = Buffer.from([
  0x05, 0xcc, 0x9a, 0xbf, 0xa8, 0x3c, 0xdc, 0x2d, 0xff, 0xa2, 0x8a, 0xe0, 0xd4, 0x68,
  0x59, 0x5e, 0xce, 0xd0, 0x56, 0x4a, 0x16, 0xf6, 0xed, 0x9a, 0x3e, 0x2c, 0x5b, 0x4e,
  0x65, 0x2d, 0x98, 0x26,
]);

const SECP256K1_ORDER_N = BigInt(
  "0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141"
);

function u32be(x) {
  const b = Buffer.alloc(4);
  b.writeUInt32BE(x >>> 0, 0);
  return b;
}

function u64be(x) {
  const b = Buffer.alloc(8);
  b.writeBigUInt64BE(BigInt(x), 0);
  return b;
}

function bytes32FromAscii(s) {
  const out = Buffer.alloc(32);
  Buffer.from(s, "utf8").copy(out, 0, 0, Math.min(32, s.length));
  return out;
}

function toSeed(name) {
  return Buffer.from(sha256(Buffer.from(name, "utf-8")));
}

function evmUint256BitmapForFirstN(n) {
  // EVM uint256 layout: bit0 is LSB of byte31.
  const bm = Buffer.alloc(32, 0);
  for (let i = 0; i < n; i++) {
    const byteIndex = 31 - (i >> 3);
    const bitMask = 1 << (i & 7);
    bm[byteIndex] |= bitMask;
  }
  return bm;
}

function bigintFromBytesBE(b) {
  return BigInt("0x" + Buffer.from(b).toString("hex"));
}

function modN(x) {
  const r = x % SECP256K1_ORDER_N;
  return r >= 0n ? r : r + SECP256K1_ORDER_N;
}

function bytes32FromBigintBE(x) {
  let v = x;
  if (v < 0n) v = (v % SECP256K1_ORDER_N + SECP256K1_ORDER_N) % SECP256K1_ORDER_N;
  v %= SECP256K1_ORDER_N;
  const hex = v.toString(16).padStart(64, "0");
  return Buffer.from(hex, "hex");
}

function keccakConcat(chunks) {
  return Buffer.from(keccak_256(Buffer.concat(chunks)));
}

function commitmentAddrFromPoint(R) {
  const uncompressed = Buffer.from(R.toRawBytes(false)); // 65: 0x04 || x || y
  const digest = Buffer.from(keccak_256(uncompressed.subarray(1))); // keccak(x||y)
  return digest.subarray(12, 32); // last 20 bytes
}

function schnorrSignForVerifier({ coalitionPubkeyCompressed, messageHash, coalitionPrivkey }) {
  const px = coalitionPubkeyCompressed.subarray(1, 33);
  const recoveryId = coalitionPubkeyCompressed[0] & 1; // 0/1

  // Random nonce k in [1,n-1]
  let k = 0n;
  while (k === 0n) {
    k = modN(bigintFromBytesBE(secp256k1.utils.randomPrivateKey()));
  }
  const R = secp256k1.Point.BASE.multiply(k);
  const commitmentAddr = commitmentAddrFromPoint(R);

  // challenge = keccak256(px || recoveryId || msg_hash || commitment_addr) mod n
  const challengeHash = keccakConcat([
    Buffer.from(px),
    Buffer.from([recoveryId]),
    messageHash,
    commitmentAddr,
  ]);
  const e = modN(bigintFromBytesBE(challengeHash));

  // s = k + e*x mod n
  const s = modN(k + e * modN(coalitionPrivkey));
  return { s: bytes32FromBigintBE(s), commitmentAddr };
}

async function airdropIfNeeded(connection, pubkey, sol = 5) {
  const bal = await connection.getBalance(pubkey, "processed");
  const need = sol * 1e9;
  if (bal >= need) return;
  const sig = await connection.requestAirdrop(pubkey, need - bal);
  await connection.confirmTransaction(sig, "processed");
}

function loadIdl() {
  const idlPath = path.join(__dirname, "..", "target", "idl", "molpha.json");
  const raw = JSON.parse(fs.readFileSync(idlPath, "utf8"));
  // Anchor JS currently expects legacy account layout metadata (e.g. sizes/types).
  // This repo's IDL (spec 0.1.0) provides account discriminators only, which breaks
  // `new Program()` during Account namespace construction. We don't need account
  // clients for this CU bench, so we strip accounts to allow instruction building.
  const idl = { ...raw };
  delete idl.accounts;
  const programId = new PublicKey(raw.address);
  return { idl, programId };
}

async function main() {
  const providerUrl = process.env.ANCHOR_PROVIDER_URL || "http://127.0.0.1:8899";
  const walletPath =
    process.env.ANCHOR_WALLET ||
    path.join(require("os").homedir(), ".config", "solana", "id.json");
  const secretKey = Uint8Array.from(JSON.parse(fs.readFileSync(walletPath, "utf8")));
  const payerKeypair = Keypair.fromSecretKey(secretKey);
  const wallet = new anchor.Wallet(payerKeypair);
  const connection = new anchor.web3.Connection(providerUrl, {
    commitment: "processed",
    confirmTransactionInitialTimeout: 60_000,
  });
  const provider = new anchor.AnchorProvider(connection, wallet, {
    commitment: "processed",
  });
  anchor.setProvider(provider);
  const { idl, programId } = loadIdl();
  // Anchor 0.32 `Program` constructor signature is `(idl, provider?, coder?, getCustomResolver?)`.
  // Program id is taken from `idl.address`.
  const program = new anchor.Program(idl, provider);
  if (!program.programId.equals(programId)) {
    throw new Error(
      `IDL program id mismatch: idl=${program.programId.toBase58()} expected=${programId.toBase58()}`
    );
  }

  const payer = provider.wallet.payer ?? provider.wallet;
  await airdropIfNeeded(provider.connection, payer.publicKey, 10);

  // --- Create mint + token accounts (real SPL, mainnet-like) ---
  const mint = await createMint(
    provider.connection,
    payer,
    payer.publicKey,
    null,
    6 // decimals
  );

  const [protocolConfigPda] = PublicKey.findProgramAddressSync(
    [Buffer.from("config")],
    programId
  );

  // This bench assumes a fresh local validator (`solana-test-validator --reset`)
  // so node indexes start from 1 and the signer bitmap (first N nodes) matches.
  const existingCfg = await provider.connection.getAccountInfo(protocolConfigPda, "processed");
  if (existingCfg) {
    throw new Error(
      "protocol_config already exists. Please restart your local validator with `solana-test-validator --reset`, then `anchor deploy`, then rerun."
    );
  }

  const payerAta = await getOrCreateAssociatedTokenAccount(
    provider.connection,
    payer,
    mint,
    payer.publicKey
  );

  const programTokenAccount = getAssociatedTokenAddressSync(
    mint,
    protocolConfigPda,
    true
  );

  // Fund payer with tokens for deposits/subscriptions.
  await mintTo(
    provider.connection,
    payer,
    mint,
    payerAta.address,
    payer.publicKey,
    100_000_000_000n // 100_000 tokens (6 decimals) to cover 32x deposits + feed costs
  );

  // --- Initialize protocol (idempotent-ish) ---
  let initErr = null;
  try {
    await program.methods
      .initialize()
      .accountsPartial({
        protocolConfig: protocolConfigPda,
        underlyingToken: mint,
        programTokenAccount,
        authority: payer.publicKey,
        systemProgram: SystemProgram.programId,
        tokenProgram: TOKEN_PROGRAM_ID,
        associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
      })
      .rpc({ commitment: "processed" });
  } catch (e) {
    initErr = e;
  }
  const cfgInfo = await provider.connection.getAccountInfo(protocolConfigPda, "processed");
  if (!cfgInfo) {
    throw new Error(`protocol_config not initialized; initialize() failed: ${initErr?.message ?? initErr}`);
  }

  // Ensure the program's token ATA exists (create_feed requires it initialized).
  const programAta = await getOrCreateAssociatedTokenAccount(
    provider.connection,
    payer,
    mint,
    protocolConfigPda,
    true // allowOwnerOffCurve
  );
  if (!programAta.address.equals(programTokenAccount)) {
    throw new Error("unexpected program token ATA address mismatch");
  }

  // Set a deposit so register_node can transfer.
  await program.methods
    .updateProtocolConfig({
      basePricePerSecondScaled: null,
      frequencyCoefficient: null,
      signersCoefficient: null,
      rewardPercentage: null,
      nodeSecurityDeposit: new anchor.BN(1_000_000_000), // 1000 tokens (6 decimals)
      registryXAgg: null,
      signaturesRequired: null,
      nodeCount: null,
      redundancyBuffer: null,
    })
    .accountsPartial({
      protocolConfig: protocolConfigPda,
      authority: payer.publicKey,
    })
    .rpc({ commitment: "processed" });

  // --- Register 32 nodes once (pubkeys are random; no need for node operator to sign) ---
  const nodePdas = [];
  const privKeys = [];
  for (let i = 0; i < 32; i++) {
    const skBytes = secp256k1.utils.randomPrivateKey();
    const sk = modN(bigintFromBytesBE(skBytes));
    privKeys.push(sk);
    const pkCompressed = Buffer.from(secp256k1.getPublicKey(skBytes, true)); // 33

    const nodePubkey = Keypair.generate().publicKey;
    const [nodePda] = PublicKey.findProgramAddressSync(
      [Buffer.from("node"), nodePubkey.toBuffer()],
      programId
    );
    const nodeSecurityDepositAccount = getAssociatedTokenAddressSync(mint, nodePda, true);

    try {
      await program.methods
        .registerNode(nodePubkey, pkCompressed, "https://example.com/node")
        .accountsPartial({
          node: nodePda,
          protocolConfig: protocolConfigPda,
          underlyingToken: mint,
          nodeOperatorTokenAccount: payerAta.address,
          nodeSecurityDepositAccount,
          authority: payer.publicKey,
          systemProgram: SystemProgram.programId,
          tokenProgram: TOKEN_PROGRAM_ID,
          associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
        })
        .preInstructions([
          ComputeBudgetProgram.setComputeUnitLimit({ units: 400_000 }),
        ])
        .rpc({ commitment: "processed" });
    } catch (_e) {
      // ok if already exists (re-runs)
    }

    nodePdas.push(nodePda);
  }

  const registryVersion = 1;
  const configHash = bytes32FromAscii("cu-config-alt-real");
  const valueBytes32 = bytes32FromAscii("cu-value-alt-real");
  const timestamp = 1_700_001_234n;

  async function measureForSignerCount(signerCount) {
    // Create a dedicated data source + feed with threshold == signerCount.
    const dataSourceName = `cu-alt-real-ds-${signerCount}-${Date.now()}`;
    const [dataSourcePda] = PublicKey.findProgramAddressSync(
      [Buffer.from("data_source"), payer.publicKey.toBuffer(), toSeed(dataSourceName), Buffer.from([0])],
      programId
    );
    try {
      await program.methods
        .createDataSource({
          dataSourceType: { public: {} },
          metadataHash: Array.from(Buffer.alloc(32, 1)),
          name: dataSourceName,
        })
        .accountsPartial({
          authority: payer.publicKey,
          dataSource: dataSourcePda,
          systemProgram: SystemProgram.programId,
        })
        .rpc({ commitment: "processed" });
    } catch (_e) {}

    const feedName = `cu-alt-real-feed-${signerCount}-${Date.now()}`;
    const jobId = bytes32FromAscii(`cu-alt-real-job-${signerCount}`);
    const [feedPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("feed"), payer.publicKey.toBuffer(), toSeed(feedName), Buffer.from([0])],
      programId
    );

    await program.methods
      .createFeed(
        {
          name: feedName,
          jobId: Array.from(jobId),
          feedType: { public: {} },
          minSignaturesThreshold: signerCount,
          frequency: new anchor.BN(300),
          ipfsCid: "QmCuBenchAlt",
        },
        new anchor.BN(86400)
      )
      .accountsPartial({
        feed: feedPda,
        dataSource: dataSourcePda,
        authority: payer.publicKey,
        protocolConfig: protocolConfigPda,
        userTokenAccount: payerAta.address,
        programTokenAccount,
        underlyingToken: mint,
        systemProgram: SystemProgram.programId,
        tokenProgram: TOKEN_PROGRAM_ID,
        associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
      })
      .rpc({ commitment: "processed" });

    // Ensure deterministic group bitmap: group_size == node_count (full set).
    await program.methods
      .updateProtocolConfig({
        basePricePerSecondScaled: null,
        frequencyCoefficient: null,
        signersCoefficient: null,
        rewardPercentage: null,
        nodeSecurityDeposit: null,
        registryXAgg: null,
        signaturesRequired: signerCount,
        // For deterministic group bitmap derivation we want group_size == node_count.
        // This bench assumes a reset local validator where we just registered exactly 32 nodes.
        nodeCount: 32,
        redundancyBuffer: 32 - signerCount,
      })
      .accountsPartial({
        protocolConfig: protocolConfigPda,
        authority: payer.publicKey,
      })
      .rpc({ commitment: "processed" });

    const signersBitmap = evmUint256BitmapForFirstN(signerCount);

    // Coalition pubkey is the sum of first `signerCount` node pubkeys; coalition privkey is sum of privkeys (mod n).
    let coalitionPoint = secp256k1.Point.ZERO;
    let coalitionSk = 0n;
    for (let i = 0; i < signerCount; i++) {
      coalitionSk = modN(coalitionSk + privKeys[i]);
      coalitionPoint = coalitionPoint.add(secp256k1.Point.BASE.multiply(privKeys[i]));
    }
    const coalitionCompressed = Buffer.from(coalitionPoint.toRawBytes(true)); // 33

    const messageHash = keccakConcat([
      EVM_ROUND_ID_PREFIX,
      jobId,
      u32be(registryVersion),
      u32be(signerCount),
      signersBitmap,
      valueBytes32,
      u64be(timestamp),
    ]);

    const { s, commitmentAddr } = schnorrSignForVerifier({
      coalitionPubkeyCompressed: coalitionCompressed,
      messageHash,
      coalitionPrivkey: coalitionSk,
    });

    const args = {
      jobId: Array.from(jobId),
      value: new anchor.BN(valueBytes32.readBigInt64BE(0).toString()),
      valueBytes32: Array.from(valueBytes32),
      timestamp: new anchor.BN(timestamp.toString()),
      round: 0,
      signersBitmap: Array.from(signersBitmap),
      s: Array.from(s),
      commitmentAddr: Array.from(commitmentAddr),
      registryVersion,
      configHash: Array.from(configHash),
      maxStaleness: 0,
    };

    const signerNodePdas = nodePdas.slice(0, signerCount);
    const remainingAccounts = signerNodePdas.map((pubkey) => ({
      pubkey,
      isSigner: false,
      isWritable: false,
    }));

    // --- Create & extend a REAL ALT on the validator ---
    const recentSlot = await provider.connection.getSlot("processed");
    const [createLutIx, lutAddress] = AddressLookupTableProgram.createLookupTable({
      authority: payer.publicKey,
      payer: payer.publicKey,
      recentSlot,
    });
    const extendLutIx = AddressLookupTableProgram.extendLookupTable({
      authority: payer.publicKey,
      payer: payer.publicKey,
      lookupTable: lutAddress,
      addresses: signerNodePdas,
    });

    {
      const blockhash = await provider.connection.getLatestBlockhash("processed");
      const tx = new anchor.web3.Transaction().add(createLutIx, extendLutIx);
      tx.feePayer = payer.publicKey;
      tx.recentBlockhash = blockhash.blockhash;
      tx.sign(payer);
      const sig = await provider.connection.sendRawTransaction(tx.serialize(), {
        skipPreflight: true,
      });
      await provider.connection.confirmTransaction(sig, "processed");
    }

    // After extending, the LUT can be \"recently extended\" in the current slot,
    // where only a prefix of addresses is active. Wait for the next slot so all
    // appended addresses are usable in v0 transactions.
    let lutResp = await provider.connection.getAddressLookupTable(lutAddress);
    if (!lutResp.value) throw new Error("failed to fetch lookup table account");
    const lastExtendedSlot = lutResp.value.state.lastExtendedSlot;
    for (let i = 0; i < 50; i++) {
      const cur = await provider.connection.getSlot("processed");
      if (cur > lastExtendedSlot) break;
      await new Promise((r) => setTimeout(r, 200));
    }
    lutResp = await provider.connection.getAddressLookupTable(lutAddress);
    if (!lutResp.value) throw new Error("failed to fetch lookup table account (after wait)");

    const verifyIx = await program.methods
      .verifyAnswer(args)
      .accountsPartial({
        protocolConfig: protocolConfigPda,
        feed: feedPda,
      })
      .remainingAccounts(remainingAccounts)
      .instruction();

    const { blockhash } = await provider.connection.getLatestBlockhash("processed");
    const v0Message = new TransactionMessage({
      payerKey: payer.publicKey,
      recentBlockhash: blockhash,
      instructions: [
        ComputeBudgetProgram.setComputeUnitLimit({ units: 1_400_000 }),
        verifyIx,
      ],
    }).compileToV0Message([lutResp.value]);

    const vtx = new VersionedTransaction(v0Message);
    vtx.sign([payer]);

    const sim = await provider.connection.simulateTransaction(vtx, {
      sigVerify: false,
      commitment: "processed",
    });
    if (sim.value.err) {
      throw new Error(
        `simulation failed: ${JSON.stringify(sim.value.err)}\nlogs:\n${(sim.value.logs || []).join(
          "\n"
        )}`
      );
    }
    if (typeof sim.value.unitsConsumed !== "number") {
      throw new Error(`simulateTransaction did not return unitsConsumed`);
    }
    return sim.value.unitsConsumed;
  }

  const signerCounts = [3, 7, 18, 32];
  const out = {};
  for (const n of signerCounts) {
    const cu = await measureForSignerCount(n);
    out[`cu${n}`] = cu;
    console.log("measured", { signerCount: n, cu });
  }

  console.log("verify_answer_alt REALISTIC CU", out);
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});

