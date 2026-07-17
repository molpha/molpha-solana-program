import * as anchor from "@coral-xyz/anchor";
import { BN } from "@coral-xyz/anchor";
import { assert } from "chai";
import { Point } from "@noble/secp256k1";
import { keccak_256 } from "@noble/hashes/sha3";
import {
  ComputeBudgetProgram,
  PublicKey,
  SystemProgram,
} from "@solana/web3.js";
import {
  ASSOCIATED_TOKEN_PROGRAM_ID,
  TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import { Clock } from "solana-bankrun";
import { signNodePop } from "../../cli/src/lib/signing/node-pop";
import { setupTestContext, TestContext } from "../setup";
import {
  buildRegisterNodeArgs,
  deriveNodeId,
  findNodeOperatorPda,
  findNodePda,
  findSecp256k1KeyClaimPda,
  fundNodeOwnerInBankrun,
  getProtocolTreasury,
  VIRTUAL_INDEX,
} from "../helpers/submit-data-update";

const PROTOCOL_CONFIG_SEED = Buffer.from("molpha_config");
const REGISTRY_STATE_SEED = Buffer.from("molpha_registry");

function scalar(value: number): Uint8Array {
  const out = new Uint8Array(32);
  out[31] = value;
  return out;
}

function compressedForSecret(secret: Uint8Array): Uint8Array {
  return Point.BASE.multiply(
    BigInt(`0x${Buffer.from(secret).toString("hex")}`)
  ).toRawBytes(true);
}

async function advanceClock(ctx: TestContext, seconds: number): Promise<void> {
  const current = await ctx.provider.context.banksClient.getClock();
  ctx.provider.context.setClock(
    new Clock(
      current.slot + BigInt(seconds),
      current.epochStartTimestamp,
      current.epoch,
      current.leaderScheduleEpoch,
      current.unixTimestamp + BigInt(seconds)
    )
  );
}

async function initializeRegistry(ctx: TestContext) {
  const program = ctx.molphaProgram as anchor.Program;
  const [protocolConfigPda] = PublicKey.findProgramAddressSync(
    [PROTOCOL_CONFIG_SEED],
    program.programId
  );
  const [registryStatePda] = PublicKey.findProgramAddressSync(
    [REGISTRY_STATE_SEED],
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
  await advanceClock(ctx, 5);
  return { program, protocolConfigPda, registryStatePda, treasury };
}

describe("register_node cryptographic validation", () => {
  it("accepts a valid PoP and rejects malformed, false, rogue, and duplicate identities", async () => {
    const ctx = await setupTestContext();
    const { program, protocolConfigPda, registryStatePda, treasury } =
      await initializeRegistry(ctx);
    const secret = scalar(7);
    const compressed = compressedForSecret(secret);

    const submitRegistration = async (params: {
      ownerIndex: number;
      newIndex: number;
      secret: Uint8Array;
      compressed: Uint8Array;
      salt: Uint8Array;
      suppliedNodeId?: Uint8Array;
      corruptS?: boolean;
    }) => {
      const owner = ctx.nodes[params.ownerIndex];
      const ownerUsdc = await fundNodeOwnerInBankrun(
        ctx,
        owner,
        ctx.underlyingTokenMint
      );
      const expectedNodeId = deriveNodeId(
        owner.publicKey,
        params.compressed,
        params.salt
      );
      const suppliedNodeId = params.suppliedNodeId ?? expectedNodeId;
      const pop = signNodePop({
        programId: program.programId,
        nodeId: suppliedNodeId,
        secp256k1PrivateKey: params.secret,
        nonce: scalar(19 + params.ownerIndex),
      });
      if (params.corruptS) pop.popSigS[31] ^= 1;

      return program.methods
        .registerNode(
          buildRegisterNodeArgs({
            newIndex: params.newIndex,
            registrationSalt: params.salt,
            secp256k1PubkeyCompressed: params.compressed,
            popSigR: pop.popSigR,
            popSigS: pop.popSigS,
          }),
        )
        .accountsPartial({
          authority: ctx.authority.publicKey,
          nodeOwner: owner.publicKey,
          protocolConfig: protocolConfigPda,
          registryState: registryStatePda,
          nodeOperator: findNodeOperatorPda(program.programId, owner.publicKey),
          node: findNodePda(program.programId, params.newIndex),
          secp256K1KeyClaim: findSecp256k1KeyClaimPda(
            program.programId,
            params.compressed
          ),
          usdcMint: ctx.underlyingTokenMint,
          ownerUsdc,
          treasury,
          systemProgram: SystemProgram.programId,
          tokenProgram: TOKEN_PROGRAM_ID,
        })
        .preInstructions([
          ComputeBudgetProgram.setComputeUnitLimit({ units: 400_000 }),
        ])
        .signers([owner])
        .rpc();
    };

    try {
      await submitRegistration({
        ownerIndex: 0,
        newIndex: 0,
        secret,
        compressed,
        salt: new Uint8Array(32),
      });
      assert.fail("zero salt must fail");
    } catch (error) {
      assert.include(String(error), "InvalidRegistrationSalt");
    }

    try {
      await submitRegistration({
        ownerIndex: 0,
        newIndex: 0,
        secret,
        compressed,
        salt: new Uint8Array(32).fill(1),
        corruptS: true,
      });
      assert.fail("invalid PoP must fail");
    } catch (error) {
      assert.include(String(error), "InvalidPoP");
    }

    const malformed = new Uint8Array(33).fill(0xff);
    malformed[0] = 0x02;
    try {
      await submitRegistration({
        ownerIndex: 0,
        newIndex: 0,
        secret,
        compressed: malformed,
        salt: new Uint8Array(32).fill(1),
      });
      assert.fail("off-curve key must fail");
    } catch (error) {
      assert.include(String(error), "PubkeyNotOnCurve");
    }

    let registry = await program.account.registryState.fetch(registryStatePda);
    assert.equal(registry.currentNodeCount, 0);
    assert.isNull(
      await ctx.provider.context.banksClient.getAccount(
        findSecp256k1KeyClaimPda(program.programId, compressed)
      )
    );

    await submitRegistration({
      ownerIndex: 0,
      newIndex: 0,
      secret,
      compressed,
      salt: new Uint8Array(32).fill(1),
    });
    const node = await program.account.node.fetch(
      findNodePda(program.programId, 0)
    );
    const full = Point.fromHex(Buffer.from(compressed)).toRawBytes(false);
    assert.deepEqual(node.secp256K1PubkeyX, Array.from(full.subarray(1, 33)));
    assert.deepEqual(node.secp256K1PubkeyY, Array.from(full.subarray(33, 65)));

    await advanceClock(ctx, 5);
    try {
      await submitRegistration({
        ownerIndex: 1,
        newIndex: 1,
        secret,
        compressed,
        salt: new Uint8Array(32).fill(2),
      });
      assert.fail("active duplicate key must fail");
    } catch (error) {
      assert.include(String(error), "NodeAlreadyRegistered");
    }

    await program.methods
      .deactivateNode()
      .accountsPartial({
        authority: ctx.nodes[0].publicKey,
        protocolConfig: protocolConfigPda,
        registryState: registryStatePda,
        removedOperator: findNodeOperatorPda(
          program.programId,
          ctx.nodes[0].publicKey
        ),
        removedNode: findNodePda(program.programId, 0),
        virtualNode: findNodePda(program.programId, VIRTUAL_INDEX),
        lastNode: ctx.authority.publicKey,
        movedOperator: ctx.authority.publicKey,
        rentReceiver: ctx.authority.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .signers([ctx.nodes[0]])
      .rpc();
    await advanceClock(ctx, 5);

    try {
      await submitRegistration({
        ownerIndex: 1,
        newIndex: 0,
        secret,
        compressed,
        salt: new Uint8Array(32).fill(3),
      });
      assert.fail("deactivated key must remain reserved");
    } catch (error) {
      assert.include(String(error), "NodeAlreadyRegistered");
    }

    registry = await program.account.registryState.fetch(registryStatePda);
    assert.equal(registry.currentNodeCount, 0);
    const claim = await program.account.secp256K1KeyClaim.fetch(
      findSecp256k1KeyClaimPda(program.programId, compressed)
    );
    assert.isTrue(claim.claimed);
    assert.equal(claim.owner.toBase58(), ctx.nodes[0].publicKey.toBase58());
    assert.deepEqual(claim.keyFingerprint, Array.from(keccak_256(compressed)));
  }, 180_000);
});
