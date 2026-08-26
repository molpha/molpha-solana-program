/* eslint-disable no-console */
/**
 * Shared helpers for settle-round CU benches (bankrun).
 */
const anchor = require("@coral-xyz/anchor");
const {
  AddressLookupTableAccount,
  AddressLookupTableProgram,
  ComputeBudgetProgram,
  Ed25519Program,
  Keypair,
  PublicKey,
  SystemProgram,
  SYSVAR_INSTRUCTIONS_PUBKEY,
  TransactionMessage,
  VersionedTransaction,
} = require("@solana/web3.js");
const {
  ASSOCIATED_TOKEN_PROGRAM_ID,
  TOKEN_PROGRAM_ID,
  getAssociatedTokenAddressSync,
} = require("@solana/spl-token");
const crypto = require("crypto");
const { keccak_256 } = require("@noble/hashes/sha3");
const { signNodePop, findSecp256k1KeyClaimPda } = require("./node-pop");
const {
  PROTOCOL_CONFIG_SEED,
  REGISTRY_STATE_SEED,
  REGISTRY_SEED,
  NODE_SEED,
  loadIdl,
  loadProgramId,
  findRegistryPda,
  findNodePda,
  generateSignerMaterial,
  deriveSignersBitmap,
  u32le,
} = require("./cu-bench-attestation-common");

const PLAN_SEED = Buffer.from("molpha_plan");
const SUBSCRIPTION_SEED = Buffer.from("molpha_subscription");
const GATEWAY_SEED = Buffer.from("molpha_gateway");
const ROUND_SEED = Buffer.from("molpha_round");
const X402_ROUND_SEED = Buffer.from("molpha_x402_round");
const NODE_ID_PREFIX = Buffer.from("MOLPHA_NODE_ID_V1");

/** Avoid `111...112` — it breaks v0 ALT account resolution in bankrun. */
const MOCK_USDC_MINT = Keypair.fromSeed(
  crypto.createHash("sha256").update("molpha-settle-cu-bench-mint").digest()
).publicKey;
const U64_MAX = 0xffffffffffffffffn;
const LAMPORTS = 10_000_000;
const ACCOUNT_LAMPORTS = 1_000_000_000;

const NODE_COUNT = 32;
const SIGNER_COUNTS = [1, 3, 8, 18];
const COMPUTE_UNIT_LIMIT = 1_400_000;

/** Soft ceilings (~25% over bankrun baseline). */
const CU_CEILINGS = {
  settle_subscription_round: {
    1: 33_000,
    3: 32_000,
    8: 36_000,
    18: 40_000,
  },
  settle_x402_round: {
    1: 40_000,
    3: 44_000,
    8: 41_000,
    18: 45_000,
  },
};

const INIT = {
  rewardPerSignature: 10,
  gatewayFee: 20,
  disputeWindowSlots: 256,
  gatewayBondMin: 500_000_000,
  redundancyBuffer: 2,
  challengerBountyBps: 500,
  x402RoundBase: 1_000,
  x402RoundPerSig: 15,
  minimumNodeDeposit: 100_000_000,
  subscriptionPrice: 2_000_000,
  planMaxSigners: 32,
  planMaxDelegates: 4,
};

function u64le(value) {
  const out = Buffer.alloc(8);
  out.writeBigUInt64LE(BigInt(value), 0);
  return out;
}

function u8(value) {
  return Buffer.from([value & 0xff]);
}

function sha256(parts) {
  const h = crypto.createHash("sha256");
  for (const part of parts) h.update(part);
  return h.digest();
}

function hashRequestAuthFields(sourceId, signaturesRequired, timestamp) {
  const body = Buffer.concat([
    Buffer.from(sourceId),
    u8(signaturesRequired),
    u64le(timestamp),
  ]);
  return sha256([Buffer.from("MOLPHA_REQAUTH_V1", "utf8"), body]);
}

function deriveSettleSignersBitmap({
  sourceId,
  registryVersion,
  nodeCount,
  signaturesRequired,
  timestamp,
  redundancyBuffer = INIT.redundancyBuffer,
}) {
  return deriveSignersBitmap(
    sourceId,
    registryVersion,
    nodeCount,
    signaturesRequired,
    redundancyBuffer,
    timestamp
  );
}

function zeroedAggSig(signersBitmap) {
  return {
    s: Array.from(new Uint8Array(32)),
    rx: Array.from(new Uint8Array(32)),
    ryParity: 0,
    commitment: Array.from(new Uint8Array(20)),
    signersBitmap: Array.from(signersBitmap ?? new Uint8Array(32)),
  };
}

function x402Price(signaturesRequired) {
  return INIT.x402RoundBase + signaturesRequired * INIT.x402RoundPerSig;
}

function deriveNodeId(owner, secpCompressed, registrationSalt) {
  return Buffer.from(
    keccak_256(
      Buffer.concat([
        NODE_ID_PREFIX,
        owner.toBuffer(),
        Buffer.from(secpCompressed),
        Buffer.from(registrationSalt),
      ])
    )
  );
}

function derivePayerId(railId, addressBytes) {
  const railIdBe = Buffer.alloc(2);
  railIdBe.writeUInt16BE(railId, 0);
  return Buffer.from(keccak_256(Buffer.concat([railIdBe, Buffer.from(addressBytes)])));
}

function findGatewayPda(programId, authority) {
  return PublicKey.findProgramAddressSync(
    [GATEWAY_SEED, authority.toBuffer()],
    programId
  )[0];
}

function findGatewayBondAta(usdcMint, gatewayPda) {
  return getAssociatedTokenAddressSync(usdcMint, gatewayPda, true);
}

function findSubscriptionRoundPda(
  programId,
  subscriptionOwner,
  sourceId,
  signaturesRequired,
  timestamp
) {
  return PublicKey.findProgramAddressSync(
    [
      ROUND_SEED,
      subscriptionOwner.toBuffer(),
      Buffer.from(sourceId),
      u8(signaturesRequired),
      u64le(timestamp),
    ],
    programId
  )[0];
}

function findX402RoundPda(
  programId,
  payerId,
  gateway,
  sourceId,
  signaturesRequired,
  canonicalTimestamp
) {
  return PublicKey.findProgramAddressSync(
    [
      X402_ROUND_SEED,
      Buffer.from(payerId),
      gateway.toBuffer(),
      Buffer.from(sourceId),
      u8(signaturesRequired),
      u64le(canonicalTimestamp),
    ],
    programId
  )[0];
}

function getProtocolTreasury(protocolConfigPda, usdcMint) {
  return getAssociatedTokenAddressSync(usdcMint, protocolConfigPda, true);
}

function mintAccountData(mintAuthority, supply = 1_000_000_000_000n) {
  const data = Buffer.alloc(82);
  data.writeUInt32LE(1, 0);
  mintAuthority.toBuffer().copy(data, 4);
  data.writeBigUInt64LE(supply, 36);
  data.writeUInt8(6, 44);
  data.writeUInt8(1, 45);
  data.writeUInt32LE(0, 46);
  return data;
}

function tokenAccountData(mint, owner, amount) {
  const data = Buffer.alloc(165);
  mint.toBuffer().copy(data, 0);
  owner.toBuffer().copy(data, 32);
  data.writeBigUInt64LE(BigInt(amount), 64);
  data.writeUInt32LE(0, 72);
  data.writeUInt8(1, 108);
  data.writeUInt32LE(0, 109);
  data.writeBigUInt64LE(0n, 113);
  data.writeUInt32LE(0, 121);
  return data;
}

function fundAtaInBankrun(context, ata, usdcMint, owner, amount) {
  context.setAccount(ata, {
    lamports: 2_039_280,
    owner: TOKEN_PROGRAM_ID,
    executable: false,
    data: tokenAccountData(usdcMint, owner, amount),
  });
}

function fundOwnerInBankrun(context, owner, lamports = ACCOUNT_LAMPORTS) {
  context.setAccount(owner, {
    lamports,
    owner: SystemProgram.programId,
    executable: false,
    data: Buffer.alloc(0),
  });
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

const STATIC_PROGRAM_IDS = new Set(
  [
    SystemProgram.programId,
    TOKEN_PROGRAM_ID,
    ASSOCIATED_TOKEN_PROGRAM_ID,
    SYSVAR_INSTRUCTIONS_PUBKEY,
    AddressLookupTableProgram.programId,
    Ed25519Program.programId,
    ComputeBudgetProgram.programId,
  ].map((id) => id.toBase58())
);

function collectLookupAddresses(ix) {
  const seen = new Set();
  const addresses = [];
  for (const meta of ix.keys) {
    if (meta.isSigner) continue;
    const key = meta.pubkey.toBase58();
    if (seen.has(key) || STATIC_PROGRAM_IDS.has(key)) continue;
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

function remainingAccountsForBitmap(bitmap, nodePdasByIndex, writable = false) {
  const remaining = [];
  for (let bit = 0; bit < 256; bit++) {
    const byteIndex = 31 - (bit >> 3);
    const bitMask = 1 << (bit & 7);
    if ((bitmap[byteIndex] & bitMask) === 0) continue;
    const nodePda = nodePdasByIndex.get(bit);
    if (!nodePda) throw new Error(`Missing registered node for bit ${bit}`);
    remaining.push({
      pubkey: nodePda,
      isSigner: false,
      isWritable: writable,
    });
  }
  return remaining;
}

async function simulateV0(provider, signers, instructions, lookupTable) {
  const [blockhash] = await provider.context.banksClient.getLatestBlockhash();
  const message = new TransactionMessage({
    payerKey: signers[0].publicKey,
    recentBlockhash: blockhash,
    instructions: [
      ComputeBudgetProgram.setComputeUnitLimit({ units: COMPUTE_UNIT_LIMIT }),
      ...instructions,
    ],
  }).compileToV0Message([lookupTable]);
  const tx = new VersionedTransaction(message);
  tx.sign(signers);

  const sim = await provider.context.banksClient.simulateTransaction(tx);
  if (sim.result !== null) {
    throw new Error(
      `simulation failed: ${sim.result}\nlogs:\n${(sim.meta?.logMessages || []).join("\n")}`
    );
  }
  if (!sim.meta) throw new Error("simulateTransaction returned no meta");
  return Number(sim.meta.computeUnitsConsumed);
}

function buildBankrunPrefundedAccounts(payer, usdcMint) {
  const userTokenAccount = getAssociatedTokenAddressSync(
    usdcMint,
    payer.publicKey
  );
  return [
    {
      address: payer.publicKey,
      info: {
        lamports: 1_000_000_000_000,
        executable: false,
        owner: SystemProgram.programId,
        data: Buffer.alloc(0),
      },
    },
    {
      address: usdcMint,
      info: {
        lamports: 1_000_000_000,
        executable: false,
        owner: TOKEN_PROGRAM_ID,
        data: mintAccountData(payer.publicKey),
      },
    },
    {
      address: userTokenAccount,
      info: {
        lamports: 2_039_280,
        executable: false,
        owner: TOKEN_PROGRAM_ID,
        data: tokenAccountData(usdcMint, payer.publicKey, 500_000_000_000),
      },
    },
  ];
}

function bytes32FromBigintBE(value) {
  const SECP256K1_ORDER_N = BigInt(
    "0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141"
  );
  let scalar = value % SECP256K1_ORDER_N;
  if (scalar < 0n) scalar += SECP256K1_ORDER_N;
  return Buffer.from(scalar.toString(16).padStart(64, "0"), "hex");
}

async function registerNodesForBench(
  provider,
  program,
  payer,
  accounts,
  signerMaterial
) {
  const nodePdasByIndex = new Map();
  for (let index = 0; index < signerMaterial.compressedPubkeys.length; index++) {
    const nodeOwner = signerMaterial.owners[index];
    const secpCompressed = signerMaterial.compressedPubkeys[index];
    const ownerUsdc = getAssociatedTokenAddressSync(
      accounts.usdcMint,
      nodeOwner.publicKey
    );
    fundOwnerInBankrun(provider.context, nodeOwner.publicKey);
    fundAtaInBankrun(
      provider.context,
      ownerUsdc,
      accounts.usdcMint,
      nodeOwner.publicKey,
      1_000_000_000
    );

    const registrationSalt = Buffer.alloc(32, index + 1);
    const nodeId = deriveNodeId(
      nodeOwner.publicKey,
      secpCompressed,
      registrationSalt
    );
    const pop = signNodePop({
      programId: program.programId,
      nodeId,
      secp256k1PrivateKey: bytes32FromBigintBE(signerMaterial.privKeys[index]),
    });
    const nodePda = findNodePda(program.programId, nodeOwner.publicKey);
    nodePdasByIndex.set(index, nodePda);

    await program.methods
      .registerNode({
        newRegistryVersion: index + 1,
        registrationSalt: Array.from(registrationSalt),
        ip: [127, 0, 0, 1],
        port: 9_000 + index,
        secp256K1PubkeyCompressed: Array.from(secpCompressed),
        secp256K1KeyFingerprint: Array.from(keccak_256(secpCompressed)),
        popSigR: Array.from(pop.popSigR),
        popSigS: Array.from(pop.popSigS),
      })
      .accountsPartial({
        authority: payer.publicKey,
        nodeOwner: nodeOwner.publicKey,
        protocolConfig: accounts.protocolConfigPda,
        registryState: accounts.registryStatePda,
        currentRegistry: findRegistryPda(program.programId, index),
        newRegistry: findRegistryPda(program.programId, index + 1),
        node: nodePda,
        secp256K1KeyClaim: findSecp256k1KeyClaimPda(
          program.programId,
          secpCompressed
        ),
        usdcMint: accounts.usdcMint,
        ownerUsdc,
        treasury: accounts.treasury,
        systemProgram: SystemProgram.programId,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .preInstructions([
        ComputeBudgetProgram.setComputeUnitLimit({ units: 450_000 }),
      ])
      .signers([nodeOwner])
      .rpc();
  }
  return nodePdasByIndex;
}

async function bootstrapSettleBenchState(provider, program, payer, nodeCount) {
  const programId = program.programId;
  const usdcMint = MOCK_USDC_MINT;
  const [protocolConfigPda] = PublicKey.findProgramAddressSync(
    [PROTOCOL_CONFIG_SEED],
    programId
  );
  const [registryStatePda] = PublicKey.findProgramAddressSync(
    [REGISTRY_STATE_SEED],
    programId
  );
  const treasury = getProtocolTreasury(protocolConfigPda, usdcMint);
  const genesisRegistry = findRegistryPda(programId, 0);
  const ownerUsdc = getAssociatedTokenAddressSync(usdcMint, payer.publicKey);

  await program.methods
    .initialize({
      withdrawalCooldownSecs: new anchor.BN(5),
      maxNodeCount: nodeCount + 4,
      redundancyBuffer: INIT.redundancyBuffer,
      minimumNodeDeposit: new anchor.BN(INIT.minimumNodeDeposit),
      rewardPerSignature: new anchor.BN(INIT.rewardPerSignature),
      gatewayFee: new anchor.BN(INIT.gatewayFee),
      receiptSubmitterBonus: new anchor.BN(5),
      protocolFeeBps: 100,
      maxReceiptDelaySlots: new anchor.BN(256),
      protocolFee: new anchor.BN(0),
      disputeWindowSlots: new anchor.BN(INIT.disputeWindowSlots),
      gatewayBondMin: new anchor.BN(INIT.gatewayBondMin),
      challengerBountyBps: INIT.challengerBountyBps,
      usdcMint,
      x402RoundBase: new anchor.BN(INIT.x402RoundBase),
      x402RoundPerSig: new anchor.BN(INIT.x402RoundPerSig),
    })
    .accountsPartial({
      protocolConfig: protocolConfigPda,
      registryState: registryStatePda,
      registry: genesisRegistry,
      usdcMint,
      treasury,
      authority: payer.publicKey,
      tokenProgram: TOKEN_PROGRAM_ID,
      associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
      systemProgram: SystemProgram.programId,
    })
    .rpc();

  const [planPda] = PublicKey.findProgramAddressSync(
    [PLAN_SEED, Buffer.from([1])],
    programId
  );
  const [subscriptionPda] = PublicKey.findProgramAddressSync(
    [SUBSCRIPTION_SEED, payer.publicKey.toBuffer()],
    programId
  );

  await program.methods
    .addPlan({
      planType: { standard: {} },
      subscriptionPrice: new anchor.BN(INIT.subscriptionPrice),
      maxSigners: INIT.planMaxSigners,
      maxDelegates: INIT.planMaxDelegates,
      privateApiEnabled: false,
      isActive: true,
    })
    .accountsPartial({
      protocolConfig: protocolConfigPda,
      plan: planPda,
      authority: payer.publicKey,
      systemProgram: SystemProgram.programId,
    })
    .rpc();

  await program.methods
    .subscribe({ standard: {} })
    .accountsPartial({
      owner: payer.publicKey,
      protocolConfig: protocolConfigPda,
      plan: planPda,
      subscription: subscriptionPda,
      usdcMint,
      ownerUsdc,
      treasury,
      systemProgram: SystemProgram.programId,
      tokenProgram: TOKEN_PROGRAM_ID,
    })
    .rpc();

  const gatewayAuthority = Keypair.generate();
  const authorityUsdc = getAssociatedTokenAddressSync(
    usdcMint,
    gatewayAuthority.publicKey
  );
  fundOwnerInBankrun(provider.context, gatewayAuthority.publicKey, 1_000_000_000);
  fundAtaInBankrun(
    provider.context,
    authorityUsdc,
    usdcMint,
    gatewayAuthority.publicKey,
    10_000_000_000
  );

  const gatewayPda = findGatewayPda(programId, gatewayAuthority.publicKey);
  const gatewayBondAta = findGatewayBondAta(usdcMint, gatewayPda);
  await program.methods
    .registerGateway({
      ip: [10, 0, 0, 1],
      port: 8080,
      bondAmount: new anchor.BN(INIT.gatewayBondMin),
    })
    .accountsPartial({
      gatewayAuthority: gatewayAuthority.publicKey,
      protocolConfig: protocolConfigPda,
      gateway: gatewayPda,
      usdcMint,
      authorityUsdc,
      gatewayBond: gatewayBondAta,
      systemProgram: SystemProgram.programId,
      tokenProgram: TOKEN_PROGRAM_ID,
      associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
    })
    .signers([gatewayAuthority])
    .rpc();

  const signerMaterial = generateSignerMaterial(nodeCount);
  const nodePdasByIndex = await registerNodesForBench(
    provider,
    program,
    payer,
    { protocolConfigPda, registryStatePda, usdcMint, treasury },
    signerMaterial
  );

  const registryState = await program.account.registryState.fetch(
    registryStatePda
  );
  const registryVersion = Number(registryState.currentVersion);
  const registryPda = findRegistryPda(programId, registryVersion);

  return {
    protocolConfigPda,
    registryStatePda,
    registryPda,
    registryVersion,
    subscriptionPda,
    gatewayPda,
    gatewayAuthority,
    authorityUsdc,
    treasury,
    usdcMint,
    nodePdasByIndex,
  };
}

async function buildSettleSubscriptionIx(program, setup, payer, signerCount, sourceId, timestamp) {
  const bitmap = deriveSettleSignersBitmap({
    sourceId,
    registryVersion: setup.registryVersion,
    nodeCount: setup.registryVersion,
    signaturesRequired: signerCount,
    timestamp,
  });
  const requestAuthHash = hashRequestAuthFields(
    sourceId,
    signerCount,
    timestamp
  );
  const consumerSigIx = Ed25519Program.createInstructionWithPrivateKey({
    privateKey: payer.secretKey,
    message: requestAuthHash,
  });
  const roundRecordPda = findSubscriptionRoundPda(
    program.programId,
    payer.publicKey,
    sourceId,
    signerCount,
    timestamp
  );
  const settleIx = await program.methods
    .settleSubscriptionRound({
      timestamp: new anchor.BN(timestamp),
      sourceId: Array.from(sourceId),
      signaturesRequired: signerCount,
      registryVersion: setup.registryVersion,
      aggSig: zeroedAggSig(bitmap),
      value: Array.from(new Uint8Array(32)),
      consumerAuthority: payer.publicKey,
    })
    .accountsPartial({
      payer: payer.publicKey,
      protocolConfig: setup.protocolConfigPda,
      registryState: setup.registryStatePda,
      registry: setup.registryPda,
      subscription: setup.subscriptionPda,
      consumerDelegate: null,
      gateway: setup.gatewayPda,
      roundRecord: roundRecordPda,
      instructionsSysvar: SYSVAR_INSTRUCTIONS_PUBKEY,
      systemProgram: SystemProgram.programId,
    })
    .remainingAccounts(
      remainingAccountsForBitmap(bitmap, setup.nodePdasByIndex, true)
    )
    .instruction();

  return { consumerSigIx, settleIx };
}

async function buildSettleX402Ix(
  program,
  setup,
  signerCount,
  sourceId,
  canonicalTimestamp,
  payerId
) {
  const bitmap = deriveSettleSignersBitmap({
    sourceId,
    registryVersion: setup.registryVersion,
    nodeCount: setup.registryVersion,
    signaturesRequired: signerCount,
    timestamp: canonicalTimestamp,
  });
  const roundRecordPda = findX402RoundPda(
    program.programId,
    payerId,
    setup.gatewayPda,
    sourceId,
    signerCount,
    canonicalTimestamp
  );
  const settleIx = await program.methods
    .settleX402Round({
      payerId: Array.from(payerId),
      amount: new anchor.BN(x402Price(signerCount)),
      sourceId: Array.from(sourceId),
      canonicalTimestamp: new anchor.BN(canonicalTimestamp),
      signaturesRequired: signerCount,
      registryVersion: setup.registryVersion,
      aggSig: zeroedAggSig(bitmap),
      value: Array.from(new Uint8Array(32)),
      paymentRail: 1,
      paymentRef: Buffer.alloc(0),
    })
    .accountsPartial({
      gatewayAuthority: setup.gatewayAuthority.publicKey,
      protocolConfig: setup.protocolConfigPda,
      registryState: setup.registryStatePda,
      registry: setup.registryPda,
      gateway: setup.gatewayPda,
      usdcMint: setup.usdcMint,
      gatewayUsdc: setup.authorityUsdc,
      treasury: setup.treasury,
      tokenProgram: TOKEN_PROGRAM_ID,
      roundRecord: roundRecordPda,
      systemProgram: SystemProgram.programId,
    })
    .remainingAccounts(
      remainingAccountsForBitmap(bitmap, setup.nodePdasByIndex, false)
    )
    .instruction();

  return settleIx;
}

module.exports = {
  MOCK_USDC_MINT,
  NODE_COUNT,
  SIGNER_COUNTS,
  COMPUTE_UNIT_LIMIT,
  CU_CEILINGS,
  INIT,
  loadIdl,
  loadProgramId,
  buildBankrunPrefundedAccounts,
  bootstrapSettleBenchState,
  buildSettleSubscriptionIx,
  buildSettleX402Ix,
  collectLookupAddresses,
  seedLookupTable,
  simulateV0,
  derivePayerId,
  deriveSettleSignersBitmap,
  x402Price,
};
