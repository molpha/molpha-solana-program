import * as anchor from "@coral-xyz/anchor";
import { BN } from "@coral-xyz/anchor";
import { assert } from "chai";
import {
  ComputeBudgetProgram,
  PublicKey,
  SystemProgram,
} from "@solana/web3.js";
import {
  ASSOCIATED_TOKEN_PROGRAM_ID,
  TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import { setupTestContext } from "../setup";
import { signNodePop } from "../../cli/src/lib/signing/node-pop";
import {
  buildRegisterNodeArgs,
  deriveNodeId,
  EVM_FIXTURE_SOURCE_ID,
  EVM_FIXTURE_VALUE,
  evmBitmapBitSet,
  findNodePda,
  findRegistryPda,
  findSecp256k1KeyClaimPda,
  fundNodeOwnerInBankrun,
  generateE2eSignerMaterial,
  getProtocolTreasury,
  registerNodesForE2e,
  signE2eDataUpdate,
} from "../helpers/submit-data-update";

const CONFIG_SEED = Buffer.from("molpha_config");
const REGISTRY_STATE_SEED = Buffer.from("molpha_registry");

function registryNode(registry: any, index: number): PublicKey {
  return new PublicKey(Uint8Array.from(registry.nodes[index]));
}

describe("immutable versioned node registry", () => {
  it("preserves old snapshots and verifies them after policy and membership changes", async () => {
    const ctx = await setupTestContext();
    const program = ctx.molphaProgram as anchor.Program;
    const [protocolConfigPda] = PublicKey.findProgramAddressSync(
      [CONFIG_SEED],
      program.programId
    );
    const [registryStatePda] = PublicKey.findProgramAddressSync(
      [REGISTRY_STATE_SEED],
      program.programId
    );
    const registry0 = findRegistryPda(program.programId, 0);
    const treasury = getProtocolTreasury(
      protocolConfigPda,
      ctx.underlyingTokenMint
    );

    try {
      await program.methods
        .initialize({
          withdrawalCooldownSecs: new BN(5),
          maxNodeCount: 256,
          redundancyBuffer: 2,
          minimumNodeDeposit: new BN(100_000_000),
          rewardPerSignature: new BN(10),
          gatewayFee: new BN(20),
          receiptSubmitterBonus: new BN(5),
          protocolFeeBps: 100,
          maxReceiptDelaySlots: new BN(256),
          protocolFee: new BN(30),
          disputeWindowSlots: new BN(1),
          gatewayBondMin: new BN(500_000_000),
          challengerBountyBps: 500,
          usdcMint: ctx.underlyingTokenMint,
          agentRoundBase: new BN(1_000),
          agentRoundPerSig: new BN(15),
          agentEpochLenSecs: new BN(86_400),
        })
        .accountsPartial({
          protocolConfig: protocolConfigPda,
          registryState: registryStatePda,
          registry: registry0,
          usdcMint: ctx.underlyingTokenMint,
          treasury,
          authority: ctx.authority.publicKey,
          tokenProgram: TOKEN_PROGRAM_ID,
          associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
        })
        .rpc();
    } catch (error) {
      throw new Error(`initialize failed: ${String(error)}`);
    }

    const genesisRaw = await ctx.provider.context.banksClient.getAccount(
      registry0
    );
    assert.equal(genesisRaw?.data.length, 8_208);
    const genesis = await (program.account as any).registry.fetch(registry0);
    assert.equal(Number(genesis.version), 0);
    assert.equal(Number(genesis.nodeCount), 0);
    assert.equal(genesis.redundancyBuffer, 2);

    const allSignerMaterial = generateE2eSignerMaterial(4);
    const signerMaterial = {
      privKeys: allSignerMaterial.privKeys.slice(0, 3),
      compressedPubkeys: allSignerMaterial.compressedPubkeys.slice(0, 3),
    };
    try {
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
        signerMaterial,
        async () => undefined
      );
    } catch (error) {
      throw new Error(`registration failed: ${String(error)}`);
    }

    const registry3Pda = findRegistryPda(program.programId, 3);
    const registry3RawBefore =
      await ctx.provider.context.banksClient.getAccount(registry3Pda);
    const registry3 = await (program.account as any).registry.fetch(
      registry3Pda
    );
    assert.equal(Number(registry3.nodeCount), 3);
    for (let index = 0; index < 3; index++) {
      assert.equal(
        registryNode(registry3, index).toBase58(),
        findNodePda(program.programId, ctx.nodes[index].publicKey).toBase58()
      );
    }

    const skippedRegistry = findRegistryPda(program.programId, 9);
    try {
      await program.methods
        .updateRegistryPolicy(9, 1)
        .accountsPartial({
          authority: ctx.authority.publicKey,
          protocolConfig: protocolConfigPda,
          registryState: registryStatePda,
          currentRegistry: registry3Pda,
          newRegistry: skippedRegistry,
          systemProgram: SystemProgram.programId,
        })
        .rpc();
      assert.fail("skipped registry version must fail");
    } catch (error) {
      assert.include(String(error), "InvalidRegistryVersion");
    }
    assert.isNull(
      await ctx.provider.context.banksClient.getAccount(skippedRegistry)
    );

    const registry4Pda = findRegistryPda(program.programId, 4);
    try {
      await program.methods
        .updateRegistryPolicy(4, 2)
        .accountsPartial({
          authority: ctx.authority.publicKey,
          protocolConfig: protocolConfigPda,
          registryState: registryStatePda,
          currentRegistry: registry3Pda,
          newRegistry: registry4Pda,
          systemProgram: SystemProgram.programId,
        })
        .rpc();
      assert.fail("equal registry policy must fail");
    } catch (error) {
      assert.include(String(error), "RegistryPolicyUnchanged");
    }
    assert.isNull(
      await ctx.provider.context.banksClient.getAccount(registry4Pda)
    );
    try {
      await program.methods
        .updateRegistryPolicy(4, 1)
        .accountsPartial({
          authority: ctx.authority.publicKey,
          protocolConfig: protocolConfigPda,
          registryState: registryStatePda,
          currentRegistry: registry3Pda,
          newRegistry: registry4Pda,
          systemProgram: SystemProgram.programId,
        })
        .rpc();
    } catch (error) {
      throw new Error(`policy update failed: ${String(error)}`);
    }
    const registry4 = await (program.account as any).registry.fetch(
      registry4Pda
    );
    assert.equal(Number(registry4.nodeCount), 3);
    assert.equal(registry4.redundancyBuffer, 1);
    assert.equal(registry3.redundancyBuffer, 2);

    const removedNode = findNodePda(program.programId, ctx.nodes[1].publicKey);
    const registry5Pda = findRegistryPda(program.programId, 5);
    try {
      await program.methods
        .deactivateNode({
          nodeOwner: ctx.nodes[1].publicKey,
          removeIndex: 0,
          newRegistryVersion: 5,
        })
        .accountsPartial({
          authority: ctx.nodes[1].publicKey,
          protocolConfig: protocolConfigPda,
          registryState: registryStatePda,
          currentRegistry: registry4Pda,
          newRegistry: registry5Pda,
          node: removedNode,
          systemProgram: SystemProgram.programId,
        })
        .signers([ctx.nodes[1]])
        .rpc();
      assert.fail("wrong removal index must fail");
    } catch (error) {
      assert.include(String(error), "InvalidNodeAccount");
    }
    assert.isNull(
      await ctx.provider.context.banksClient.getAccount(registry5Pda)
    );
    try {
      await program.methods
        .deactivateNode({
          nodeOwner: ctx.nodes[1].publicKey,
          removeIndex: 1,
          newRegistryVersion: 5,
        })
        .accountsPartial({
          authority: ctx.nodes[1].publicKey,
          protocolConfig: protocolConfigPda,
          registryState: registryStatePda,
          currentRegistry: registry4Pda,
          newRegistry: registry5Pda,
          node: removedNode,
          systemProgram: SystemProgram.programId,
        })
        .signers([ctx.nodes[1]])
        .rpc();
    } catch (error) {
      throw new Error(`deactivation failed: ${String(error)}`);
    }

    const registry5 = await (program.account as any).registry.fetch(
      registry5Pda
    );
    assert.equal(Number(registry5.nodeCount), 2);
    assert.equal(
      registryNode(registry5, 1).toBase58(),
      findNodePda(program.programId, ctx.nodes[2].publicKey).toBase58()
    );
    const removed = await (program.account as any).node.fetch(removedNode);
    assert.property(removed.status, "deactivated");

    const addedOwner = ctx.nodes[3];
    const addedCompressed = allSignerMaterial.compressedPubkeys[3];
    const addedSalt = Uint8Array.from(Buffer.alloc(32, 4));
    const addedNodeId = deriveNodeId(
      addedOwner.publicKey,
      addedCompressed,
      addedSalt
    );
    const addedPop = signNodePop({
      programId: program.programId,
      nodeId: addedNodeId,
      secp256k1PrivateKey: Uint8Array.from(
        Buffer.from(
          allSignerMaterial.privKeys[3].toString(16).padStart(64, "0"),
          "hex"
        )
      ),
    });
    const addedNode = findNodePda(program.programId, addedOwner.publicKey);
    const addedOwnerUsdc = await fundNodeOwnerInBankrun(
      ctx,
      addedOwner,
      ctx.underlyingTokenMint
    );
    const registry6Pda = findRegistryPda(program.programId, 6);
    await program.methods
      .registerNode(
        buildRegisterNodeArgs({
          newIndex: 5,
          registrationSalt: addedSalt,
          secp256k1PubkeyCompressed: addedCompressed,
          popSigR: addedPop.popSigR,
          popSigS: addedPop.popSigS,
        })
      )
      .accountsPartial({
        authority: ctx.authority.publicKey,
        nodeOwner: addedOwner.publicKey,
        protocolConfig: protocolConfigPda,
        registryState: registryStatePda,
        currentRegistry: registry5Pda,
        newRegistry: registry6Pda,
        node: addedNode,
        secp256K1KeyClaim: findSecp256k1KeyClaimPda(
          program.programId,
          addedCompressed
        ),
        usdcMint: ctx.underlyingTokenMint,
        ownerUsdc: addedOwnerUsdc,
        treasury,
        systemProgram: SystemProgram.programId,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .preInstructions([
        ComputeBudgetProgram.setComputeUnitLimit({ units: 450_000 }),
      ])
      .signers([addedOwner])
      .rpc();

    await program.methods
      .updateNodeEndpoint({ ip: [10, 0, 0, 7], port: 9443 })
      .accountsPartial({
        nodeOwner: ctx.nodes[0].publicKey,
        node: findNodePda(program.programId, ctx.nodes[0].publicKey),
      })
      .signers([ctx.nodes[0]])
      .rpc();

    const registry3RawAfter = await ctx.provider.context.banksClient.getAccount(
      registry3Pda
    );
    assert.deepEqual(registry3RawAfter?.data, registry3RawBefore?.data);
    assert.deepEqual(
      (await ctx.provider.context.banksClient.getAccount(registry0))?.data,
      genesisRaw?.data
    );

    const signed = signE2eDataUpdate(
      3,
      signerMaterial,
      3,
      EVM_FIXTURE_SOURCE_ID,
      {
        value: EVM_FIXTURE_VALUE,
        canonicalTimestamp: 1_700_000_123,
      },
      1,
      3,
      2
    );
    const historicalAccounts = [];
    for (let bit = 0; bit < 256; bit++) {
      if (evmBitmapBitSet(signed.signersBitmap, bit)) {
        historicalAccounts.push({
          pubkey: registryNode(registry3, bit),
          isSigner: false,
          isWritable: false,
        });
      }
    }
    try {
      await program.methods
        .verifyDataUpdate(signed.submitArgs)
        .accountsPartial({ registry: registry3Pda })
        .remainingAccounts(historicalAccounts)
        .preInstructions([
          ComputeBudgetProgram.setComputeUnitLimit({ units: 1_400_000 }),
        ])
        .rpc();
    } catch (error) {
      throw new Error(`historical verification failed: ${String(error)}`);
    }

    for (const malformed of [
      [...historicalAccounts].reverse(),
      [...historicalAccounts, historicalAccounts[0]],
    ]) {
      try {
        await program.methods
          .verifyDataUpdate(signed.submitArgs)
          .accountsPartial({ registry: registry3Pda })
          .remainingAccounts(malformed)
          .rpc();
        assert.fail("malformed historical Node account list must fail");
      } catch (error) {
        assert.match(String(error), /MissingSignerAccount|InvalidNodeAccount/);
      }
    }

    try {
      await program.methods
        .submitDataUpdate(signed.submitArgs)
        .accountsPartial({
          submitter: ctx.authority.publicKey,
          registryState: registryStatePda,
          registry: registry3Pda,
          systemProgram: SystemProgram.programId,
        })
        .remainingAccounts(historicalAccounts)
        .rpc();
      assert.fail("historical registry must not mutate the live feed");
    } catch (error) {
      assert.include(String(error), "InvalidRegistryVersion");
    }

    const state = await (program.account as any).registryState.fetch(
      registryStatePda
    );
    assert.equal(Number(state.currentVersion), 6);
  });
});
