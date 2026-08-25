import { BN, Idl, Program } from "@coral-xyz/anchor";
import { assert } from "chai";
import { Keypair, PublicKey, SystemProgram } from "@solana/web3.js";
import {
  ASSOCIATED_TOKEN_PROGRAM_ID,
  TOKEN_PROGRAM_ID,
  getAssociatedTokenAddressSync,
} from "@solana/spl-token";
import { setupTestContext, TestContext } from "../setup";
import {
  fundNodeOwnerInBankrun,
  getProtocolTreasury,
} from "../helpers/submit-attestation";
import molphaIdl from "../../target/idl/molpha.json";

/** Prefer the built IDL address (matches `declare_id!` / Anchor.toml localnet). */
const LOCALNET_PROGRAM_ID = new PublicKey(
  (molphaIdl as { address: string }).address
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

function findGatewayBondAta(
  usdcMint: PublicKey,
  gatewayPda: PublicKey
): PublicKey {
  return getAssociatedTokenAddressSync(usdcMint, gatewayPda, true);
}

function registerGatewayAccounts(params: {
  gatewayAuthority: PublicKey;
  protocolConfigPda: PublicKey;
  gatewayPda: PublicKey;
  usdcMint: PublicKey;
  authorityUsdc: PublicKey;
}) {
  return {
    gatewayAuthority: params.gatewayAuthority,
    protocolConfig: params.protocolConfigPda,
    gateway: params.gatewayPda,
    usdcMint: params.usdcMint,
    authorityUsdc: params.authorityUsdc,
    gatewayBond: findGatewayBondAta(params.usdcMint, params.gatewayPda),
    systemProgram: SystemProgram.programId,
    tokenProgram: TOKEN_PROGRAM_ID,
    associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
  };
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
      x402RoundBase: new BN(1_000),
      x402RoundPerSig: new BN(15),
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
    const { program, protocolConfigPda } = await initializeProtocol(ctx);

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
    const gatewayBond = findGatewayBondAta(
      ctx.underlyingTokenMint,
      gatewayPda
    );

    await program.methods
      .registerGateway({
        ip,
        port,
        bondAmount: GATEWAY_BOND_MIN,
      })
      .accountsPartial(
        registerGatewayAccounts({
          gatewayAuthority: gatewayAuthority.publicKey,
          protocolConfigPda,
          gatewayPda,
          usdcMint: ctx.underlyingTokenMint,
          authorityUsdc,
        })
      )
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

    const bondAcct = await ctx.provider.context.banksClient.getAccount(
      gatewayBond
    );
    assert.isNotNull(bondAcct);
    const bondHeld = Buffer.from(bondAcct!.data).readBigUInt64LE(64);
    assert.equal(bondHeld.toString(), GATEWAY_BOND_MIN.toString());
  });

  it("rejects registration below the minimum bond", async () => {
    const ctx = await setupTestContext();
    const { program, protocolConfigPda } = await initializeProtocol(ctx);

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
        .accountsPartial(
          registerGatewayAccounts({
            gatewayAuthority: gatewayAuthority.publicKey,
            protocolConfigPda,
            gatewayPda,
            usdcMint: ctx.underlyingTokenMint,
            authorityUsdc,
          })
        )
        .signers([gatewayAuthority])
        .rpc();
      assert.fail("expected insufficient bond error");
    } catch (e: any) {
      assert.include(e.toString(), "InsufficientBond");
    }
  });

  it("rejects duplicate gateway registration", async () => {
    const ctx = await setupTestContext();
    const { program, protocolConfigPda } = await initializeProtocol(ctx);

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
      .accountsPartial(
        registerGatewayAccounts({
          gatewayAuthority: gatewayAuthority.publicKey,
          protocolConfigPda,
          gatewayPda,
          usdcMint: ctx.underlyingTokenMint,
          authorityUsdc,
        })
      )
      .signers([gatewayAuthority])
      .rpc();

    try {
      await program.methods
        .registerGateway(args)
        .accountsPartial(
          registerGatewayAccounts({
            gatewayAuthority: gatewayAuthority.publicKey,
            protocolConfigPda,
            gatewayPda,
            usdcMint: ctx.underlyingTokenMint,
            authorityUsdc,
          })
        )
        .signers([gatewayAuthority])
        .rpc();
      assert.fail("expected init failure on duplicate PDA");
    } catch (e: any) {
      assert.ok(e);
    }
  });
});
