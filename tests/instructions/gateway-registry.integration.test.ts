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
  "MLPHd2Pv9D4WhGUyh3FJYGvd4rR9SMProkSNcXm7zh9"
);

const PROTOCOL_CONFIG_SEED = Buffer.from("molpha_config");
const REGISTRY_STATE_SEED = Buffer.from("molpha_registry");
const GATEWAY_SEED = Buffer.from("molpha_gateway");

function seedSystemSigner(ctx: TestContext, signer: Keypair): void {
  ctx.provider.context.setAccount(signer.publicKey, {
    lamports: 1_000_000_000,
    owner: SystemProgram.programId,
    executable: false,
    data: Buffer.alloc(0),
  });
}

function findGatewayPda(
  programId: PublicKey,
  ip: number[],
  port: number
): PublicKey {
  const ipBuf = Buffer.from(ip);
  const portBuf = Buffer.alloc(2);
  portBuf.writeUInt16LE(port, 0);

  const [pda] = PublicKey.findProgramAddressSync(
    [GATEWAY_SEED, ipBuf, portBuf],
    programId
  );
  return pda;
}

async function initializeProtocol(ctx: TestContext): Promise<{
  program: any;
  protocolConfigPda: PublicKey;
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

  return { program, protocolConfigPda };
}

describe("gateway registry", () => {
  it("allows the admin to add and remove a gateway endpoint", async () => {
    const ctx = await setupTestContext();
    const { program, protocolConfigPda } = await initializeProtocol(ctx);

    const ip = [10, 0, 0, 1];
    const port = 8080;
    const gatewayPda = findGatewayPda(program.programId, ip, port);

    await program.methods
      .addGateway({ ip, port })
      .accountsPartial({
        protocolConfig: protocolConfigPda,
        gateway: gatewayPda,
        authority: ctx.authority.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .rpc();

    const gateway = await program.account.gateway.fetch(gatewayPda);
    assert.deepEqual(gateway.ip as number[], ip);
    assert.equal(gateway.port as number, port);
    assert.isAbove(Number(gateway.registeredAt), 0);

    await program.methods
      .removeGateway({ ip, port })
      .accountsPartial({
        protocolConfig: protocolConfigPda,
        gateway: gatewayPda,
        authority: ctx.authority.publicKey,
      })
      .rpc();

    const closed = await ctx.provider.context.banksClient.getAccount(
      gatewayPda
    );
    assert.isNull(closed);
  });

  it("rejects non-admin callers for add/remove", async () => {
    const ctx = await setupTestContext();
    const { program, protocolConfigPda } = await initializeProtocol(ctx);

    const unauthorized = Keypair.generate();
    seedSystemSigner(ctx, unauthorized);

    const ip = [10, 0, 0, 2];
    const port = 8081;
    const gatewayPda = findGatewayPda(program.programId, ip, port);

    try {
      await program.methods
        .addGateway({ ip, port })
        .accountsPartial({
          protocolConfig: protocolConfigPda,
          gateway: gatewayPda,
          authority: unauthorized.publicKey,
          systemProgram: SystemProgram.programId,
        })
        .signers([unauthorized])
        .rpc();
      assert.fail("expected unauthorised authority error");
    } catch (e: any) {
      assert.include(e.toString(), "UnauthorisedAuthority");
    }

    await program.methods
      .addGateway({ ip, port })
      .accountsPartial({
        protocolConfig: protocolConfigPda,
        gateway: gatewayPda,
        authority: ctx.authority.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .rpc();

    try {
      await program.methods
        .removeGateway({ ip, port })
        .accountsPartial({
          protocolConfig: protocolConfigPda,
          gateway: gatewayPda,
          authority: unauthorized.publicKey,
        })
        .signers([unauthorized])
        .rpc();
      assert.fail("expected unauthorised authority error");
    } catch (e: any) {
      assert.include(e.toString(), "UnauthorisedAuthority");
    }
  });

  it("rejects duplicate gateway registration", async () => {
    const ctx = await setupTestContext();
    const { program, protocolConfigPda } = await initializeProtocol(ctx);

    const ip = [10, 0, 0, 3];
    const port = 8082;
    const gatewayPda = findGatewayPda(program.programId, ip, port);

    await program.methods
      .addGateway({ ip, port })
      .accountsPartial({
        protocolConfig: protocolConfigPda,
        gateway: gatewayPda,
        authority: ctx.authority.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .rpc();

    try {
      await program.methods
        .addGateway({ ip, port })
        .accountsPartial({
          protocolConfig: protocolConfigPda,
          gateway: gatewayPda,
          authority: ctx.authority.publicKey,
          systemProgram: SystemProgram.programId,
        })
        .rpc();
      assert.fail("expected init failure on duplicate PDA");
    } catch (e: any) {
      assert.ok(e);
    }
  });
});
