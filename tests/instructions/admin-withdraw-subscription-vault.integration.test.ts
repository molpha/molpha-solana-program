import { BN, Idl, Program } from "@coral-xyz/anchor";
import { assert } from "chai";
import { Keypair, PublicKey, SystemProgram } from "@solana/web3.js";
import {
  ASSOCIATED_TOKEN_PROGRAM_ID,
  TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import { setupTestContext, TestContext } from "../setup";
import { getProtocolTreasury } from "../helpers/evm-submit-data-update";
import molphaIdl from "../../target/idl/molpha.json";

const LOCALNET_PROGRAM_ID = new PublicKey(
  "MoLFeTRpDZgckPjjbLwW1wB9n85bQiqboPnvw9RwoG8"
);
const PROTOCOL_CONFIG_SEED = Buffer.from("molpha_config");
const REGISTRY_STATE_SEED = Buffer.from("molpha_registry");
const PLAN_SEED = Buffer.from("molpha_plan");
const SUBSCRIPTION_SEED = Buffer.from("molpha_subscription");

function seedTokenAccount(
  ctx: TestContext,
  address: PublicKey,
  mint: PublicKey,
  owner: PublicKey,
  amount: bigint = 0n
): PublicKey {
  const data = Buffer.alloc(165);
  mint.toBuffer().copy(data, 0);
  owner.toBuffer().copy(data, 32);
  data.writeBigUInt64LE(amount, 64);
  data.writeUInt8(1, 108);
  ctx.provider.context.setAccount(address, {
    lamports: 2_039_280,
    owner: TOKEN_PROGRAM_ID,
    executable: false,
    data,
  });
  return address;
}

function seedSystemSigner(ctx: TestContext, signer: Keypair): void {
  ctx.provider.context.setAccount(signer.publicKey, {
    lamports: 1_000_000_000,
    owner: SystemProgram.programId,
    executable: false,
    data: Buffer.alloc(0),
  });
}

async function readTokenAmount(
  ctx: TestContext,
  address: PublicKey
): Promise<bigint> {
  const account = await ctx.provider.context.banksClient.getAccount(address);
  assert(account, `missing token account ${address.toBase58()}`);
  return Buffer.from(account!.data).readBigUInt64LE(64);
}

async function initializeProtocol(ctx: TestContext): Promise<{
  program: any;
  protocolConfigPda: PublicKey;
  planPda: PublicKey;
  subscriptionPda: PublicKey;
  treasury: PublicKey;
}> {
  const program = new Program(
    {
      ...molphaIdl,
      address: LOCALNET_PROGRAM_ID.toBase58(),
    } as Idl,
    ctx.provider
  ) as any;
  const [protocolConfigPda] = PublicKey.findProgramAddressSync(
    [PROTOCOL_CONFIG_SEED],
    program.programId
  );
  const [registryStatePda] = PublicKey.findProgramAddressSync(
    [REGISTRY_STATE_SEED],
    program.programId
  );
  const planId = 1;
  const [planPda] = PublicKey.findProgramAddressSync(
    [PLAN_SEED, Buffer.from([planId])],
    program.programId
  );
  const [subscriptionPda] = PublicKey.findProgramAddressSync(
    [SUBSCRIPTION_SEED, ctx.authority.publicKey.toBuffer()],
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

  await program.methods
    .addPlan({
      planType: { standard: {} },
      subscriptionPrice: new BN(2_000_000),
      maxJobs: 3,
      signaturesRequired: 1,
      maxSigners: 2,
      privateApiEnabled: false,
      isActive: true,
    })
    .accountsPartial({
      protocolConfig: protocolConfigPda,
      plan: planPda,
      authority: ctx.authority.publicKey,
      systemProgram: SystemProgram.programId,
    })
    .rpc();

  return {
    program,
    protocolConfigPda,
    planPda,
    subscriptionPda,
    treasury,
  };
}

describe("admin withdraw subscription vault", () => {
  it("withdraws the full subscription vault balance to the admin destination", async () => {
    const ctx = await setupTestContext();
    const { program, protocolConfigPda, planPda, subscriptionPda, treasury } =
      await initializeProtocol(ctx);

    const startingUserBalance = await readTokenAmount(
      ctx,
      ctx.userTokenAccount
    );

    await program.methods
      .subscribe({ standard: {} })
      .accountsPartial({
        owner: ctx.authority.publicKey,
        protocolConfig: protocolConfigPda,
        plan: planPda,
        subscription: subscriptionPda,
        usdcMint: ctx.underlyingTokenMint,
        ownerUsdc: ctx.userTokenAccount,
        treasury,
        systemProgram: SystemProgram.programId,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .rpc();

    assert.equal((await readTokenAmount(ctx, treasury)).toString(), "2000000");
    assert.equal(
      (await readTokenAmount(ctx, ctx.userTokenAccount)).toString(),
      (startingUserBalance - 2_000_000n).toString()
    );

    await program.methods
      .adminWithdrawSubscriptionVault()
      .accountsPartial({
        protocolConfig: protocolConfigPda,
        usdcMint: ctx.underlyingTokenMint,
        treasury,
        destination: ctx.userTokenAccount,
        authority: ctx.authority.publicKey,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .rpc();

    assert.equal((await readTokenAmount(ctx, treasury)).toString(), "0");
    assert.equal(
      (await readTokenAmount(ctx, ctx.userTokenAccount)).toString(),
      startingUserBalance.toString()
    );
  });

  it("rejects non-authority callers", async () => {
    const ctx = await setupTestContext();
    const unauthorized = Keypair.generate();
    seedSystemSigner(ctx, unauthorized);
    const unauthorizedDestination = seedTokenAccount(
      ctx,
      Keypair.generate().publicKey,
      ctx.underlyingTokenMint,
      unauthorized.publicKey
    );
    const { program, protocolConfigPda, treasury } = await initializeProtocol(
      ctx
    );

    try {
      await program.methods
        .adminWithdrawSubscriptionVault()
        .accountsPartial({
          protocolConfig: protocolConfigPda,
          usdcMint: ctx.underlyingTokenMint,
          treasury,
          destination: unauthorizedDestination,
          authority: unauthorized.publicKey,
          tokenProgram: TOKEN_PROGRAM_ID,
        })
        .signers([unauthorized])
        .rpc();
      assert.fail("expected admin withdraw to reject non-authority signer");
    } catch (error: any) {
      assert.include(String(error), "UnauthorisedAuthority");
    }
  });

  it("is a no-op when the subscription vault is empty", async () => {
    const ctx = await setupTestContext();
    const { program, protocolConfigPda, treasury } = await initializeProtocol(
      ctx
    );
    const startingUserBalance = await readTokenAmount(
      ctx,
      ctx.userTokenAccount
    );

    await program.methods
      .adminWithdrawSubscriptionVault()
      .accountsPartial({
        protocolConfig: protocolConfigPda,
        usdcMint: ctx.underlyingTokenMint,
        treasury,
        destination: ctx.userTokenAccount,
        authority: ctx.authority.publicKey,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .rpc();

    assert.equal((await readTokenAmount(ctx, treasury)).toString(), "0");
    assert.equal(
      (await readTokenAmount(ctx, ctx.userTokenAccount)).toString(),
      startingUserBalance.toString()
    );
  });
});
