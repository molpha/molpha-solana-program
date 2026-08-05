/**
 * update_node_endpoint: active node operators can change IP/port.
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
  findPermanentNodePda,
  getProtocolTreasury,
  registerNodesForE2e,
  signerMaterialForVerifyAnswerEvm,
} from "../helpers/submit-data-update";

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

describe("update_node_endpoint", () => {
  it("updates ip and port for an active node operator", async () => {
    const ctx = await setupTestContext();
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
        withdrawalCooldownSecs: new BN(5),
        maxNodeCount: 8,
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
        authority: ctx.authority.publicKey,
        protocolConfig: protocolConfigPda,
        registryState: registryStatePda,
        treasury,
        usdcMint: ctx.underlyingTokenMint,
        systemProgram: SystemProgram.programId,
        tokenProgram: TOKEN_PROGRAM_ID,
        associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
      })
      .rpc();

    await advanceClock(ctx, 5);
    const fixtureMaterial = signerMaterialForVerifyAnswerEvm();
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
      {
        privKeys: fixtureMaterial.privKeys.slice(0, 1),
        compressedPubkeys: fixtureMaterial.compressedPubkeys.slice(0, 1),
      },
      advanceClock
    );

    const nodeOwner = ctx.nodes[0].publicKey;
    const node = findPermanentNodePda(program.programId, nodeOwner);
    const before = await program.account.node.fetch(node);
    assert.deepEqual(Array.from(before.ip), [127, 0, 0, 1]);
    assert.equal(before.port, 8080);

    try {
      await program.methods
        .updateNodeEndpoint({
          ip: [0, 0, 0, 0],
          port: 0,
        })
        .accountsPartial({
          nodeOwner,
          node,
        })
        .signers([ctx.nodes[0]])
        .rpc();
      assert.fail("update_node_endpoint should reject invalid endpoint");
    } catch (error: unknown) {
      assert.include(String(error), "InvalidNodeEndpoint");
    }

    const newIp = [10, 0, 0, 5];
    const newPort = 9443;

    await program.methods
      .updateNodeEndpoint({
        ip: newIp,
        port: newPort,
      })
      .accountsPartial({
        nodeOwner,
        node,
      })
      .signers([ctx.nodes[0]])
      .rpc();

    const after = await program.account.node.fetch(node);
    assert.deepEqual(Array.from(after.ip), newIp);
    assert.equal(after.port, newPort);
  });
});
