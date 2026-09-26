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
import { fetchAccountOrNull } from "../lib/accounts";
import { getCliContext } from "../lib/client";
import { withOffenceCoalitionKeys } from "../lib/coalition";
import {
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

function registryNode(registry: any, index: number): PublicKey {
  const value = registry.nodes[index];
  return new PublicKey(Uint8Array.from(value));
}

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
  const evidenceRegistryAccount = await fetchAccountOrNull<any>(
    ctx.program,
    "registry",
    evidenceRegistry,
  );
  if (!evidenceRegistryAccount) {
    throw new Error(`Evidence registry ${evidenceVersion} is missing`);
  }
  const offence = await withOffenceCoalitionKeys(
    ctx.program,
    evidenceRegistryAccount,
    evidence,
  );

  const remaining = remainingBitsFromOffence(offence).map((bit) => ({
    pubkey: registryNode(evidenceRegistryAccount, bit),
    isWritable: true,
    isSigner: false,
  }));

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
    .description("Freeze a node whose liveliness score is below the threshold")
    .option("--node-index <number>", "registry index of the inactive node")
    .option(
      "--node-owner <pubkey>",
      "node owner; used to find the current-registry index",
    )
    .option(
      "--registry-version <number>",
      "defaults to the current registry version",
    )
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const [registryState] = findRegistryStatePda(ctx.programId);
      const state = await fetchAccountOrNull<any>(
        ctx.program,
        "registryState",
        registryState,
      );
      if (!state) throw new Error("Registry state is not initialized");
      const registryVersion =
        options.registryVersion !== undefined
          ? parseInteger(options.registryVersion, "registry version")
          : Number(state.currentVersion);
      const [registryPda] = findRegistryPda(ctx.programId, registryVersion);
      const registry = await fetchAccountOrNull<any>(
        ctx.program,
        "registry",
        registryPda,
      );
      if (!registry) throw new Error(`Registry ${registryVersion} is missing`);

      let nodeIndex: number;
      if (options.nodeIndex !== undefined) {
        nodeIndex = parseInteger(options.nodeIndex, "node index");
      } else if (options.nodeOwner) {
        const owner = parsePublicKey(options.nodeOwner, "node owner");
        const [node] = findNodePda(ctx.programId, owner);
        nodeIndex = -1;
        for (let index = 0; index < Number(registry.nodeCount); index++) {
          if (registryNode(registry, index).equals(node)) {
            nodeIndex = index;
            break;
          }
        }
        if (nodeIndex < 0) {
          throw new Error("Node is not a current registry member");
        }
      } else {
        throw new Error("Missing --node-index or --node-owner");
      }

      const offence = {
        inactivity: {
          registryVersion,
          nodeIndex,
        },
      };
      await sendSlashOffence(
        command,
        offence,
        inactivityEvidenceHash(registryVersion, nodeIndex),
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
