import { BN, Idl, Program } from "@coral-xyz/anchor";
import { assert } from "chai";
import { Keypair, PublicKey, SystemProgram } from "@solana/web3.js";
import {
  ASSOCIATED_TOKEN_PROGRAM_ID,
  TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import { setupTestContext, TestContext } from "../setup";
import {
  fundNodeOwnerInBankrun,
  getProtocolTreasury,
} from "../helpers/evm-submit-data-update";
import molphaIdl from "../../target/idl/molpha.json";

const LOCALNET_PROGRAM_ID = new PublicKey(
  "MoLFeTRpDZgckPjjbLwW1wB9n85bQiqboPnvw9RwoG8"
);

const PROTOCOL_CONFIG_SEED = Buffer.from("molpha_config");
const REGISTRY_STATE_SEED = Buffer.from("molpha_registry");
const GATEWAY_SEED = Buffer.from("molpha_gateway");
const GATEWAY_BOND_MIN = new BN(500_000_000);

function findGatewayPda(programId: PublicKey, authority: PublicKey): PublicKey {
  const [pda] = PublicKey.findProgramAddressSync(
    [GATEWAY_SEED, authority.toBuffer()],
    programId
  );
  return pda;
}

async function initializeProtocol(ctx: TestContext): Promise<{
  program: any;
  protocolConfigPda: PublicKey;
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
      protocolFee: new BN(0),
      disputeWindowSlots: new BN(256),
      gatewayBondMin: GATEWAY_BOND_MIN,
      challengerBountyBps: 500,
      usdcMint: ctx.underlyingTokenMint,
      agentRoundBase: new BN(1_000),
      agentRoundPerSig: new BN(15),
      agentCachedReadPrice: new BN(100),
      maxAgentPerRequestCap: new BN(1_000_000),
      maxAgentEpochCap: new BN(10_000_000),
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

  return { program, protocolConfigPda, treasury };
}

describe("gateway registry", () => {
  it("allows a gateway authority to register with endpoint and bond", async () => {
    const ctx = await setupTestContext();
    const { program, protocolConfigPda, treasury } = await initializeProtocol(
      ctx
    );

    const gatewayAuthority = Keypair.generate();
    const authorityUsdc = await fundNodeOwnerInBankrun(
      ctx,
      gatewayAuthority,
      ctx.underlyingTokenMint,
      BigInt(1_000_000_000)
    );

    const ip = [10, 0, 0, 1];
    const port = 8080;
    const gatewayPda = findGatewayPda(
      program.programId,
      gatewayAuthority.publicKey
    );

    await program.methods
      .registerGateway({
        ip,
        port,
        bondAmount: GATEWAY_BOND_MIN,
      })
      .accountsPartial({
        gatewayAuthority: gatewayAuthority.publicKey,
        protocolConfig: protocolConfigPda,
        gateway: gatewayPda,
        usdcMint: ctx.underlyingTokenMint,
        authorityUsdc,
        treasury,
        systemProgram: SystemProgram.programId,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers([gatewayAuthority])
      .rpc();

    const gateway = await program.account.gateway.fetch(gatewayPda);
    assert.deepEqual(gateway.ip as number[], ip);
    assert.equal(gateway.port as number, port);
    assert.equal(gateway.index as number, 0);
    assert.equal(gateway.bond.toString(), GATEWAY_BOND_MIN.toString());
    assert.equal(
      gateway.authority.toBase58(),
      gatewayAuthority.publicKey.toBase58()
    );
    assert.isAbove(Number(gateway.registeredAt), 0);
  });

  it("rejects registration below the minimum bond", async () => {
    const ctx = await setupTestContext();
    const { program, protocolConfigPda, treasury } = await initializeProtocol(
      ctx
    );

    const gatewayAuthority = Keypair.generate();
    const authorityUsdc = await fundNodeOwnerInBankrun(
      ctx,
      gatewayAuthority,
      ctx.underlyingTokenMint
    );
    const gatewayPda = findGatewayPda(
      program.programId,
      gatewayAuthority.publicKey
    );

    try {
      await program.methods
        .registerGateway({
          ip: [10, 0, 0, 2],
          port: 8081,
          bondAmount: GATEWAY_BOND_MIN.sub(new BN(1)),
        })
        .accountsPartial({
          gatewayAuthority: gatewayAuthority.publicKey,
          protocolConfig: protocolConfigPda,
          gateway: gatewayPda,
          usdcMint: ctx.underlyingTokenMint,
          authorityUsdc,
          treasury,
          systemProgram: SystemProgram.programId,
          tokenProgram: TOKEN_PROGRAM_ID,
        })
        .signers([gatewayAuthority])
        .rpc();
      assert.fail("expected insufficient bond error");
    } catch (e: any) {
      assert.include(e.toString(), "InsufficientBond");
    }
  });

  it("rejects duplicate gateway registration", async () => {
    const ctx = await setupTestContext();
    const { program, protocolConfigPda, treasury } = await initializeProtocol(
      ctx
    );

    const gatewayAuthority = Keypair.generate();
    const authorityUsdc = await fundNodeOwnerInBankrun(
      ctx,
      gatewayAuthority,
      ctx.underlyingTokenMint
    );
    const gatewayPda = findGatewayPda(
      program.programId,
      gatewayAuthority.publicKey
    );
    const args = {
      ip: [10, 0, 0, 3],
      port: 8082,
      bondAmount: GATEWAY_BOND_MIN,
    };

    await program.methods
      .registerGateway(args)
      .accountsPartial({
        gatewayAuthority: gatewayAuthority.publicKey,
        protocolConfig: protocolConfigPda,
        gateway: gatewayPda,
        usdcMint: ctx.underlyingTokenMint,
        authorityUsdc,
        treasury,
        systemProgram: SystemProgram.programId,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers([gatewayAuthority])
      .rpc();

    try {
      await program.methods
        .registerGateway(args)
        .accountsPartial({
          gatewayAuthority: gatewayAuthority.publicKey,
          protocolConfig: protocolConfigPda,
          gateway: gatewayPda,
          usdcMint: ctx.underlyingTokenMint,
          authorityUsdc,
          treasury,
          systemProgram: SystemProgram.programId,
          tokenProgram: TOKEN_PROGRAM_ID,
        })
        .signers([gatewayAuthority])
        .rpc();
      assert.fail("expected init failure on duplicate PDA");
    } catch (e: any) {
      assert.ok(e);
    }
  });
});
