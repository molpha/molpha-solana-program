import * as anchor from "@coral-xyz/anchor";
import { BN } from "@coral-xyz/anchor";
import { PublicKey, SystemProgram } from "@solana/web3.js";
import {
  ASSOCIATED_TOKEN_PROGRAM_ID,
  TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import * as fs from "fs";
import { setupTestContext } from "../setup";
import idl from "../../target/idl/molpha.json";
import {
  EVM_FIXTURE_NODE_PUBKEYS,
  getProtocolTreasury,
  registerNodesForE2e,
} from "../helpers/evm-submit-data-update";

const PROTOCOL_CONFIG_SEED = Buffer.from("molpha_config");
const REGISTRY_STATE_SEED = Buffer.from("molpha_registry");

async function advanceClock(
  ctx: Awaited<ReturnType<typeof setupTestContext>>,
  seconds: number
): Promise<void> {
  const { Clock } = await import("solana-bankrun");
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

describe("register_node e2e (no registry seeding)", () => {
  it("registers eight fixture nodes via instruction", async () => {
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
    const treasury = getProtocolTreasury(protocolConfigPda, ctx.underlyingTokenMint);

    await program.methods
      .initialize({
        registryCooldownSecs: new BN(1),
        previousGraceSecs: new BN(1),
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
      EVM_FIXTURE_NODE_PUBKEYS.slice(0, 8),
      advanceClock
    );

    const registry = await program.account.registryState.fetch(registryStatePda);
    expect(registry.currentNodeCount).toBe(8);
    expect(registry.currentVersion).toBe(8);
  });
});
