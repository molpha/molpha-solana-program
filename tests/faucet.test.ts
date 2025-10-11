import * as anchor from "@coral-xyz/anchor";
import { assert } from "chai";
import {
  PublicKey,
  SystemProgram,
  Keypair,
  SYSVAR_RENT_PUBKEY,
} from "@solana/web3.js";
import {
  getAssociatedTokenAddressSync,
  getAccount,
  TOKEN_PROGRAM_ID,
  ASSOCIATED_TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import { BankrunProvider, startAnchor } from "anchor-bankrun";
import { Clock } from "solana-bankrun";
import { Faucet } from "../target/types/faucet";

interface FaucetTestContext {
  faucetProgram: anchor.Program<Faucet>;
  provider: BankrunProvider;
  authority: anchor.Wallet;
  user: Keypair;
  tokenMint: Keypair;
}

async function setupFaucetTestContext(): Promise<FaucetTestContext> {
  // Create a fresh context for the faucet tests
  const authority = new anchor.Wallet(Keypair.generate());
  const user = Keypair.generate();
  const tokenMint = Keypair.generate();

  const accounts = [
    {
      address: authority.publicKey,
      info: {
        lamports: 10_000_000_000, // 10 SOL
        executable: false,
        owner: anchor.web3.SystemProgram.programId,
        data: Buffer.alloc(0),
      },
    },
    {
      address: user.publicKey,
      info: {
        lamports: 10_000_000_000, // 10 SOL for the user too
        executable: false,
        owner: anchor.web3.SystemProgram.programId,
        data: Buffer.alloc(0),
      },
    },
  ];

  const context = await startAnchor(
    ".",
    [
      {
        name: "faucet",
        programId: new PublicKey(
          "5UAgne7K7mBfdtV6pU2jAT58U32PjQ2MPw4eubsKJPwv"
        ),
      },
    ],
    accounts
  );

  const provider = new BankrunProvider(context);
  provider.wallet = authority;
  anchor.setProvider(provider);

  const faucetProgram = anchor.workspace.Faucet as anchor.Program<Faucet>;

  return {
    faucetProgram,
    provider,
    authority,
    user,
    tokenMint,
  };
}

describe("Faucet Program", () => {
  let ctx: FaucetTestContext;
  let faucetConfigPDA: PublicKey;
  let userTokenAccount: PublicKey;
  let userCooldownPDA: PublicKey;

  before(async () => {
    ctx = await setupFaucetTestContext();

    // Initialize the faucet as part of setup
    const amountPerRequest = new anchor.BN(1000 * 1e6); // 1000 tokens with 6 decimals
    const cooldownSeconds = new anchor.BN(5); // 5 seconds for testing
    const tokenDecimals = 6;
    const tokenName = "Test Token";
    const tokenSymbol = "TEST";

    [faucetConfigPDA] = PublicKey.findProgramAddressSync(
      [Buffer.from("faucet_config"), ctx.tokenMint.publicKey.toBuffer()],
      ctx.faucetProgram.programId
    );

    await ctx.faucetProgram.methods
      .initialize(
        amountPerRequest,
        cooldownSeconds,
        tokenDecimals,
        tokenName,
        tokenSymbol
      )
      .accountsPartial({
        faucetConfig: faucetConfigPDA,
        tokenMint: ctx.tokenMint.publicKey,
        authority: ctx.authority.publicKey,
        systemProgram: SystemProgram.programId,
        tokenProgram: TOKEN_PROGRAM_ID,
        rent: SYSVAR_RENT_PUBKEY,
      })
      .signers([ctx.authority.payer, ctx.tokenMint])
      .rpc();

    userTokenAccount = getAssociatedTokenAddressSync(
      ctx.tokenMint.publicKey,
      ctx.user.publicKey
    );

    [userCooldownPDA] = PublicKey.findProgramAddressSync(
      [
        Buffer.from("user_cooldown"),
        ctx.user.publicKey.toBuffer(),
        ctx.tokenMint.publicKey.toBuffer(),
      ],
      ctx.faucetProgram.programId
    );
  });

  describe("Initialize", () => {
    it("Successfully initializes a faucet with new token mint", async () => {
      // Verify the faucet config was created correctly
      const faucetConfig = await ctx.faucetProgram.account.faucetConfig.fetch(
        faucetConfigPDA
      );
      assert.ok(faucetConfig.authority.equals(ctx.authority.publicKey));
      assert.ok(faucetConfig.tokenMint.equals(ctx.tokenMint.publicKey));
      assert.equal(
        faucetConfig.amountPerRequest.toNumber(),
        1000 * 1e6
      );
      assert.equal(
        faucetConfig.cooldownSeconds.toNumber(),
        5
      );
      assert.equal(faucetConfig.tokenDecimals, 6);
      assert.equal(faucetConfig.isActive, true);

      console.log(
        "Faucet initialized successfully with token mint:",
        ctx.tokenMint.publicKey.toString()
      );
    });
  });

  describe("Request Tokens", () => {

    it("Successfully requests tokens from faucet", async () => {
      await ctx.faucetProgram.methods
        .requestTokens()
        .accountsPartial({
          faucetConfig: faucetConfigPDA,
          userCooldown: userCooldownPDA,
          tokenMint: ctx.tokenMint.publicKey,
          userTokenAccount: userTokenAccount,
          user: ctx.user.publicKey,
          systemProgram: SystemProgram.programId,
          tokenProgram: TOKEN_PROGRAM_ID,
          associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
        })
        .signers([ctx.user])
        .rpc();

      // Verify cooldown was set
      const userCooldown = await ctx.faucetProgram.account.userCooldown.fetch(
        userCooldownPDA
      );
      assert.ok(userCooldown.user.equals(ctx.user.publicKey));
      assert.isTrue(userCooldown.lastRequest > 0);

      // Verify tokens were minted to user
      const userAccount = await getAccount(
        ctx.provider.connection,
        userTokenAccount
      );
      assert.equal(userAccount.amount.toString(), (1000 * 1e6).toString());

      console.log("User successfully received tokens from faucet");
    });

    it("Fails to request tokens during cooldown period", async () => {
      try {
        await ctx.faucetProgram.methods
          .requestTokens()
          .accountsPartial({
            faucetConfig: faucetConfigPDA,
            userCooldown: userCooldownPDA,
            tokenMint: ctx.tokenMint.publicKey,
            userTokenAccount: userTokenAccount,
            user: ctx.user.publicKey,
            systemProgram: SystemProgram.programId,
            tokenProgram: TOKEN_PROGRAM_ID,
            associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
          })
          .signers([ctx.user])
          .rpc();

        assert.fail("Should have failed due to cooldown");
      } catch (error: any) {
        assert.include(error.message, "CooldownActive");
      }
    });

    it("Successfully requests tokens after cooldown expires", async () => {
      // Advance the clock in bankrun to simulate cooldown expiration
      const currentClock = await ctx.provider.context.banksClient.getClock();
      
      // Create new clock with advanced timestamp
      const newClock = new Clock(
        currentClock.slot + BigInt(100),
        currentClock.epochStartTimestamp,
        currentClock.epoch,
        currentClock.leaderScheduleEpoch,
        currentClock.unixTimestamp + BigInt(10) // Advance by 10 seconds
      );
      
      ctx.provider.context.setClock(newClock);

      await ctx.faucetProgram.methods
        .requestTokens()
        .accountsPartial({
          faucetConfig: faucetConfigPDA,
          userCooldown: userCooldownPDA,
          tokenMint: ctx.tokenMint.publicKey,
          userTokenAccount: userTokenAccount,
          user: ctx.user.publicKey,
          systemProgram: SystemProgram.programId,
          tokenProgram: TOKEN_PROGRAM_ID,
          associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
        })
        .signers([ctx.user])
        .rpc();

      // Verify tokens were minted again (should have 2000 tokens now)
      const userAccount = await getAccount(
        ctx.provider.connection,
        userTokenAccount
      );
      assert.equal(userAccount.amount.toString(), (2000 * 1e6).toString());

      console.log("User successfully received tokens again after cooldown");
    });
  });
});
