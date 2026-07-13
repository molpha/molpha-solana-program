import { BN } from "@coral-xyz/anchor";
import { assert } from "chai";
import { Keypair, PublicKey, SystemProgram } from "@solana/web3.js";
import {
  ASSOCIATED_TOKEN_PROGRAM_ID,
  TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import { setupTestContext } from "../setup";
import { fundNodeOwnerInBankrun } from "../helpers/evm-submit-data-update";
import {
  createFundedEscrow,
  findAgentEscrowPda,
  findEscrowAta,
  initializeProtocolForAgents,
  INIT,
  readTokenAmount,
} from "../helpers/agent-access";

describe("agent escrow lifecycle", () => {
  it("creates, funds, updates, pauses, freezes, and closes an escrow", async () => {
    const ctx = await setupTestContext();
    const proto = await initializeProtocolForAgents(ctx);

    // ---- create + deposit (happy path) ----
    const setup = await createFundedEscrow(ctx, proto, {
      maxSigners: 4,
      depositAmount: 250_000,
    });

    const treasuryBefore = await ctx.provider.context.banksClient.getAccount(
      proto.treasury
    );

    let escrow = await proto.program.account.agentEscrow.fetch(setup.escrowPda);
    assert.equal(escrow.owner.toBase58(), setup.owner.publicKey.toBase58());
    assert.equal(
      escrow.authority.toBase58(),
      setup.hotKey.publicKey.toBase58()
    );
    assert.equal(await readTokenAmount(ctx, setup.escrowAta), BigInt(250_000));
    assert.equal(escrow.spentTotal.toNumber(), 0);
    assert.equal(escrow.maxSigners, 4);
    assert.equal(escrow.epochSpent.toNumber(), 0);
    assert.equal(escrow.lastRequestNonce.toNumber(), 0);
    assert.deepEqual(escrow.status, { active: {} });

    // ---- permissionless deposit by a third party; treasury receives USDC ----
    const sponsor = Keypair.generate();
    const sponsorUsdc = await fundNodeOwnerInBankrun(
      ctx,
      sponsor,
      ctx.underlyingTokenMint,
      BigInt(500_000)
    );
    await proto.program.methods
      .depositAgentCredits(new BN(100_000))
      .accountsPartial({
        depositor: sponsor.publicKey,
        protocolConfig: proto.protocolConfigPda,
        agentEscrow: setup.escrowPda,
        usdcMint: ctx.underlyingTokenMint,
        depositorUsdc: sponsorUsdc,
        escrowAta: setup.escrowAta,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers([sponsor])
      .rpc();
    // Real USDC lands in the escrow ATA; the treasury is untouched by deposits now.
    assert.equal(await readTokenAmount(ctx, setup.escrowAta), BigInt(350_000));

    const treasuryAfter = await ctx.provider.context.banksClient.getAccount(
      proto.treasury
    );
    const before = Buffer.from(treasuryBefore!.data).readBigUInt64LE(64);
    const after = Buffer.from(treasuryAfter!.data).readBigUInt64LE(64);
    assert.equal(after - before, BigInt(0));

    // ---- zero deposit rejected ----
    try {
      await proto.program.methods
        .depositAgentCredits(new BN(0))
        .accountsPartial({
          depositor: sponsor.publicKey,
          protocolConfig: proto.protocolConfigPda,
          agentEscrow: setup.escrowPda,
          usdcMint: ctx.underlyingTokenMint,
          depositorUsdc: sponsorUsdc,
          escrowAta: setup.escrowAta,
          tokenProgram: TOKEN_PROGRAM_ID,
        })
        .signers([sponsor])
        .rpc();
      assert.fail("expected InvalidAmount");
    } catch (e: any) {
      assert.include(e.toString(), "InvalidAmount");
    }

    // ---- policy update (owner-gated; cap re-validated). `authority` is a PDA seed now, so
    // it can no longer be rotated in place. ----
    await proto.program.methods
      .updateAgentPolicy({
        maxSigners: 6,
        epochCap: null,
        expiresAt: null,
      })
      .accountsPartial({
        owner: setup.owner.publicKey,
        protocolConfig: proto.protocolConfigPda,
        agentEscrow: setup.escrowPda,
      })
      .signers([setup.owner])
      .rpc();
    escrow = await proto.program.account.agentEscrow.fetch(setup.escrowPda);
    assert.equal(
      escrow.authority.toBase58(),
      setup.hotKey.publicKey.toBase58()
    );
    assert.equal(escrow.maxSigners, 6);

    try {
      await proto.program.methods
        .updateAgentPolicy({
          maxSigners: 0,
          epochCap: null,
          expiresAt: null,
        })
        .accountsPartial({
          owner: setup.owner.publicKey,
          protocolConfig: proto.protocolConfigPda,
          agentEscrow: setup.escrowPda,
        })
        .signers([setup.owner])
        .rpc();
      assert.fail("expected InvalidAgentCaps");
    } catch (e: any) {
      assert.include(e.toString(), "InvalidAgentCaps");
    }

    // ---- non-owner cannot update ----
    try {
      await proto.program.methods
        .updateAgentPolicy({
          maxSigners: 1,
          epochCap: null,
          expiresAt: null,
        })
        .accountsPartial({
          owner: sponsor.publicKey,
          protocolConfig: proto.protocolConfigPda,
          agentEscrow: setup.escrowPda,
        })
        .signers([sponsor])
        .rpc();
      assert.fail("expected UnauthorisedAgentOwner");
    } catch (e: any) {
      assert.include(e.toString(), "UnauthorisedAgentOwner");
    }

    // ---- pause / resume ----
    await proto.program.methods
      .setAgentPaused(true)
      .accountsPartial({
        owner: setup.owner.publicKey,
        agentEscrow: setup.escrowPda,
      })
      .signers([setup.owner])
      .rpc();
    escrow = await proto.program.account.agentEscrow.fetch(setup.escrowPda);
    assert.deepEqual(escrow.status, { paused: {} });

    await proto.program.methods
      .setAgentPaused(false)
      .accountsPartial({
        owner: setup.owner.publicKey,
        agentEscrow: setup.escrowPda,
      })
      .signers([setup.owner])
      .rpc();
    escrow = await proto.program.account.agentEscrow.fetch(setup.escrowPda);
    assert.deepEqual(escrow.status, { active: {} });

    // ---- freeze: protocol authority only; owner cannot unfreeze ----
    try {
      await proto.program.methods
        .setAgentFrozen(true)
        .accountsPartial({
          authority: setup.owner.publicKey,
          protocolConfig: proto.protocolConfigPda,
          agentEscrow: setup.escrowPda,
        })
        .signers([setup.owner])
        .rpc();
      assert.fail("expected UnauthorisedAuthority");
    } catch (e: any) {
      assert.include(e.toString(), "UnauthorisedAuthority");
    }

    await proto.program.methods
      .setAgentFrozen(true)
      .accountsPartial({
        authority: ctx.authority.publicKey,
        protocolConfig: proto.protocolConfigPda,
        agentEscrow: setup.escrowPda,
      })
      .rpc();
    escrow = await proto.program.account.agentEscrow.fetch(setup.escrowPda);
    assert.deepEqual(escrow.status, { frozen: {} });

    // Frozen blocks owner pause/resume, deposits, and close.
    try {
      await proto.program.methods
        .setAgentPaused(false)
        .accountsPartial({
          owner: setup.owner.publicKey,
          agentEscrow: setup.escrowPda,
        })
        .signers([setup.owner])
        .rpc();
      assert.fail("expected AgentNotActive");
    } catch (e: any) {
      assert.include(e.toString(), "AgentNotActive");
    }
    try {
      await proto.program.methods
        .depositAgentCredits(new BN(1))
        .accountsPartial({
          depositor: sponsor.publicKey,
          protocolConfig: proto.protocolConfigPda,
          agentEscrow: setup.escrowPda,
          usdcMint: ctx.underlyingTokenMint,
          depositorUsdc: sponsorUsdc,
          escrowAta: setup.escrowAta,
          tokenProgram: TOKEN_PROGRAM_ID,
        })
        .signers([sponsor])
        .rpc();
      assert.fail("expected AgentNotActive");
    } catch (e: any) {
      assert.include(e.toString(), "AgentNotActive");
    }

    // Unfreeze restores to Paused (owner must explicitly resume).
    await proto.program.methods
      .setAgentFrozen(false)
      .accountsPartial({
        authority: ctx.authority.publicKey,
        protocolConfig: proto.protocolConfigPda,
        agentEscrow: setup.escrowPda,
      })
      .rpc();
    escrow = await proto.program.account.agentEscrow.fetch(setup.escrowPda);
    assert.deepEqual(escrow.status, { paused: {} });

    // ---- close: rejected while funded, allowed when empty ----
    try {
      await proto.program.methods
        .closeAgentEscrow()
        .accountsPartial({
          owner: setup.owner.publicKey,
          agentEscrow: setup.escrowPda,
          escrowAta: setup.escrowAta,
          usdcMint: ctx.underlyingTokenMint,
          tokenProgram: TOKEN_PROGRAM_ID,
        })
        .signers([setup.owner])
        .rpc();
      assert.fail("expected AgentEscrowNotEmpty");
    } catch (e: any) {
      assert.include(e.toString(), "AgentEscrowNotEmpty");
    }

    const emptySetup = await createFundedEscrow(ctx, proto, {
      depositAmount: 0,
    });
    await proto.program.methods
      .closeAgentEscrow()
      .accountsPartial({
        owner: emptySetup.owner.publicKey,
        agentEscrow: emptySetup.escrowPda,
        escrowAta: emptySetup.escrowAta,
        usdcMint: ctx.underlyingTokenMint,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers([emptySetup.owner])
      .rpc();
    const closed = await ctx.provider.context.banksClient.getAccount(
      emptySetup.escrowPda
    );
    assert.isTrue(closed === null || closed.data.length === 0);
    const closedAta = await ctx.provider.context.banksClient.getAccount(
      emptySetup.escrowAta
    );
    assert.isTrue(closedAta === null || closedAta.data.length === 0);
  });

  it("rejects creation with invalid caps", async () => {
    const ctx = await setupTestContext();
    const proto = await initializeProtocolForAgents(ctx);
    const owner = Keypair.generate();
    await fundNodeOwnerInBankrun(ctx, owner, ctx.underlyingTokenMint);

    const createEscrow = (authority: PublicKey, maxSigners: number) => {
      const escrowPda = findAgentEscrowPda(
        proto.program.programId,
        owner.publicKey,
        authority
      );
      return proto.program.methods
        .createAgentEscrow({
          authority,
          maxSigners,
          epochCap: new BN(INIT.epochCap),
          expiresAt: new BN(0),
        })
        .accountsPartial({
          owner: owner.publicKey,
          protocolConfig: proto.protocolConfigPda,
          agentEscrow: escrowPda,
          usdcMint: ctx.underlyingTokenMint,
          escrowAta: findEscrowAta(ctx.underlyingTokenMint, escrowPda),
          tokenProgram: TOKEN_PROGRAM_ID,
          associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
        })
        .signers([owner])
        .rpc();
    };

    // Zero caps rejected.
    try {
      await createEscrow(Keypair.generate().publicKey, 0);
      assert.fail("expected InvalidAgentCaps");
    } catch (e: any) {
      assert.include(e.toString(), "InvalidAgentCaps");
    }

    // owner == authority rejected (reserved for x402-provisioned escrows).
    try {
      await createEscrow(owner.publicKey, 4);
      assert.fail("expected InvalidAgentCaps");
    } catch (e: any) {
      assert.include(e.toString(), "InvalidAgentCaps");
    }
  });
});
