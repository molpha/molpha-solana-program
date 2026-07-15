/**
 * Registry lifecycle: deactivate_node (RemoveSwap / RemoveTail) and
 * finalize_registry_transition after grace.
 */
import * as anchor from "@coral-xyz/anchor";
import { BN } from "@coral-xyz/anchor";
import { assert } from "chai";
import { PublicKey, SystemProgram } from "@solana/web3.js";
import {
  ASSOCIATED_TOKEN_PROGRAM_ID,
  TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import { Clock } from "solana-bankrun";
import { setupTestContext, TestContext } from "../setup";
import {
  EVM_FIXTURE_NODE_PUBKEYS,
  findNodeOperatorPda,
  findNodePda,
  getProtocolTreasury,
  registerNodesForE2e,
  VIRTUAL_INDEX,
} from "../helpers/evm-submit-data-update";

const PROTOCOL_CONFIG_SEED = Buffer.from("molpha_config");
const REGISTRY_STATE_SEED = Buffer.from("molpha_registry");

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

async function initializeRegistry(
  ctx: TestContext,
  maxNodeCount: number
): Promise<{
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
      registryCooldownSecs: new BN(1),
      previousGraceSecs: new BN(1),
      withdrawalCooldownSecs: new BN(5),
      maxNodeCount,
      redundancyBuffer: 2,
      minimumNodeDeposit: new BN(100_000_000),
      rewardPerSignature: new BN(10),
      gatewayFee: new BN(20),
      receiptSubmitterBonus: new BN(5),
      protocolFeeBps: 100,
      maxReceiptDelaySlots: new BN(256),
      usdcMint: ctx.underlyingTokenMint,
      agentRoundBase: new BN(1_000),
      agentRoundPerSig: new BN(15),
      agentEpochLenSecs: new BN(86_400),
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

async function deactivateNodeAtIndex(
  ctx: TestContext,
  program: anchor.Program,
  accounts: {
    protocolConfigPda: PublicKey;
    registryStatePda: PublicKey;
  },
  removedIndex: number
): Promise<void> {
  const registry = await program.account.registryState.fetch(
    accounts.registryStatePda
  );
  const nodeCount = registry.currentNodeCount;
  const lastIndex = nodeCount - 1;

  const nodeOwner = ctx.nodes[removedIndex];
  const removedOperator = findNodeOperatorPda(
    program.programId,
    nodeOwner.publicKey
  );
  const removedNode = findNodePda(program.programId, removedIndex);
  const virtualNode = findNodePda(program.programId, VIRTUAL_INDEX);

  let lastNode: PublicKey = SystemProgram.programId;
  let movedOperator: PublicKey = SystemProgram.programId;
  if (removedIndex !== lastIndex) {
    lastNode = findNodePda(program.programId, lastIndex);
    const lastNodeData = await (program.account as any).node.fetch(lastNode);
    movedOperator = findNodeOperatorPda(
      program.programId,
      lastNodeData.owner as PublicKey
    );
  }

  try {
    await (program.methods as any)
      .deactivateNode()
      .accountsPartial({
        authority: nodeOwner.publicKey,
        protocolConfig: accounts.protocolConfigPda,
        registryState: accounts.registryStatePda,
        removedOperator,
        removedNode,
        virtualNode,
        lastNode,
        movedOperator,
        rentReceiver: ctx.authority.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .signers([nodeOwner])
      .rpc();
  } catch (error: unknown) {
    throw new Error(`deactivate_node failed: ${String(error)}`);
  }
}

describe("registry lifecycle", () => {
  it("deactivate_node RemoveSwap then finalize_registry_transition", async () => {
    const nodeCount = 3;
    const ctx = await setupTestContext();
    const { program, protocolConfigPda, registryStatePda, treasury } =
      await initializeRegistry(ctx, 8);

    await advanceClock(ctx, 5);
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
      EVM_FIXTURE_NODE_PUBKEYS.slice(0, nodeCount),
      advanceClock
    );

    const before = await program.account.registryState.fetch(registryStatePda);
    assert.equal(before.currentNodeCount, nodeCount);
    const versionBefore = before.currentVersion;

    await advanceClock(ctx, 5);

    await deactivateNodeAtIndex(
      ctx,
      program,
      {
        protocolConfigPda,
        registryStatePda,
      },
      0
    );

    const afterDeactivate = await program.account.registryState.fetch(
      registryStatePda
    );
    assert.equal(afterDeactivate.currentNodeCount, nodeCount - 1);
    assert.equal(afterDeactivate.currentVersion, versionBefore + 1);
    assert.ok(afterDeactivate.previousExpiresAt.gt(new BN(0)));

    await advanceClock(ctx, 5);

    const virtualNode = findNodePda(program.programId, VIRTUAL_INDEX);
    const deactivatedOperator = findNodeOperatorPda(
      program.programId,
      ctx.nodes[0].publicKey
    );

    await program.methods
      .finalizeRegistryTransition()
      .accountsPartial({
        registryState: registryStatePda,
        virtualNode,
        nodeOperator: deactivatedOperator,
        rentReceiver: ctx.authority.publicKey,
      })
      .rpc();

    const afterFinalize = await program.account.registryState.fetch(
      registryStatePda
    );
    assert.equal(afterFinalize.previousExpiresAt.toString(), "0");
    assert.equal(afterFinalize.currentNodeCount, nodeCount - 1);
  });
});
