import * as anchor from "@coral-xyz/anchor";
import { BN } from "@coral-xyz/anchor";
import { assert } from "chai";
import { PublicKey, SystemProgram } from "@solana/web3.js";
import {
  ASSOCIATED_TOKEN_PROGRAM_ID,
  TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import { setupTestContext } from "../setup";
import {
  buildEvmSubmitDataUpdateArgs,
  expectedRegistryIndex,
  getProtocolTreasury,
  seedEvmJobAndFeed,
  seedEvmRemoveSwapGraceState,
  signerRemainingAccounts,
  VIRTUAL_INDEX,
} from "../helpers/evm-submit-data-update";

const PROTOCOL_CONFIG_SEED = Buffer.from("molpha_config");
const REGISTRY_STATE_SEED = Buffer.from("molpha_registry");
const REGISTRY_TRANSITION_SEED = Buffer.from("molpha_registry_transition");

async function initializeRegistry(ctx: Awaited<ReturnType<typeof setupTestContext>>) {
  const program = ctx.molphaProgram as any;
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
      previousGraceSecs: new BN(3600),
      withdrawalCooldownSecs: new BN(5),
      maxNodeCount: 32,
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

  return { program, protocolConfigPda, registryStatePda };
}

describe("registry transition verify (RegistryIndex-only + VIRTUAL_INDEX)", () => {
  let ctx: Awaited<ReturnType<typeof setupTestContext>>;
  let program: any;
  let registryStatePda: PublicKey;

  beforeAll(async () => {
    ctx = await setupTestContext();
    ({ program, registryStatePda } = await initializeRegistry(ctx));
  });

  it("expectedRegistryIndex remaps removed and moved slots", () => {
    const transition = {
      removedOldIndex: 1,
      transitionType: "removeSwap" as const,
      movedOldIndex: 7,
    };
    assert.equal(expectedRegistryIndex(1, transition), VIRTUAL_INDEX);
    assert.equal(expectedRegistryIndex(7, transition), 1);
    assert.equal(expectedRegistryIndex(0, transition), 0);
  });

  it("verify_data_update accepts previous version with remapped RegistryIndex accounts", async () => {
    const registry = await program.account.registryState.fetch(registryStatePda);

    const previousVersion = 1;
    const currentVersion = 2;
    const previousVersionBytes = Buffer.alloc(4);
    previousVersionBytes.writeUInt32LE(previousVersion);
    const [transitionPda, transitionBump] = PublicKey.findProgramAddressSync(
      [REGISTRY_TRANSITION_SEED, previousVersionBytes],
      program.programId
    );

    const removedOldIndex = 1;
    const movedOldIndex = 7;
    const previousExpiresAt = new BN(Math.floor(Date.now() / 1000) + 3600);

    const transitionFields = {
      previousVersion,
      currentVersion,
      expiresAt: previousExpiresAt,
      transitionType: { removeSwap: {} },
      removedOldIndex,
      removedNode: PublicKey.unique(),
      movedOldIndex,
      movedNode: PublicKey.unique(),
      bump: transitionBump,
    };

    const transitionData = await program.coder.accounts.encode(
      "registryTransitionAccount",
      transitionFields
    );
    ctx.provider.context.setAccount(transitionPda, {
      lamports: 10_000_000,
      owner: program.programId,
      executable: false,
      data: Buffer.from(transitionData),
    });

    await seedEvmRemoveSwapGraceState(ctx, program, registryStatePda, {
      removedOldIndex,
      movedOldIndex,
      previousVersion,
      currentVersion,
      previousExpiresAt,
      transitionPda,
      authority: registry.authority,
      graceSecs: registry.graceSecs,
      withdrawalCooldownSecs: registry.withdrawalCooldownSecs,
      maxNodeCount: registry.maxNodeCount,
      registryBump: registry.bump,
    });

    const { jobPda: evmJobPda } = await seedEvmJobAndFeed(
      ctx,
      program,
      ctx.authority.publicKey
    );

    const verifyArgs = {
      ...buildEvmSubmitDataUpdateArgs(),
      registryVersion: previousVersion,
    };

    const transitionInfo = {
      removedOldIndex,
      transitionType: "removeSwap" as const,
      movedOldIndex,
    };

    await program.methods
      .verifyDataUpdate(verifyArgs)
      .accountsPartial({
        registryState: registryStatePda,
        transitionAccount: transitionPda,
        job: evmJobPda,
      })
      .remainingAccounts(
        signerRemainingAccounts(
          program.programId,
          verifyArgs.signersBitmap,
          transitionInfo
        )
      )
      .preInstructions([
        anchor.web3.ComputeBudgetProgram.setComputeUnitLimit({ units: 500_000 }),
      ])
      .rpc();
  });

  it("verify_data_update rejects naive registry_index[bit] PDAs after RemoveSwap", async () => {
    const registry = await program.account.registryState.fetch(registryStatePda);

    const previousVersion = 1;
    const currentVersion = 2;
    const previousVersionBytes = Buffer.alloc(4);
    previousVersionBytes.writeUInt32LE(previousVersion);
    const [transitionPda, transitionBump] = PublicKey.findProgramAddressSync(
      [REGISTRY_TRANSITION_SEED, previousVersionBytes],
      program.programId
    );

    const removedOldIndex = 1;
    const movedOldIndex = 7;
    const previousExpiresAt = new BN(Math.floor(Date.now() / 1000) + 3600);

    const transitionData = await program.coder.accounts.encode(
      "registryTransitionAccount",
      {
        previousVersion,
        currentVersion,
        expiresAt: previousExpiresAt,
        transitionType: { removeSwap: {} },
        removedOldIndex,
        removedNode: PublicKey.unique(),
        movedOldIndex,
        movedNode: PublicKey.unique(),
        bump: transitionBump,
      }
    );
    ctx.provider.context.setAccount(transitionPda, {
      lamports: 10_000_000,
      owner: program.programId,
      executable: false,
      data: Buffer.from(transitionData),
    });

    await seedEvmRemoveSwapGraceState(ctx, program, registryStatePda, {
      removedOldIndex,
      movedOldIndex,
      previousVersion,
      currentVersion,
      previousExpiresAt,
      transitionPda,
      authority: registry.authority,
      graceSecs: registry.graceSecs,
      withdrawalCooldownSecs: registry.withdrawalCooldownSecs,
      maxNodeCount: registry.maxNodeCount,
      registryBump: registry.bump,
    });

    const { jobPda: evmJobPda } = await seedEvmJobAndFeed(
      ctx,
      program,
      ctx.authority.publicKey
    );

    const verifyArgs = {
      ...buildEvmSubmitDataUpdateArgs(),
      registryVersion: previousVersion,
    };

    try {
      await program.methods
        .verifyDataUpdate(verifyArgs)
        .accountsPartial({
          registryState: registryStatePda,
          transitionAccount: transitionPda,
          job: evmJobPda,
        })
        .remainingAccounts(
          signerRemainingAccounts(program.programId, verifyArgs.signersBitmap)
        )
        .rpc();
      assert.fail("expected registry index validation failure for unmapped remaining accounts");
    } catch (error: any) {
      const err = String(error);
      assert.ok(
        err.includes("InvalidRegistryIndexAccount") ||
          err.includes("InvalidNodeIndex") ||
          err.includes("MissingSignerAccount") ||
          err.includes("InvalidAggregateSignature"),
        `expected registry account validation error, got: ${err}`
      );
    }
  });
});
