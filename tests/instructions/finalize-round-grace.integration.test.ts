/**
 * Regression: finalize_round must succeed after registry grace expires and the
 * round's registry version is no longer current or previous.
 */
import * as anchor from "@coral-xyz/anchor";
import { BN } from "@coral-xyz/anchor";
import { assert } from "chai";
import * as crypto from "crypto";
import { Clock } from "solana-bankrun";
import {
  ASSOCIATED_TOKEN_PROGRAM_ID,
  getAssociatedTokenAddressSync,
  TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import { Keypair, PublicKey, SystemProgram } from "@solana/web3.js";
import { setupTestContext, TestContext } from "../setup";
import {
  EVM_FIXTURE_NODE_PUBKEYS,
  findNodePdaByOwner,
  findRegistryIndexPda,
  fundNodeOwnerInBankrun,
  getProtocolTreasury,
  registerNodesForE2e,
} from "../helpers/evm-submit-data-update";
import {
  bitmapForFirstN,
  deriveJobId,
  FEED_SEED,
  finalizeRemainingAccounts,
  GATEWAY_SEED,
  hashCreditedNodes,
  JOB_SEED,
  PLAN_SEED,
  sendSettleRoundReceipt,
  settleRemainingAccounts,
  SUBSCRIPTION_SEED,
} from "../helpers/settle-round-receipt";

const PROTOCOL_CONFIG_SEED = Buffer.from("molpha_config");
const REGISTRY_STATE_SEED = Buffer.from("molpha_registry");
const GATEWAY_BOND_MIN = new BN(500_000_000);
const REWARD_PER_SIGNATURE = new BN(100);
const GATEWAY_FEE = new BN(50);
const DISPUTE_WINDOW_SLOTS = 2;
const PREVIOUS_GRACE_SECS = 1;

async function advanceClock(ctx: TestContext, seconds: number): Promise<void> {
  const currentClock = await ctx.provider.context.banksClient.getClock();
  const newClock = new Clock(
    currentClock.slot + BigInt(seconds),
    currentClock.epochStartTimestamp,
    currentClock.epoch,
    currentClock.leaderScheduleEpoch,
    currentClock.unixTimestamp + BigInt(seconds)
  );
  ctx.provider.context.setClock(newClock);
}

function findGatewayPda(programId: PublicKey, authority: PublicKey): PublicKey {
  const [pda] = PublicKey.findProgramAddressSync(
    [GATEWAY_SEED, authority.toBuffer()],
    programId
  );
  return pda;
}

async function initializeProtocol(ctx: TestContext): Promise<{
  program: anchor.Program;
  protocolConfigPda: PublicKey;
  registryStatePda: PublicKey;
  treasury: PublicKey;
}> {
  const program = ctx.molphaProgram as anchor.Program;
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

  await program.methods
    .initialize({
      registryCooldownSecs: new BN(0),
      previousGraceSecs: new BN(PREVIOUS_GRACE_SECS),
      withdrawalCooldownSecs: new BN(5),
      maxNodeCount: 8,
      redundancyBuffer: 0,
      minimumNodeDeposit: new BN(100_000_000),
      rewardPerSignature: REWARD_PER_SIGNATURE,
      gatewayFee: GATEWAY_FEE,
      receiptSubmitterBonus: new BN(5),
      protocolFeeBps: 100,
      maxReceiptDelaySlots: new BN(256),
      protocolFee: new BN(0),
      disputeWindowSlots: new BN(DISPUTE_WINDOW_SLOTS),
      gatewayBondMin: GATEWAY_BOND_MIN,
      challengerBountyBps: 500,
      usdcMint: ctx.underlyingTokenMint,
    })
    .accountsPartial({
      protocolConfig: protocolConfigPda,
      registryState: registryStatePda,
      usdcMint: ctx.underlyingTokenMint,
      treasury,
      authority: ctx.authority.publicKey,
      tokenProgram: TOKEN_PROGRAM_ID,
      associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
      systemProgram: SystemProgram.programId,
    })
    .rpc();

  return { program, protocolConfigPda, registryStatePda, treasury };
}

describe("finalize_round after registry grace expiry", () => {
  it("finalizes and pays rewards when the settled registry version is rolled past", async () => {
    const nodeCount = 2;
    const ctx = await setupTestContext();
    const { program, protocolConfigPda, registryStatePda, treasury } =
      await initializeProtocol(ctx);

    const payer = ctx.authority.payer;
    const payerUsdc = ctx.userTokenAccount;

    const [planPda] = PublicKey.findProgramAddressSync(
      [PLAN_SEED, Buffer.from([1])],
      program.programId
    );
    const [subscriptionPda] = PublicKey.findProgramAddressSync(
      [SUBSCRIPTION_SEED, payer.publicKey.toBuffer()],
      program.programId
    );

    await program.methods
      .addPlan({
        planType: { standard: {} },
        subscriptionPrice: new BN(100_000),
        maxJobs: 8,
        maxSigners: 8,
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
        usdcMint: ctx.underlyingTokenMint,
        ownerUsdc: payerUsdc,
        treasury,
        systemProgram: SystemProgram.programId,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .rpc();

    const apiConfigHash = crypto.randomBytes(32);
    const jobId = deriveJobId(payer.publicKey, apiConfigHash);
    const [jobPda] = PublicKey.findProgramAddressSync(
      [JOB_SEED, jobId],
      program.programId
    );
    const [feedPda] = PublicKey.findProgramAddressSync(
      [FEED_SEED, jobId],
      program.programId
    );

    await program.methods
      .createJob(
        {
          apiConfigHash: Array.from(apiConfigHash),
          signaturesRequired: 1,
          decimals: 8,
        },
        Array.from(jobId)
      )
      .accountsPartial({
        creator: payer.publicKey,
        jobOwner: payer.publicKey,
        protocolConfig: protocolConfigPda,
        plan: planPda,
        job: jobPda,
        subscription: subscriptionPda,
        feed: feedPda,
        delegate: null,
        systemProgram: SystemProgram.programId,
      })
      .rpc();

    const gatewayAuthority = Keypair.generate();
    const gatewayUsdc = await fundNodeOwnerInBankrun(
      ctx,
      gatewayAuthority,
      ctx.underlyingTokenMint,
      BigInt(2_000_000_000)
    );
    const gatewayPda = findGatewayPda(program.programId, gatewayAuthority.publicKey);

    await program.methods
      .registerGateway({
        ip: [10, 0, 0, 9],
        port: 8080,
        bondAmount: GATEWAY_BOND_MIN,
      })
      .accountsPartial({
        gatewayAuthority: gatewayAuthority.publicKey,
        protocolConfig: protocolConfigPda,
        gateway: gatewayPda,
        usdcMint: ctx.underlyingTokenMint,
        authorityUsdc: gatewayUsdc,
        treasury,
        systemProgram: SystemProgram.programId,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers([gatewayAuthority])
      .rpc();

    const gateway = await program.account.gateway.fetch(gatewayPda);

    await advanceClock(ctx, 2);
    await registerNodesForE2e(
      ctx,
      program,
      {
        protocolConfigPda,
        registryStatePda,
        usdcMint: ctx.underlyingTokenMint,
        ownerUsdc: payerUsdc,
        treasury,
      },
      EVM_FIXTURE_NODE_PUBKEYS.slice(0, nodeCount),
      advanceClock,
      { finalize: false }
    );

    const registryIndexPdasByIndex = new Map<number, PublicKey>();
    const nodePdasByIndex = new Map<number, PublicKey>();
    for (let index = 0; index < nodeCount; index++) {
      const nodeOwner = ctx.nodes[index].publicKey;
      nodePdasByIndex.set(
        index,
        findNodePdaByOwner(program.programId, nodeOwner)
      );
      registryIndexPdasByIndex.set(
        index,
        findRegistryIndexPda(program.programId, index)
      );
    }

    const registryBeforeSettle = await program.account.registryState.fetch(
      registryStatePda
    );
    const settledRegistryVersion = registryBeforeSettle.currentVersion as number;

    const creditedBitmap = bitmapForFirstN(1);
    const timestamp = 1_700_000_001;
    const value = crypto.randomBytes(32);

    const roundRecordPda = await sendSettleRoundReceipt(ctx, program, {
      payer,
      protocolConfigPda,
      registryStatePda,
      jobPda,
      subscriptionPda,
      gatewayPda,
      gatewayAuthority,
      jobId,
      registryVersion: settledRegistryVersion,
      gatewayIndex: gateway.index as number,
      signaturesRequired: 1,
      creditedBitmap,
      timestamp,
      value,
      remainingAccounts: settleRemainingAccounts(
        creditedBitmap,
        registryIndexPdasByIndex,
        nodePdasByIndex
      ),
    });

    const creditedNodePda = nodePdasByIndex.get(0)!;
    const creditedNodeOwner = ctx.nodes[0];
    const creditedNodeBefore = await program.account.node.fetch(creditedNodePda);
    assert.equal(
      creditedNodeBefore.pendingRewards.toString(),
      REWARD_PER_SIGNATURE.toString()
    );

    const roundRecord = await program.account.roundReceiptRecord.fetch(
      roundRecordPda
    );
    const expectedHash = hashCreditedNodes([creditedNodePda]);
    assert.deepEqual(
      Array.from(roundRecord.creditedNodesHash as number[]),
      Array.from(expectedHash)
    );

    // Add another node -> registry transition; settled version becomes previous.
    await advanceClock(ctx, PREVIOUS_GRACE_SECS + 1);
    await registerNodesForE2e(
      ctx,
      program,
      {
        protocolConfigPda,
        registryStatePda,
        usdcMint: ctx.underlyingTokenMint,
        ownerUsdc: payerUsdc,
        treasury,
      },
      [EVM_FIXTURE_NODE_PUBKEYS[2]],
      advanceClock,
      { startIndex: nodeCount, finalize: false }
    );

    // Let previous-version grace expire and roll registry metadata forward.
    await advanceClock(ctx, PREVIOUS_GRACE_SECS + 1);
    await program.methods
      .finalizeRegistryTransition()
      .accountsPartial({
        registryState: registryStatePda,
        virtualRegistryIndex: null,
        rentReceiver: payer.publicKey,
      })
      .rpc();

    const registryAfterGrace = await program.account.registryState.fetch(
      registryStatePda
    );
    assert.notEqual(
      settledRegistryVersion,
      registryAfterGrace.currentVersion as number
    );
    assert.notEqual(
      settledRegistryVersion,
      registryAfterGrace.previousVersion as number
    );

    // Past dispute window; old code would reject with InvalidRegistryVersion here.
    await advanceClock(ctx, DISPUTE_WINDOW_SLOTS + 2);

    await program.methods
      .finalizeRound()
      .accountsPartial({
        caller: payer.publicKey,
        protocolConfig: protocolConfigPda,
        roundRecord: roundRecordPda,
        gateway: gatewayPda,
      })
      .remainingAccounts(
        finalizeRemainingAccounts(creditedBitmap, nodePdasByIndex)
      )
      .rpc();

    const creditedNodeAfter = await program.account.node.fetch(creditedNodePda);
    assert.equal(creditedNodeAfter.pendingRewards.toString(), "0");
    assert.equal(
      creditedNodeAfter.claimableRewards.toString(),
      REWARD_PER_SIGNATURE.toString()
    );

    const finalizedRecord = await program.account.roundReceiptRecord.fetch(
      roundRecordPda
    );
    assert.isTrue(finalizedRecord.finalized as boolean);

    const claimDestination = await fundNodeOwnerInBankrun(
      ctx,
      creditedNodeOwner,
      ctx.underlyingTokenMint
    );
    const balanceBefore = Buffer.from(
      (
        await ctx.provider.context.banksClient.getAccount(claimDestination)
      )!.data
    ).readBigUInt64LE(64);

    await program.methods
      .claimNodeRewards()
      .accountsPartial({
        owner: creditedNodeOwner.publicKey,
        protocolConfig: protocolConfigPda,
        node: creditedNodePda,
        treasury,
        destination: claimDestination,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers([creditedNodeOwner])
      .rpc();

    const balanceAfter = Buffer.from(
      (await ctx.provider.context.banksClient.getAccount(claimDestination))!.data
    ).readBigUInt64LE(64);
    assert.equal(
      (balanceAfter - balanceBefore).toString(),
      REWARD_PER_SIGNATURE.toString()
    );
  });
});
