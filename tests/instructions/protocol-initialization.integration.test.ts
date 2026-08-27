/**
 * Protocol initialization: `initialize` creates `ProtocolConfig`, genesis
 * `RegistryState`, and the protocol treasury ATA.
 */
import { BN, Idl, Program } from "@coral-xyz/anchor";
import { assert } from "chai";
import { Keypair, PublicKey, SystemProgram } from "@solana/web3.js";
import {
  ASSOCIATED_TOKEN_PROGRAM_ID,
  TOKEN_2022_PROGRAM_ID,
  TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import { setupTestContext, TestContext } from "../setup";
import {
  advanceClock,
  buildInitializeArgs,
  rpcInitialize,
} from "../helpers/settlement-access";
import {
  findRegistryPda,
  getProtocolTreasury,
} from "../helpers/submit-attestation";
import molphaIdl from "../../target/idl/molpha.json";

/** Prefer the built IDL address (matches `declare_id!` / Anchor.toml localnet). */
const LOCALNET_PROGRAM_ID = new PublicKey(
  (molphaIdl as { address: string }).address
);
const PROTOCOL_CONFIG_SEED = Buffer.from("molpha_config");
const REGISTRY_STATE_SEED = Buffer.from("molpha_registry");

const INIT_ARGS = buildInitializeArgs(PublicKey.default, {
  protocol_fee: new BN(30),
  dispute_window_slots: new BN(1_024),
});

interface ProtocolAccounts {
  program: any;
  protocolConfigPda: PublicKey;
  registryStatePda: PublicKey;
  treasury: PublicKey;
}

function programFromCtx(ctx: TestContext): any {
  return new Program(
    {
      ...molphaIdl,
      address: LOCALNET_PROGRAM_ID.toBase58(),
    } as Idl,
    ctx.provider
  );
}

function findProtocolAccounts(
  programId: PublicKey,
  usdcMint: PublicKey
): Omit<ProtocolAccounts, "program"> {
  const [protocolConfigPda] = PublicKey.findProgramAddressSync(
    [PROTOCOL_CONFIG_SEED],
    programId
  );
  const [registryStatePda] = PublicKey.findProgramAddressSync(
    [REGISTRY_STATE_SEED],
    programId
  );
  const treasury = getProtocolTreasury(protocolConfigPda, usdcMint);
  return { protocolConfigPda, registryStatePda, treasury };
}

async function readTokenAmount(
  ctx: TestContext,
  address: PublicKey
): Promise<bigint> {
  const account = await ctx.provider.context.banksClient.getAccount(address);
  assert(account, `missing token account ${address.toBase58()}`);
  return Buffer.from(account!.data).readBigUInt64LE(64);
}

function seedMint(
  ctx: TestContext,
  mint: PublicKey,
  mintAuthority: PublicKey,
  tokenProgram: PublicKey = TOKEN_PROGRAM_ID
): void {
  const mintData = Buffer.alloc(82);
  mintData.writeUInt32LE(1, 0);
  mintAuthority.toBuffer().copy(mintData, 4);
  mintData.writeBigUInt64LE(BigInt(1_000_000_000_000), 36);
  mintData.writeUInt8(6, 44);
  mintData.writeUInt8(1, 45);
  mintData.writeUInt32LE(0, 46);
  ctx.provider.context.setAccount(mint, {
    lamports: 1_000_000_000,
    owner: tokenProgram,
    executable: false,
    data: mintData,
  });
}

async function initializeProtocol(
  ctx: TestContext,
  overrides: {
    usdcMint?: PublicKey;
    args?: Record<string, unknown>;
  } = {}
): Promise<ProtocolAccounts> {
  const program = programFromCtx(ctx);
  const usdcMint = overrides.usdcMint ?? ctx.underlyingTokenMint;
  const accounts = findProtocolAccounts(program.programId, usdcMint);

  await rpcInitialize(
    program,
    {
      protocolConfig: accounts.protocolConfigPda,
      registryState: accounts.registryStatePda,
      usdcMint,
      treasury: accounts.treasury,
      authority: ctx.authority.publicKey,
    },
    usdcMint,
    {
      protocol_fee: INIT_ARGS.protocol_fee,
      dispute_window_slots: INIT_ARGS.dispute_window_slots,
      ...(overrides.args ?? {}),
    }
  );

  return { program, ...accounts };
}

describe("protocol initialization", () => {
  it("initializes protocol config, registry genesis state, and empty treasury", async () => {
    const ctx = await setupTestContext();
    const { program, protocolConfigPda, registryStatePda, treasury } =
      await initializeProtocol(ctx);

    const protocolConfig = await program.account.protocolConfig.fetch(
      protocolConfigPda
    );
    assert.equal(
      protocolConfig.authority.toBase58(),
      ctx.authority.publicKey.toBase58()
    );
    assert.equal(
      protocolConfig.usdcMint.toBase58(),
      ctx.underlyingTokenMint.toBase58()
    );
    assert.equal(protocolConfig.rewardPerSignature.toString(), "10");
    assert.equal(protocolConfig.maxReceiptDelaySlots.toString(), "256");
    assert.equal(protocolConfig.protocolFee.toString(), "30");
    assert.equal(protocolConfig.disputeWindowSlots.toString(), "1024");
    assert.equal(protocolConfig.gatewayBondMin.toString(), "500000000");
    assert.equal(protocolConfig.challengerBountyBps, 500);
    assert.equal(protocolConfig.minimumNodeDeposit.toString(), "100000000");
    assert.equal(protocolConfig.withdrawalCooldownSecs.toString(), "5");
    assert.equal(protocolConfig.x402RoundBase.toString(), "1000");
    assert.equal(protocolConfig.x402RoundPerSig.toString(), "15");
    assert.equal(protocolConfig.minSigners, 3);

    const registryState = await program.account.registryState.fetch(
      registryStatePda
    );
    assert.equal(registryState.currentVersion, 0);

    const registryPda = findRegistryPda(program.programId, 0);
    const registry = await program.account.registry.fetch(registryPda);
    assert.equal(registry.version, 0);
    assert.equal(registry.nodeCount, 0);
    assert.equal(registry.redundancyBuffer, 2);
    assert.equal(
      (await ctx.provider.context.banksClient.getAccount(registryPda))!.data
        .length,
      8_208
    );

    const treasuryAccount = await ctx.provider.context.banksClient.getAccount(
      treasury
    );
    assert(treasuryAccount, "treasury ATA must exist");
    assert.equal((await readTokenAmount(ctx, treasury)).toString(), "0");

    const treasuryData = Buffer.from(treasuryAccount!.data);
    assert.equal(
      new PublicKey(treasuryData.subarray(0, 32)).toBase58(),
      ctx.underlyingTokenMint.toBase58()
    );
    assert.equal(
      new PublicKey(treasuryData.subarray(32, 64)).toBase58(),
      protocolConfigPda.toBase58()
    );
  });

  it("rejects duplicate initialization", async () => {
    const ctx = await setupTestContext();
    const { program, protocolConfigPda, registryStatePda, treasury } =
      await initializeProtocol(ctx);

    await advanceClock(ctx, 5);
    // Bankrun keeps the same recent blockhash until the slot advances; warp so
    // this retry is not rejected as "This transaction has already been processed".
    const clock = await ctx.provider.context.banksClient.getClock();
    ctx.provider.context.warpToSlot(clock.slot + 1n);

    try {
      await program.methods
        .initialize({
          ...INIT_ARGS,
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
      assert.fail("expected duplicate initialize to fail");
    } catch (error) {
      assert.match(
        String(error),
        /already in use|AccountNotInitialized/i
      );
    }
  });

  it("rejects usdc mint mismatch between args and account", async () => {
    const ctx = await setupTestContext();
    const program = programFromCtx(ctx);
    const otherMint = Keypair.generate().publicKey;
    seedMint(ctx, otherMint, ctx.authority.publicKey);
    const accounts = findProtocolAccounts(
      program.programId,
      ctx.underlyingTokenMint
    );

    try {
      await program.methods
        .initialize({
          ...INIT_ARGS,
          usdcMint: otherMint,
        })
        .accountsPartial({
          protocolConfig: accounts.protocolConfigPda,
          registryState: accounts.registryStatePda,
          usdcMint: ctx.underlyingTokenMint,
          treasury: accounts.treasury,
          authority: ctx.authority.publicKey,
          tokenProgram: TOKEN_PROGRAM_ID,
          associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
        })
        .rpc();
      assert.fail("expected mint mismatch to fail");
    } catch (error: any) {
      assert.include(String(error), "InvalidMint");
    }
  });

  it("rejects a Token-2022 mint", async () => {
    const ctx = await setupTestContext();
    const token2022Mint = Keypair.generate().publicKey;
    seedMint(
      ctx,
      token2022Mint,
      ctx.authority.publicKey,
      TOKEN_2022_PROGRAM_ID
    );

    try {
      await initializeProtocol(ctx, { usdcMint: token2022Mint });
      assert.fail("expected Token-2022 mint to fail");
    } catch (error: any) {
      assert.include(String(error), "AccountOwnedByWrongProgram");
    }
  });
});
