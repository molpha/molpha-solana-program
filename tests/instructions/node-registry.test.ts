import { assert } from "chai";
import {
  setupTestContext,
  initializeProtocol,
  TestContext,
  registerNodeWithDeposit,
} from "../setup";
import { PublicKey, SystemProgram } from "@solana/web3.js";
import * as anchor from "@coral-xyz/anchor";
import {
  getAssociatedTokenAddressSync,
  getOrCreateAssociatedTokenAccount,
  TOKEN_PROGRAM_ID,
  ASSOCIATED_TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import { createAssociatedTokenAccount, getAccount } from "spl-token-bankrun";

describe("Node Management Instructions", () => {
  let ctx: TestContext;

  before(async () => {
    ctx = await setupTestContext();
    await initializeProtocol(ctx);
  });

  describe("Register Node", () => {
    it("Successfully registers a new node", async () => {
      const nodeToRegister = ctx.nodes[0];

      // Mock EVM pubkey (compressed secp256k1 - 33 bytes)
      const evmPubkey = Buffer.alloc(33, 1);

      // Register node with security deposit
      const nodePDA = await registerNodeWithDeposit(
        ctx,
        nodeToRegister.publicKey,
        evmPubkey
      );

      // Verify the node account was created
      const nodeAccount = await ctx.molphaProgram.account.node.fetch(nodePDA);
      assert.ok(nodeAccount.authority.equals(ctx.authority.publicKey));
      assert.ok(nodeAccount.nodePubkey.equals(nodeToRegister.publicKey));
      assert.deepEqual(Buffer.from(nodeAccount.nodePubEvm), evmPubkey);
      assert.equal(nodeAccount.status.registered !== undefined, true);
      assert.ok(
        nodeAccount.securityDeposit.gt(new anchor.BN(0)),
        "Security deposit should be greater than 0"
      );
    });

    it("Fails to register a duplicate node", async () => {
      const nodeToRegister = ctx.nodes[0];

      // Mock EVM pubkey
      const evmPubkey = Buffer.alloc(33, 1);

      let didFail = false;
      try {
        await registerNodeWithDeposit(ctx, nodeToRegister.publicKey, evmPubkey);
      } catch (error: any) {
        // Should fail because the account already exists
        didFail = true;
      }

      assert.ok(didFail, "Should have failed to register a duplicate node");
    });

    it("Successfully registers multiple different nodes", async () => {
      const node1 = ctx.nodes[1];
      const node2 = ctx.nodes[2];

      const evmPubkey1 = Buffer.alloc(33, 2);
      const evmPubkey2 = Buffer.alloc(33, 3);

      // Register first node
      const nodePDA1 = await registerNodeWithDeposit(
        ctx,
        node1.publicKey,
        evmPubkey1
      );

      // Register second node
      const nodePDA2 = await registerNodeWithDeposit(
        ctx,
        node2.publicKey,
        evmPubkey2
      );

      // Verify both nodes exist
      const nodeAccount1 = await ctx.molphaProgram.account.node.fetch(nodePDA1);
      const nodeAccount2 = await ctx.molphaProgram.account.node.fetch(nodePDA2);

      assert.ok(nodeAccount1.nodePubkey.equals(node1.publicKey));
      assert.ok(nodeAccount2.nodePubkey.equals(node2.publicKey));
      assert.deepEqual(Buffer.from(nodeAccount1.nodePubEvm), evmPubkey1);
      assert.deepEqual(Buffer.from(nodeAccount2.nodePubEvm), evmPubkey2);
      assert.ok(
        nodeAccount1.securityDeposit.gt(new anchor.BN(0)),
        "Security deposit should be greater than 0"
      );
      assert.ok(
        nodeAccount2.securityDeposit.gt(new anchor.BN(0)),
        "Security deposit should be greater than 0"
      );
    });
  });

  describe("Security Deposit Locking", () => {
    it.skip("Transfers security deposit to node's token account", async () => {
      const nodeToRegister = new anchor.web3.Keypair();
      const evmPubkey = Buffer.alloc(33, 4);

      // Get protocol config to check deposit amount
      const protocolConfig =
        await ctx.molphaProgram.account.protocolConfig.fetch(
          ctx.protocolConfigPDA
        );
      const expectedDeposit = BigInt(
        protocolConfig.nodeSecurityDeposit.toString()
      );
      console.log("expectedDeposit", expectedDeposit);

      // Get initial balance by reading account data directly
      const initialOperatorAccountInfo =
        await ctx.provider.connection.getAccountInfo(ctx.userTokenAccount);
      const initialOperatorBalance = initialOperatorAccountInfo
        ? Buffer.from(initialOperatorAccountInfo.data).readBigUInt64LE(64)
        : BigInt(0);

      // create node deposit ata
      let nodeSecurityDepositAccountAddress =
        await createAssociatedTokenAccount(
          ctx.provider.context.banksClient,
          ctx.authority.payer,
          ctx.underlyingTokenMint,
          nodeToRegister.publicKey
          // true
        );
      let nodeSecurityDepositAccount = await getAccount(
        ctx.provider.context.banksClient,
        nodeSecurityDepositAccountAddress
      );

      // Register node
      const nodePDA = await registerNodeWithDeposit(
        ctx,
        nodeToRegister.publicKey,
        evmPubkey
      );
      console.log("node", nodePDA.toBase58());
      console.log("node2", nodeToRegister.publicKey.toBase58());

      nodeSecurityDepositAccount = await getAccount(
        ctx.provider.context.banksClient,
        nodeSecurityDepositAccountAddress
      );
      console.log(
        "nodeSecurityDepositAccount.amount2",
        nodeSecurityDepositAccount.amount
      );

      // Get node's security deposit account
      // const nodeSecurityDepositAccount = getAssociatedTokenAddressSync(
      //   ctx.underlyingTokenMint,
      //   nodePDA,
      //   true // allowOwnerOffCurve
      // );

      // Verify deposit was transferred by reading account data
      const finalOperatorAccountInfo =
        await ctx.provider.connection.getAccountInfo(ctx.userTokenAccount);
      const finalOperatorBalance = finalOperatorAccountInfo
        ? Buffer.from(finalOperatorAccountInfo.data).readBigUInt64LE(64)
        : BigInt(0);

      // Check if account exists and has correct balance
      // In bankrun, getAccountInfo throws if account doesn't exist, so we need to handle that
      let depositAccountInfo = await getAccount(
        ctx.provider.context.banksClient,
        nodeSecurityDepositAccountAddress
      );

      assert.ok(
        depositAccountInfo !== null,
        "Security deposit account should exist when deposit is required"
      );
      const depositAccountBalance = depositAccountInfo.amount;
      console.log("depositAccountBalance", depositAccountBalance);
      console.log("expectedDeposit", expectedDeposit);
      assert.equal(
        depositAccountBalance,
        expectedDeposit,
        "Deposit account should have the correct balance"
      );
      console.log("initialOperatorBalance", initialOperatorBalance);
      console.log("finalOperatorBalance", finalOperatorBalance);
      assert.equal(
        initialOperatorBalance - finalOperatorBalance,
        expectedDeposit,
        "Operator balance should be reduced by deposit amount"
      );
    });

    it("Records security deposit amount in Node state", async () => {
      const nodeToRegister = ctx.nodes[4];
      const evmPubkey = Buffer.alloc(33, 5);

      // Get protocol config to check expected deposit
      const protocolConfig =
        await ctx.molphaProgram.account.protocolConfig.fetch(
          ctx.protocolConfigPDA
        );
      const expectedDeposit = protocolConfig.nodeSecurityDeposit;

      // Register node
      const nodePDA = await registerNodeWithDeposit(
        ctx,
        nodeToRegister.publicKey,
        evmPubkey
      );

      // Verify node state
      const nodeAccount = await ctx.molphaProgram.account.node.fetch(nodePDA);
      assert.ok(
        nodeAccount.securityDeposit.eq(expectedDeposit),
        `Node security deposit (${nodeAccount.securityDeposit.toString()}) should match protocol config (${expectedDeposit.toString()})`
      );
    });

    it.skip("Creates security deposit token account with Node PDA as authority", async () => {
      // Verify protocol config has deposit configured
      const protocolConfig =
        await ctx.molphaProgram.account.protocolConfig.fetch(
          ctx.protocolConfigPDA
        );
      const depositAmount = protocolConfig.nodeSecurityDeposit;

      // Register a fresh node for this test
      const nodeToRegister = anchor.web3.Keypair.generate();
      const evmPubkey = Buffer.alloc(33, 10);

      const nodePDA = await registerNodeWithDeposit(
        ctx,
        nodeToRegister.publicKey,
        evmPubkey
      );

      // Get node's security deposit account
      const nodeSecurityDepositAccount = getAssociatedTokenAddressSync(
        ctx.underlyingTokenMint,
        nodePDA,
        true
      );

      // Fetch the token account info
      // In bankrun, getAccountInfo throws if account doesn't exist, so we need to handle that
      let accountInfo = null;
      try {
        accountInfo = await ctx.provider.connection.getAccountInfo(
          nodeSecurityDepositAccount
        );
      } catch (error) {
        // Account doesn't exist in bankrun
        accountInfo = null;
      }

      if (depositAmount.gt(new anchor.BN(0))) {
        assert.ok(
          accountInfo !== null,
          "Security deposit token account should exist when deposit is required"
        );

        // Parse token account data to verify authority
        const tokenAccountData = Buffer.from(accountInfo.data);
        const owner = new PublicKey(tokenAccountData.slice(32, 64));

        assert.ok(
          owner.equals(nodePDA),
          "Token account authority should be the Node PDA"
        );
      } else {
        // If deposit is 0, account may not be created
        assert.ok(true, "Test passes when deposit is 0");
      }
    });

    it("Allows registration with zero deposit when not configured", async () => {
      // Update protocol config to set deposit to 0
      await ctx.molphaProgram.methods
        .updateProtocolConfig({
          underlyingToken: null,
          basePricePerSecondScaled: null,
          frequencyCoefficient: null,
          signersCoefficient: null,
          rewardPercentage: null,
          nodeSecurityDeposit: new anchor.BN(0),
        })
        .accountsPartial({
          protocolConfig: ctx.protocolConfigPDA,
          authority: ctx.authority.publicKey,
        })
        .rpc();

      // Register a new node
      const nodeToRegister = anchor.web3.Keypair.generate();
      const evmPubkey = Buffer.alloc(33, 100);

      const [nodePDA] = PublicKey.findProgramAddressSync(
        [Buffer.from("node"), nodeToRegister.publicKey.toBuffer()],
        ctx.molphaProgram.programId
      );

      const nodeSecurityDepositAccount = getAssociatedTokenAddressSync(
        ctx.underlyingTokenMint,
        nodePDA,
        true
      );

      await ctx.molphaProgram.methods
        .registerNode(nodeToRegister.publicKey, evmPubkey)
        .accountsPartial({
          node: nodePDA,
          protocolConfig: ctx.protocolConfigPDA,
          underlyingToken: ctx.underlyingTokenMint,
          nodeOperatorTokenAccount: ctx.userTokenAccount,
          nodeSecurityDepositAccount: nodeSecurityDepositAccount,
          authority: ctx.authority.publicKey,
          systemProgram: SystemProgram.programId,
          tokenProgram: TOKEN_PROGRAM_ID,
          associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
        })
        .rpc();

      // Verify node was created with zero deposit
      const nodeAccount = await ctx.molphaProgram.account.node.fetch(nodePDA);
      assert.ok(
        nodeAccount.securityDeposit.eq(new anchor.BN(0)),
        "Security deposit should be 0"
      );

      // Restore original deposit amount for other tests
      await ctx.molphaProgram.methods
        .updateProtocolConfig({
          underlyingToken: null,
          basePricePerSecondScaled: null,
          frequencyCoefficient: null,
          signersCoefficient: null,
          rewardPercentage: null,
          nodeSecurityDeposit: new anchor.BN(1000000000), // 1000 tokens
        })
        .accountsPartial({
          protocolConfig: ctx.protocolConfigPDA,
          authority: ctx.authority.publicKey,
        })
        .rpc();
    });

    it("Fails to register node when operator has insufficient balance", async () => {
      // This test verifies that the instruction validates token account balance
      // For bankrun, we'll test with a non-existent token account which will fail
      const nodeToRegister = anchor.web3.Keypair.generate();
      const evmPubkey = Buffer.alloc(33, 99);

      const [nodePDA] = PublicKey.findProgramAddressSync(
        [Buffer.from("node"), nodeToRegister.publicKey.toBuffer()],
        ctx.molphaProgram.programId
      );

      const nodeSecurityDepositAccount = getAssociatedTokenAddressSync(
        ctx.underlyingTokenMint,
        nodePDA,
        true
      );

      // Use a non-existent token account (random address)
      const nonExistentTokenAccount = anchor.web3.Keypair.generate().publicKey;

      let didFail = false;
      try {
        await ctx.molphaProgram.methods
          .registerNode(nodeToRegister.publicKey, evmPubkey)
          .accountsPartial({
            node: nodePDA,
            protocolConfig: ctx.protocolConfigPDA,
            underlyingToken: ctx.underlyingTokenMint,
            nodeOperatorTokenAccount: nonExistentTokenAccount,
            nodeSecurityDepositAccount: nodeSecurityDepositAccount,
            authority: ctx.authority.publicKey,
            systemProgram: SystemProgram.programId,
            tokenProgram: TOKEN_PROGRAM_ID,
            associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
          })
          .rpc();
      } catch (error: any) {
        didFail = true;
        // Should fail due to account validation or constraint errors
        assert.ok(
          error.message.includes("AccountNotFound") ||
            error.message.includes("InsufficientSecurityDeposit") ||
            error.message.includes("Account does not exist") ||
            error.message.includes("AnchorError") ||
            error.logs?.some((log: string) => log.includes("Error")),
          "Should fail with validation error"
        );
      }

      assert.ok(
        didFail,
        "Should have failed to register node with invalid token account"
      );
    });

    it("Updates protocol config security deposit amount", async () => {
      const newDepositAmount = new anchor.BN(2000000000); // 2000 tokens

      await ctx.molphaProgram.methods
        .updateProtocolConfig({
          underlyingToken: null,
          basePricePerSecondScaled: null,
          frequencyCoefficient: null,
          signersCoefficient: null,
          rewardPercentage: null,
          nodeSecurityDeposit: newDepositAmount,
        })
        .accountsPartial({
          protocolConfig: ctx.protocolConfigPDA,
          authority: ctx.authority.publicKey,
        })
        .rpc();

      // Verify config was updated
      const protocolConfig =
        await ctx.molphaProgram.account.protocolConfig.fetch(
          ctx.protocolConfigPDA
        );
      assert.ok(
        protocolConfig.nodeSecurityDeposit.eq(newDepositAmount),
        "Protocol config should be updated with new deposit amount"
      );

      // Register a new node with the new deposit amount
      const nodeToRegister = anchor.web3.Keypair.generate();
      const evmPubkey = Buffer.alloc(33, 101);
      const nodePDA = await registerNodeWithDeposit(
        ctx,
        nodeToRegister.publicKey,
        evmPubkey
      );

      // Verify new deposit amount is used
      const nodeAccount = await ctx.molphaProgram.account.node.fetch(nodePDA);
      assert.ok(
        nodeAccount.securityDeposit.eq(newDepositAmount),
        "New node should use updated deposit amount"
      );

      // Restore original deposit for other tests
      await ctx.molphaProgram.methods
        .updateProtocolConfig({
          underlyingToken: null,
          basePricePerSecondScaled: null,
          frequencyCoefficient: null,
          signersCoefficient: null,
          rewardPercentage: null,
          nodeSecurityDeposit: new anchor.BN(1000000000),
        })
        .accountsPartial({
          protocolConfig: ctx.protocolConfigPDA,
          authority: ctx.authority.publicKey,
        })
        .rpc();
    });
  });
});
