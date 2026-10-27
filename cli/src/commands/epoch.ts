import { TOKEN_PROGRAM_ID } from "@solana/spl-token";
import {
  ComputeBudgetProgram,
  Ed25519Program,
  PublicKey,
  SystemProgram,
  SYSVAR_INSTRUCTIONS_PUBKEY,
} from "@solana/web3.js";
import { Command } from "commander";
import { parseSchnorrSignatureJson } from "../lib/agg-sig";
import { fetchAccountOrNull } from "../lib/accounts";
import { CliContext, getCliContext } from "../lib/client";
import { coalitionKeyForSigners } from "../lib/coalition";
import { fetchRegistry } from "../lib/registry";
import {
  findEpochPda,
  findGatewayPda,
  findProtocolConfigPda,
  findRegistryPda,
  findTicketMarkerPda,
  findTreasuryAta,
} from "../lib/pdas";
import {
  buildAttestation,
  computeMessageHash,
  deriveSignersBitmap,
} from "../lib/signing/attestation";
import {
  creditableSlots,
  deriveDispatchMessage,
  deriveTicketHash,
  deriveTicketId,
  epochWeight,
  isTicket,
  rosterSlots,
} from "../lib/ticket";
import {
  bn,
  bytesToHex,
  hexToBytes,
  loadKeypair,
  parseInteger,
  parseBitList,
  parsePublicKey,
  writeJson,
} from "../lib/utils";

/** Weighted slots credited per `credit_epoch_rewards` transaction. */
export const CREDIT_BATCH = 12;

/** Optional compute-unit price (micro-lamports per unit) for a transaction. */
export type SendOptions = { priorityMicroLamports?: number };

function priorityInstructions(options: SendOptions) {
  return options.priorityMicroLamports
    ? [
        ComputeBudgetProgram.setComputeUnitPrice({
          microLamports: options.priorityMicroLamports,
        }),
      ]
    : [];
}

/** `finalize_epoch`: freeze the epoch's weights and create the epoch two ahead. */
export async function sendFinalizeEpoch(
  ctx: CliContext,
  index: number,
  options: SendOptions = {},
): Promise<string> {
  const [protocolConfig] = findProtocolConfigPda(ctx.programId);
  return (ctx.program.methods as any)
    .finalizeEpoch(bn(index, "epoch"))
    .accountsPartial({
      payer: ctx.wallet.publicKey,
      protocolConfig,
      target: findEpochPda(ctx.programId, index)[0],
      next: findEpochPda(ctx.programId, index + 1)[0],
      successor: findEpochPda(ctx.programId, index + 2)[0],
      systemProgram: SystemProgram.programId,
    })
    .preInstructions(priorityInstructions(options))
    .rpc();
}

/** `credit_epoch_rewards` for the given roster slots, in batches. Returns the signatures. */
export async function sendCreditEpoch(
  ctx: CliContext,
  index: number,
  address: PublicKey,
  roster: Array<number[] | Uint8Array>,
  slots: number[],
  options: SendOptions = {},
): Promise<string[]> {
  const [protocolConfig] = findProtocolConfigPda(ctx.programId);
  const signatures: string[] = [];
  for (let i = 0; i < slots.length; i += CREDIT_BATCH) {
    const batch = slots.slice(i, i + CREDIT_BATCH);
    signatures.push(
      await (ctx.program.methods as any)
        .creditEpochRewards(bn(index, "epoch"), batch)
        .accountsPartial({ protocolConfig, epochState: address })
        .remainingAccounts(
          batch.map((slot) => ({
            pubkey: new PublicKey(Uint8Array.from(roster[slot])),
            isSigner: false,
            isWritable: true,
          })),
        )
        .preInstructions(priorityInstructions(options))
        .rpc(),
    );
  }
  return signatures;
}

/** `close_epoch`: returns the epoch's rent to its payer and rolls its residual into `open`. */
export async function sendCloseEpoch(
  ctx: CliContext,
  index: number,
  open: number,
  rentPayer: PublicKey,
  options: SendOptions = {},
): Promise<string> {
  return (ctx.program.methods as any)
    .closeEpoch(bn(index, "epoch"), bn(open, "open epoch"))
    .accountsPartial({
      epochState: findEpochPda(ctx.programId, index)[0],
      openEpochState: findEpochPda(ctx.programId, open)[0],
      rentPayer,
    })
    .preInstructions(priorityInstructions(options))
    .rpc();
}

/** `sweep_treasury_revenue`: recognize unswept treasury income in the open epoch. */
export async function sendSweepTreasury(
  ctx: CliContext,
  open: number,
  options: SendOptions = {},
): Promise<string> {
  const { address: protocolConfig, config } = await fetchProtocolConfig(ctx);
  return (ctx.program.methods as any)
    .sweepTreasuryRevenue(bn(open, "epoch"))
    .accountsPartial({
      protocolConfig,
      treasury: findTreasuryAta(ctx.programId, config.usdcMint),
      epochState: findEpochPda(ctx.programId, open)[0],
      tokenProgram: TOKEN_PROGRAM_ID,
    })
    .preInstructions(priorityInstructions(options))
    .rpc();
}

async function fetchEpoch(ctx: CliContext, epoch: number) {
  const [address] = findEpochPda(ctx.programId, epoch);
  const state = await fetchAccountOrNull<any>(
    ctx.program,
    "epochState",
    address,
  );
  if (!state)
    throw new Error(`Epoch ${epoch} account ${address.toBase58()} not found`);
  return { address, state };
}

async function fetchProtocolConfig(ctx: CliContext) {
  const [address] = findProtocolConfigPda(ctx.programId);
  const config = await fetchAccountOrNull<any>(
    ctx.program,
    "protocolConfig",
    address,
  );
  if (!config) throw new Error("Protocol config is not initialized");
  return { address, config };
}

async function clusterTime(ctx: CliContext): Promise<number> {
  const slot = await ctx.connection.getSlot();
  const time = await ctx.connection.getBlockTime(slot);
  if (time === null) throw new Error("cluster time unavailable");
  return time;
}

/** Find the epoch open at cluster time among retained epoch accounts. */
export async function findOpenEpoch(
  ctx: CliContext,
  from: number,
): Promise<number> {
  const now = await clusterTime(ctx);
  const accounts = await (ctx.program.account as any).epochState.all();
  const open = accounts
    .map(({ account }: { account: any }) => account)
    .filter(
      (state: any) =>
        Number(state.epoch) >= from &&
        state.finalized === 0 &&
        Number(state.start) <= now &&
        now < Number(state.end),
    );
  if (open.length === 1) return Number(open[0].epoch);
  if (open.length > 1)
    throw new Error(`Multiple epochs appear open at cluster time ${now}`);
  throw new Error(
    `No open epoch at cluster time ${now} from ${from}; check the epoch lifecycle`,
  );
}

function epochSummary(epoch: number, address: PublicKey, state: any) {
  const slots = Array.from({ length: Number(state.rosterLen) }, (_, slot) => ({
    slot,
    node: new PublicKey(Uint8Array.from(state.roster[slot])).toBase58(),
    signed: state.signed[slot],
    selected: state.selected[slot],
    weight: epochWeight(state, slot).toString(),
  }));
  return {
    epoch,
    address: address.toBase58(),
    start: state.start.toString(),
    end: state.end.toString(),
    ticketGrace: state.ticketGrace.toString(),
    minSigners: state.minSigners,
    tau: state.tau.toString(),
    pool: state.pool.toString(),
    paid: state.paid.toString(),
    tickets: state.tickets,
    finalized: state.finalized !== 0,
    weightTotal: state.weightTotal.toString(),
    weightedCount: state.weightedCount,
    creditedCount: state.creditedCount,
    rentPayer: new PublicKey(Uint8Array.from(state.rentPayer)).toBase58(),
    roster: slots,
  };
}

export function registerEpochCommands(program: Command): void {
  const epoch = program
    .command("epoch")
    .description("Epoch reward pool lifecycle");

  epoch
    .command("show")
    .description(
      "Show an epoch's window, pool, threshold and per-node counters",
    )
    .requiredOption("--epoch <number>", "epoch index")
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const index = parseInteger(options.epoch, "epoch");
      const { address, state } = await fetchEpoch(ctx, index);
      writeJson(epochSummary(index, address, state));
    });

  epoch
    .command("finalize")
    .description(
      "Freeze an epoch's weights after its ticket deadline and create the epoch two ahead",
    )
    .requiredOption("--epoch <number>", "epoch index")
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const index = parseInteger(options.epoch, "epoch");
      const signature = await sendFinalizeEpoch(ctx, index);
      writeJson({ signature, epoch: index, created: index + 2 });
    });

  epoch
    .command("credit")
    .description("Credit finalized epoch shares to node reward balances")
    .requiredOption("--epoch <number>", "epoch index")
    .option(
      "--slots <list>",
      "comma-separated roster slots; defaults to every weighted, uncredited slot",
    )
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const index = parseInteger(options.epoch, "epoch");
      const { address, state } = await fetchEpoch(ctx, index);
      if (state.finalized === 0)
        throw new Error(`Epoch ${index} is not finalized`);
      const slots = options.slots
        ? parseBitList(options.slots, "slots")
        : creditableSlots(state);
      const signatures = await sendCreditEpoch(
        ctx,
        index,
        address,
        state.roster,
        slots,
      );
      writeJson({ epoch: index, credited: slots, signatures });
    });

  epoch
    .command("close")
    .description(
      "Close a fully credited epoch after its evidence window; its residual joins the open epoch",
    )
    .requiredOption("--epoch <number>", "epoch index")
    .option("--open-epoch <number>", "epoch open now; found automatically")
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const index = parseInteger(options.epoch, "epoch");
      const { address, state } = await fetchEpoch(ctx, index);
      const open =
        options.openEpoch !== undefined
          ? parseInteger(options.openEpoch, "open epoch")
          : await findOpenEpoch(ctx, index + 1);
      const rentPayer = new PublicKey(Uint8Array.from(state.rentPayer));
      const signature = await sendCloseEpoch(ctx, index, open, rentPayer);
      writeJson({
        signature,
        epoch: index,
        rolledInto: open,
        residual: (
          BigInt(state.pool.toString()) - BigInt(state.paid.toString())
        ).toString(),
        rentPayer: rentPayer.toBase58(),
      });
    });

  const ticket = program
    .command("ticket")
    .description("Submit sampled rounds as participation tickets");

  ticket
    .command("submit")
    .description(
      "Submit a round whose aggregate falls under its epoch's ticket threshold",
    )
    .requiredOption("--source-id <hex>", "32-byte source identifier")
    .requiredOption("--timestamp <number>", "timestamp, unix milliseconds")
    .requiredOption("--signatures-required <number>", "quorum threshold")
    .requiredOption("--registry-version <number>", "registry version")
    .requiredOption("--value <hex>", "32-byte signed value")
    .requiredOption("--agg-sig <path>", "JSON SchnorrSignature file")
    .requiredOption(
      "--epoch <number>",
      "epoch containing the timestamp (its whole second)",
    )
    .requiredOption(
      "--gateway-authority <pubkey>",
      "authority of the gateway that dispatched the round",
    )
    .option(
      "--dispatch-sig <hex>",
      "64-byte Ed25519 dispatch signature by the gateway authority",
    )
    .option(
      "--gateway-wallet <path>",
      "gateway authority keypair, to sign the dispatch locally (testing)",
    )
    .option("--force", "submit even if the local lottery check fails", false)
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const { address: protocolConfig } = await fetchProtocolConfig(ctx);
      const epochIndex = parseInteger(options.epoch, "epoch");
      const { address: epochState, state } = await fetchEpoch(ctx, epochIndex);

      const sourceId = hexToBytes(options.sourceId, 32, "source id");
      const timestamp = parseInteger(options.timestamp, "timestamp");
      const signaturesRequired = parseInteger(
        options.signaturesRequired,
        "signatures required",
      );
      const registryVersion = parseInteger(
        options.registryVersion,
        "registry version",
      );
      const value = hexToBytes(options.value, 32, "value");
      const [registryPda] = findRegistryPda(ctx.programId, registryVersion);
      const registry = await fetchRegistry(ctx.program, registryPda);
      if (!registry) throw new Error(`Registry ${registryVersion} is missing`);

      const schnorr = parseSchnorrSignatureJson(options.aggSig);
      if (!schnorr.aggSigS || !schnorr.commitment) {
        throw new Error("--agg-sig must carry aggSigS and commitment");
      }
      const attestation = buildAttestation({
        value,
        sourceId,
        registryVersion,
        signaturesRequired,
        timestamp,
        ...schnorr,
      });
      const messageHash = computeMessageHash({
        sourceId,
        registryVersion,
        signaturesRequired,
        signersBitmap: schnorr.signersBitmap,
        value,
        timestamp,
      });
      const ticketHash = deriveTicketHash(
        messageHash,
        schnorr.aggSigS,
        schnorr.commitment,
      );
      const tau = BigInt(state.tau.toString());
      if (!isTicket(ticketHash, tau) && !options.force) {
        throw new Error(
          `Round is not a ticket: hash prefix above epoch ${epochIndex} threshold ${tau}`,
        );
      }

      const gatewayAuthority = parsePublicKey(
        options.gatewayAuthority,
        "gateway authority",
      );
      const dispatch = deriveDispatchMessage(
        ctx.programId,
        sourceId,
        signaturesRequired,
        registryVersion,
        timestamp,
      );
      let dispatchIx;
      if (options.dispatchSig) {
        dispatchIx = Ed25519Program.createInstructionWithPublicKey({
          publicKey: gatewayAuthority.toBytes(),
          message: dispatch,
          signature: hexToBytes(options.dispatchSig, 64, "dispatch signature"),
        });
      } else if (options.gatewayWallet) {
        const gateway = loadKeypair(options.gatewayWallet);
        if (!gateway.publicKey.equals(gatewayAuthority)) {
          throw new Error(
            "--gateway-wallet does not match --gateway-authority",
          );
        }
        dispatchIx = Ed25519Program.createInstructionWithPrivateKey({
          privateKey: gateway.secretKey,
          message: dispatch,
        });
      } else {
        throw new Error("Pass --dispatch-sig or --gateway-wallet");
      }

      const selection = deriveSignersBitmap(
        sourceId,
        registryVersion,
        Number(registry.nodeCount),
        signaturesRequired,
        Number(registry.redundancyBuffer),
        timestamp,
        registry.members,
      );
      const slots = rosterSlots(state, registry, selection);
      const coalitionKey = coalitionKeyForSigners(
        registry,
        schnorr.signersBitmap,
      );
      const ticketId = deriveTicketId(sourceId, signaturesRequired, timestamp);
      const [ticketMarker] = findTicketMarkerPda(ctx.programId, ticketId);

      const signature = await (ctx.program.methods as any)
        .submitTicket(bn(epochIndex, "epoch"), attestation, slots, coalitionKey)
        .accountsPartial({
          submitter: ctx.wallet.publicKey,
          protocolConfig,
          registry: registryPda,
          epochState,
          gateway: findGatewayPda(ctx.programId, gatewayAuthority)[0],
          ticketMarker,
          instructionsSysvar: SYSVAR_INSTRUCTIONS_PUBKEY,
          systemProgram: SystemProgram.programId,
        })
        // The dispatch verification must sit immediately before the ticket instruction.
        .preInstructions([
          ComputeBudgetProgram.setComputeUnitLimit({ units: 600_000 }),
          dispatchIx,
        ])
        .rpc();

      writeJson({
        signature,
        epoch: epochIndex,
        ticketId: bytesToHex(ticketId),
        ticketHash: bytesToHex(ticketHash),
        ticketMarker: ticketMarker.toBase58(),
        rosterSlots: slots,
      });
    });

  ticket
    .command("close")
    .description(
      "Return a ticket marker's rent once its epoch no longer accepts tickets",
    )
    .option("--marker <pubkey>", "ticket marker address")
    .option("--ticket-id <hex>", "32-byte ticket id")
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const marker = options.marker
        ? parsePublicKey(options.marker, "ticket marker")
        : findTicketMarkerPda(
            ctx.programId,
            hexToBytes(options.ticketId ?? "", 32, "ticket id"),
          )[0];
      const state = await fetchAccountOrNull<any>(
        ctx.program,
        "ticketMarker",
        marker,
      );
      if (!state)
        throw new Error(`Ticket marker ${marker.toBase58()} not found`);
      const signature = await (ctx.program.methods as any)
        .closeTicketMarker()
        .accountsPartial({ ticketMarker: marker, submitter: state.submitter })
        .rpc();
      writeJson({
        signature,
        ticketMarker: marker.toBase58(),
        submitter: state.submitter.toBase58(),
      });
    });

  const revenue = program
    .command("revenue")
    .description(
      "Sweep treasury income into the open epoch's pool and manage the reserve",
    );

  revenue
    .command("sweep-treasury")
    .description(
      "Recognize treasury income (x402 payments, subscriptions) in the open epoch",
    )
    .option("--epoch <number>", "epoch open now; found automatically")
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const open =
        options.epoch !== undefined
          ? parseInteger(options.epoch, "epoch")
          : await findOpenEpoch(ctx, 0);
      const signature = await sendSweepTreasury(ctx, open);
      writeJson({ signature, epoch: open });
    });

  revenue
    .command("withdraw-reserve")
    .description(
      "Withdraw protocol fees and slashing proceeds (protocol authority only)",
    )
    .requiredOption(
      "--amount <amount>",
      "USDC base units, at most protocol_reserved",
    )
    .requiredOption("--destination <pubkey>", "USDC token account")
    .action(async (options, command: Command) => {
      const ctx = await getCliContext(command);
      const { address: protocolConfig, config } =
        await fetchProtocolConfig(ctx);
      const signature = await (ctx.program.methods as any)
        .withdrawProtocolReserve(bn(options.amount, "amount"))
        .accountsPartial({
          authority: ctx.wallet.publicKey,
          protocolConfig,
          treasury: findTreasuryAta(ctx.programId, config.usdcMint),
          destination: parsePublicKey(options.destination, "destination"),
          tokenProgram: TOKEN_PROGRAM_ID,
        })
        .rpc();
      writeJson({ signature, amount: options.amount });
    });
}
