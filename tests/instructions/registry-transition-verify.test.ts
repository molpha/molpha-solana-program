import * as anchor from "@coral-xyz/anchor";
import { BN } from "@coral-xyz/anchor";
import { assert } from "chai";
import { PublicKey, SystemProgram } from "@solana/web3.js";
import {
  ASSOCIATED_TOKEN_PROGRAM_ID,
  TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import { setupTestContext } from "../setup";
import {
  buildEvmSubmitDataUpdateArgs,
  expectedRegistryIndex,
  getProtocolTreasury,
  seedEvmJobAndFeed,
  seedEvmRemoveSwapGraceState,
  signerRemainingAccounts,
  VIRTUAL_INDEX,
} from "../helpers/evm-submit-data-update";

const PROTOCOL_CONFIG_SEED = Buffer.from("molpha_config");
const REGISTRY_STATE_SEED = Buffer.from("molpha_registry");

async function initializeRegistry(
  ctx: Awaited<ReturnType<typeof setupTestContext>>
) {
  const program = ctx.molphaProgram as any;
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
      previousGraceSecs: new BN(3600),
      withdrawalCooldownSecs: new BN(5),
      maxNodeCount: 32,
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

  return { program, protocolConfigPda, registryStatePda };
}

describe("registry transition verify (RegistryIndex-only + VIRTUAL_INDEX)", () => {
  let ctx: Awaited<ReturnType<typeof setupTestContext>>;
  let program: any;
  let registryStatePda: PublicKey;
  let protocolConfigPda: PublicKey;

  beforeAll(async () => {
    ctx = await setupTestContext();
    ({ program, protocolConfigPda, registryStatePda } =
      await initializeRegistry(ctx));
  });

  it("expectedRegistryIndex remaps removed and moved slots", () => {
    const transition = {
      removedOldIndex: 1,
      transitionType: "removeSwap" as const,
      movedOldIndex: 7,
    };
    assert.equal(expectedRegistryIndex(1, transition), VIRTUAL_INDEX);
    assert.equal(expectedRegistryIndex(7, transition), 1);
    assert.equal(expectedRegistryIndex(0, transition), 0);
  });
});
