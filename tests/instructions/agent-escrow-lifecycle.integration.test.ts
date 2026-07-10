import { BN } from "@coral-xyz/anchor";
import { assert } from "chai";
import { Keypair, SystemProgram } from "@solana/web3.js";
import { TOKEN_PROGRAM_ID } from "@solana/spl-token";
import { setupTestContext } from "../setup";
import { fundNodeOwnerInBankrun } from "../helpers/evm-submit-data-update";
import {
  createFundedEscrow,
  findAgentEscrowPda,
  initializeProtocolForAgents,
  INIT,
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
    assert.equal(escrow.balance.toNumber(), 250_000);
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
        treasury: proto.treasury,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers([sponsor])
      .rpc();
    escrow = await proto.program.account.agentEscrow.fetch(setup.escrowPda);
    assert.equal(escrow.balance.toNumber(), 350_000);

    const treasuryAfter = await ctx.provider.context.banksClient.getAccount(
      proto.treasury
    );
    const before = Buffer.from(treasuryBefore!.data).readBigUInt64LE(64);
    const after = Buffer.from(treasuryAfter!.data).readBigUInt64LE(64);
    assert.equal(after - before, BigInt(100_000));

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
          treasury: proto.treasury,
          tokenProgram: TOKEN_PROGRAM_ID,
        })
        .signers([sponsor])
        .rpc();
      assert.fail("expected InvalidAmount");
    } catch (e: any) {
      assert.include(e.toString(), "InvalidAmount");
    }

    // ---- policy update (owner-gated; cap re-validated) ----
    const newHotKey = Keypair.generate();
    await proto.program.methods
      .updateAgentPolicy({
        authority: newHotKey.publicKey,
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
    assert.equal(escrow.authority.toBase58(), newHotKey.publicKey.toBase58());
    assert.equal(escrow.maxSigners, 6);

    try {
      await proto.program.methods
        .updateAgentPolicy({
          authority: null,
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
          authority: null,
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
          treasury: proto.treasury,
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
        })
        .signers([setup.owner])
        .rpc();
      assert.fail("expected AgentEscrowNotEmpty");
    } catch (e: any) {
      assert.include(e.toString(), "AgentEscrowNotEmpty");
    }

    const emptySetup = await createFundedEscrow(ctx, proto, {
      agentIndex: 1,
      depositAmount: 0,
    });
    await proto.program.methods
      .closeAgentEscrow()
      .accountsPartial({
        owner: emptySetup.owner.publicKey,
        agentEscrow: emptySetup.escrowPda,
      })
      .signers([emptySetup.owner])
      .rpc();
    const closed = await ctx.provider.context.banksClient.getAccount(
      emptySetup.escrowPda
    );
    assert.isTrue(closed === null || closed.data.length === 0);
  });

  it("rejects creation with invalid caps", async () => {
    const ctx = await setupTestContext();
    const proto = await initializeProtocolForAgents(ctx);
    const owner = Keypair.generate();
    await fundNodeOwnerInBankrun(ctx, owner, ctx.underlyingTokenMint);
    const escrowPda = findAgentEscrowPda(
      proto.program.programId,
      owner.publicKey,
      0
    );

    try {
      await proto.program.methods
        .createAgentEscrow({
          agentIndex: new BN(0),
          authority: Keypair.generate().publicKey,
          maxSigners: 0,
          epochCap: new BN(INIT.epochCap),
          expiresAt: new BN(0),
        })
        .accountsPartial({
          owner: owner.publicKey,
          protocolConfig: proto.protocolConfigPda,
          agentEscrow: escrowPda,
          systemProgram: SystemProgram.programId,
        })
        .signers([owner])
        .rpc();
      assert.fail("expected InvalidAgentCaps");
    } catch (e: any) {
      assert.include(e.toString(), "InvalidAgentCaps");
    }
  });
});
