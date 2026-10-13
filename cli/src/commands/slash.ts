import {
  ASSOCIATED_TOKEN_PROGRAM_ID,
  createAssociatedTokenAccountIdempotentInstruction,
  getAssociatedTokenAddressSync,
  TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import {
  ComputeBudgetProgram,
  PublicKey,
  SystemProgram,
} from "@solana/web3.js";
import { Command } from "commander";
import { registryNode } from "../lib/signing/attestation";
import { fetchAccountOrNull } from "../lib/accounts";
import { getCliContext } from "../lib/client";
import { withOffenceCoalitionKeys } from "../lib/coalition";
import { fetchRegistry, registryExtendInstructions } from "../lib/registry";
import {
  findEpochPda,
  findNodeDepositVault,
  findNodePda,
  findProtocolConfigPda,
  findPunishmentCasePda,
  findRegistryPda,
  findRegistryStatePda,
  findTreasuryAta,
} from "../lib/pdas";
import {
  equalBytes,
  inactivityEvidenceHash,
  offenceEvidenceHash,
  offenceVariant,
  registryVersionFromOffence,
  remainingBitsFromOffence,
} from "../lib/slash";
import {
  bytesToHex,
  hexToBytes,
  parseInteger,
  parsePublicKey,
  readJsonFile,
  writeJson,
} from "../lib/utils";

async function sendSlashOffence(
  command: Command,
  evidence: Record<string, unknown>,
  expectedHash?: Uint8Array,
): Promise<void> {
  const ctx = await getCliContext(command);
  const [protocolConfig] = findProtocolConfigPda(ctx.programId);
  const [registryState] = findRegistryStatePda(ctx.programId);
  const config = await fetchAccountOrNull<any>(
    ctx.program,
    "protocolConfig",
    protocolConfig,
  );
  const state = await fetchAccountOrNull<any>(
    ctx.program,
    "registryState",
    registryState,
  );
  if (!config || !state) throw new Error("Protocol is not initialized");

  const currentVersion = Number(state.currentVersion);
  const evidenceVersion = registryVersionFromOffence(evidence);
  const evidenceHash = offenceEvidenceHash(evidence);
  if (expectedHash && !equalBytes(expectedHash, evidenceHash)) {
    throw new Error(
      `--evidence-hash ${bytesToHex(expectedHash)} does not match computed ${bytesToHex(evidenceHash)}`,
    );
  }

  const [evidenceRegistry] = findRegistryPda(ctx.programId, evidenceVersion);
  const [currentRegistry] = findRegistryPda(ctx.programId, currentVersion);
  const [newRegistry] = findRegistryPda(ctx.programId, currentVersion + 1);
  const evidenceRegistryAccount = await fetchRegistry(
    ctx.program,
    evidenceRegistry,
  );
  const currentRegistryAccount = await fetchRegistry(
    ctx.program,
    currentRegistry,
  );
  if (!currentRegistryAccount) {
    throw new Error(`Current registry ${currentVersion} is missing`);
  }
  if (!evidenceRegistryAccount) {
    throw new Error(`Evidence registry ${evidenceVersion} is missing`);
  }
  const offence = withOffenceCoalitionKeys(evidenceRegistryAccount, evidence);

  // The accounts are the guilty nodes, in ascending bit order; the program checks each against
  // the evidence registry's member at that position. Signer keys come from the snapshot.
  const remaining = remainingBitsFromOffence(
    offence,
    evidenceRegistryAccount,
  ).map((bit) => ({
    pubkey: registryNode(evidenceRegistryAccount, bit),
    isWritable: true,
    isSigner: false,
  }));
  // Inactivity is proven from a finalized epoch's counters: the epoch account trails the node.
  const variant = offenceVariant(offence);
  if (variant.kind === "inactivity") {
    remaining.push({
      pubkey: findEpochPda(ctx.programId, Number(variant.epoch))[0],
      isWritable: false,
      isSigner: false,
    });
  }

  const reporterUsdc = getAssociatedTokenAddressSync(
    config.usdcMint,
    ctx.wallet.publicKey,
  );
  const [punishmentCase] = findPunishmentCasePda(ctx.programId, evidenceHash);

  const signature = await (ctx.program.methods as any)
    .slashOffence(offence)
    .accountsPartial({
      reporter: ctx.wallet.publicKey,
      protocolConfig,
      registryState,
      evidenceRegistry,
      currentRegistry,
      newRegistry,
      punishmentCase,
      nodeDepositVault: findNodeDepositVault(ctx.programId, config.usdcMint),
      treasury: findTreasuryAta(ctx.programId, config.usdcMint),
      reporterUsdc,
      tokenProgram: TOKEN_PROGRAM_ID,
      systemProgram: SystemProgram.programId,
    })
    .remainingAccounts(remaining)
    .preInstructions([
      ComputeBudgetProgram.setComputeUnitLimit({ units: 1_400_000 }),
      // The handler copies every current member before it evicts the guilty.
      ...(await registryExtendInstructions(
        ctx.program,
        ctx.wallet.publicKey,
        currentVersion,
        Number(currentRegistryAccount.nodeCount),
      )),
      createAssociatedTokenAccountIdempotentInstruction(
        ctx.wallet.publicKey,
        reporterUsdc,
        ctx.wallet.publicKey,
        config.usdcMint,
        TOKEN_PROGRAM_ID,
        ASSOCIATED_TOKEN_PROGRAM_ID,
      ),
    ])
    .rpc();

  writeJson({
    signature,
    offence,
    evidenceHash: bytesToHex(evidenceHash),
    punishmentCase: punishmentCase.toBase58(),
    registryVersion: currentVersion + 1,
    remainingAccounts: remaining.map((entry) => entry.pubkey.toBase58()),
  });
}

export function registerSlashCommands(program: Command): void {
  const slash = program
    .command("slash")
    .description("File a node offence through slash_offence");

  slash
    .command("inactivity")
    .description(
      "Freeze a node whose availability in a finalized epoch fell below the payout floor",
    )
    .requiredOption("--epoch <number>", "finalized epoch the node missed")
    .requiredOption("--node-owner <pubkey>", "owner of the inactive node")
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const [registryState] = findRegistryStatePda(ctx.programId);
      const state = await fetchAccountOrNull<any>(
        ctx.program,
        "registryState",
        registryState,
      );
      if (!state) throw new Error("Registry state is not initialized");
      const registryVersion = Number(state.currentVersion);
      const [registryPda] = findRegistryPda(ctx.programId, registryVersion);
      const registry = await fetchRegistry(ctx.program, registryPda);
      if (!registry) throw new Error(`Registry ${registryVersion} is missing`);

      const owner = parsePublicKey(options.nodeOwner, "node owner");
      const [node] = findNodePda(ctx.programId, owner);
      let nodeIndex = -1;
      for (let index = 0; index < Number(registry.nodeCount); index++) {
        if (registryNode(registry, index).equals(node)) {
          nodeIndex = index;
          break;
        }
      }
      if (nodeIndex < 0)
        throw new Error("Node is not a current registry member");

      const epoch = parseInteger(options.epoch, "epoch");
      const epochState = await fetchAccountOrNull<any>(
        ctx.program,
        "epochState",
        findEpochPda(ctx.programId, epoch)[0],
      );
      if (!epochState) throw new Error(`Epoch ${epoch} is missing`);
      let rosterSlot = -1;
      for (let slot = 0; slot < Number(epochState.rosterLen); slot++) {
        if (
          new PublicKey(Uint8Array.from(epochState.roster[slot])).equals(node)
        ) {
          rosterSlot = slot;
          break;
        }
      }
      if (rosterSlot < 0)
        throw new Error(`Node is not on epoch ${epoch}'s roster`);

      const offence = {
        inactivity: { epoch, rosterSlot, registryVersion, nodeIndex },
      };
      await sendSlashOffence(
        command,
        offence,
        inactivityEvidenceHash(epoch, rosterSlot),
      );
    });

  slash
    .command("offence")
    .description(
      "File any offence from a JSON argument matching the on-chain Offence enum",
    )
    .requiredOption(
      "--args <path>",
      "JSON Offence: { inactivity | equivocation | unsolicitedSigning | commitmentReuse | nonceReuse }",
    )
    .option(
      "--evidence-hash <hex>",
      "optional 32-byte evidence hash; must match the digest computed from --args",
    )
    .action(async (options, command: Command) => {
      const offence = readJsonFile<Record<string, unknown>>(options.args);
      const expectedHash = options.evidenceHash
        ? hexToBytes(options.evidenceHash, 32, "evidence hash")
        : undefined;
      await sendSlashOffence(command, offence, expectedHash);
    });
}
