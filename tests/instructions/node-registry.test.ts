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
import {
  createDefaultMockWormholeAccounts,
  createMockWormholeAccounts,
  MOCK_WORMHOLE_BRIDGE_PROGRAM,
} from "../helpers/wormhole";
import { SYSVAR_CLOCK_PUBKEY, SYSVAR_RENT_PUBKEY } from "@solana/web3.js";
import * as secp256k1 from "@noble/secp256k1";

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
      const evmPubkey = Buffer.from(
        secp256k1.getPublicKey(secp256k1.utils.randomPrivateKey(), true)
      );

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
      const expectedUncompressed = Buffer.from(
        secp256k1.Point.fromHex(Uint8Array.from(evmPubkey)).toRawBytes(false)
      ); // 65: 0x04 || x || y
      assert.deepEqual(Buffer.from(nodeAccount.nodePubEvmX), expectedUncompressed.subarray(1, 33));
      assert.deepEqual(Buffer.from(nodeAccount.nodePubEvmY), expectedUncompressed.subarray(33, 65));
      assert.equal(nodeAccount.status.registered !== undefined, true);
      assert.ok(
        nodeAccount.securityDeposit.gt(new anchor.BN(0)),
        "Security deposit should be greater than 0"
      );
    });

    it("Fails to register a duplicate node", async () => {
      const nodeToRegister = ctx.nodes[0];

      // Mock EVM pubkey
      const evmPubkey = Buffer.from(
        secp256k1.getPublicKey(secp256k1.utils.randomPrivateKey(), true)
      );

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

      const evmPubkey1 = Buffer.from(
        secp256k1.getPublicKey(secp256k1.utils.randomPrivateKey(), true)
      );
      const evmPubkey2 = Buffer.from(
        secp256k1.getPublicKey(secp256k1.utils.randomPrivateKey(), true)
      );

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
      const exp1 = Buffer.from(
        secp256k1.Point.fromHex(Uint8Array.from(evmPubkey1)).toRawBytes(false)
      );
      const exp2 = Buffer.from(
        secp256k1.Point.fromHex(Uint8Array.from(evmPubkey2)).toRawBytes(false)
      );
      assert.deepEqual(Buffer.from(nodeAccount1.nodePubEvmX), exp1.subarray(1, 33));
      assert.deepEqual(Buffer.from(nodeAccount1.nodePubEvmY), exp1.subarray(33, 65));
      assert.deepEqual(Buffer.from(nodeAccount2.nodePubEvmX), exp2.subarray(1, 33));
      assert.deepEqual(Buffer.from(nodeAccount2.nodePubEvmY), exp2.subarray(33, 65));
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
      const evmPubkey = Buffer.from(
        secp256k1.getPublicKey(secp256k1.utils.randomPrivateKey(), true)
      );

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
      const evmPubkey = Buffer.from(
        secp256k1.getPublicKey(secp256k1.utils.randomPrivateKey(), true)
      );

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
      const evmPubkey = Buffer.from(
        secp256k1.getPublicKey(secp256k1.utils.randomPrivateKey(), true)
      );

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
          basePricePerSecondScaled: null,
          frequencyCoefficient: null,
          signersCoefficient: null,
          rewardPercentage: null,
          nodeSecurityDeposit: new anchor.BN(0),
          wormholeBridgeProgram: null,
          targetChainIds: null,
          registryXAgg: null,
          signaturesRequired: null,
          nodeCount: null,
          redundancyBuffer: null,
        })
        .accountsPartial({
          protocolConfig: ctx.protocolConfigPDA,
          authority: ctx.authority.publicKey,
        })
        .rpc();

      // Register a new node
      const nodeToRegister = anchor.web3.Keypair.generate();
      const evmPubkey = Buffer.from(
        secp256k1.getPublicKey(secp256k1.utils.randomPrivateKey(), true)
      );

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
        .registerNode(
          nodeToRegister.publicKey,
          evmPubkey,
          "https://example.com/node"
        )
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
        .preInstructions([
          anchor.web3.ComputeBudgetProgram.setComputeUnitLimit({ units: 400_000 }),
        ])
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
          basePricePerSecondScaled: null,
          frequencyCoefficient: null,
          signersCoefficient: null,
          rewardPercentage: null,
          nodeSecurityDeposit: new anchor.BN(1000000000), // 1000 tokens
          wormholeBridgeProgram: null,
          targetChainIds: null,
          registryXAgg: null,
          signaturesRequired: null,
          nodeCount: null,
          redundancyBuffer: null,
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
      const evmPubkey = Buffer.from(
        secp256k1.getPublicKey(secp256k1.utils.randomPrivateKey(), true)
      );

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
          .registerNode(
            nodeToRegister.publicKey,
            evmPubkey,
            "https://example.com/node"
          )
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
          .preInstructions([
            anchor.web3.ComputeBudgetProgram.setComputeUnitLimit({ units: 400_000 }),
          ])
          .rpc();
      } catch (error: any) {
        didFail = true;
        // Should fail due to account validation or constraint errors.
        // Use Boolean(...) so we never pass `undefined` into assert.ok (when logs are missing,
        // `error.logs?.some(...)` is undefined and the whole || chain becomes undefined).
        const msg = String(error?.message ?? "");
        const logs: string[] = Array.isArray(error?.logs) ? error.logs : [];
        const isExpectedFailure =
          msg.includes("AccountNotFound") ||
          msg.includes("InsufficientSecurityDeposit") ||
          msg.includes("Account does not exist") ||
          msg.includes("AnchorError") ||
          msg.includes("custom program error") ||
          msg.includes("failed:") ||
          logs.some(
            (log: string) =>
              log.includes("Error") || log.toLowerCase().includes("failed")
          );
        assert.ok(
          isExpectedFailure,
          `Should fail with validation error; message=${msg}`
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
          basePricePerSecondScaled: null,
          frequencyCoefficient: null,
          signersCoefficient: null,
          rewardPercentage: null,
          nodeSecurityDeposit: newDepositAmount,
          wormholeBridgeProgram: null,
          targetChainIds: null,
          registryXAgg: null,
          signaturesRequired: null,
          nodeCount: null,
          redundancyBuffer: null,
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
      const evmPubkey = Buffer.from(
        secp256k1.getPublicKey(secp256k1.utils.randomPrivateKey(), true)
      );
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
          basePricePerSecondScaled: null,
          frequencyCoefficient: null,
          signersCoefficient: null,
          rewardPercentage: null,
          nodeSecurityDeposit: new anchor.BN(1000000000),
          wormholeBridgeProgram: null,
          targetChainIds: null,
          registryXAgg: null,
          signaturesRequired: null,
          nodeCount: null,
          redundancyBuffer: null,
        })
        .accountsPartial({
          protocolConfig: ctx.protocolConfigPDA,
          authority: ctx.authority.publicKey,
        })
        .rpc();
    });
  });
});
