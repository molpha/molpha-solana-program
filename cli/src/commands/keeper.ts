import { PublicKey } from "@solana/web3.js";
import { Command } from "commander";
import { CliContext, getCliContext } from "../lib/client";
import {
  epochView,
  KEEPER_DEFAULTS,
  KeeperAction,
  KeeperConfig,
  planKeeper,
} from "../lib/keeper";
import { parseInteger } from "../lib/utils";
import {
  sendCloseEpoch,
  sendCreditEpoch,
  sendFinalizeEpoch,
  sendSweepTreasury,
} from "./epoch";

/** Failures worth retrying on another endpoint; anything else is the transaction's own error. */
const TRANSPORT_ERROR =
  /fetch failed|ECONN|ETIMEDOUT|EAI_AGAIN|ENOTFOUND|socket hang up|timed out|timeout|429|Too Many Requests|502|503|504|Bad Gateway|Service Unavailable|Gateway Time-out|blockhash not found/i;

export function isTransportError(error: unknown): boolean {
  const cause = (error as { cause?: unknown } | null)?.cause;
  const text =
    error instanceof Error
      ? `${error.message} ${cause === undefined ? "" : String(cause)}`
      : String(error);
  return TRANSPORT_ERROR.test(text);
}

/** Runs `fn` against each endpoint in turn, starting from the last that worked. */
export class RpcPool {
  private current = 0;
  constructor(private readonly contexts: CliContext[]) {}

  get primary(): CliContext {
    return this.contexts[0];
  }

  async run<T>(fn: (ctx: CliContext) => Promise<T>): Promise<T> {
    let lastError: unknown;
    for (let i = 0; i < this.contexts.length; i += 1) {
      const index = (this.current + i) % this.contexts.length;
      try {
        const result = await fn(this.contexts[index]);
        this.current = index;
        return result;
      } catch (error) {
        if (!isTransportError(error)) throw error;
        lastError = error;
        log("warn", "rpc-failed", {
          rpc: new URL(this.contexts[index].options.rpcUrl).host,
          error: String((error as Error).message ?? error).slice(0, 200),
        });
      }
    }
    throw lastError;
  }
}

function log(
  level: "info" | "warn" | "error",
  event: string,
  data: object = {},
) {
  process.stdout.write(
    `${JSON.stringify({ ts: new Date().toISOString(), level, event, ...data })}\n`,
  );
}

async function clusterTime(ctx: CliContext): Promise<number> {
  const slot = await ctx.connection.getSlot();
  const time = await ctx.connection.getBlockTime(slot);
  if (time === null) throw new Error("cluster time unavailable");
  return time;
}

export function registerKeeperCommand(program: Command): void {
  program
    .command("keeper")
    .description(
      "Run the permissionless epoch keeper: finalize, credit, close and sweep, with health alerts",
    )
    .option(
      "--once",
      "run one pass and exit (non-zero if the lifecycle is unhealthy)",
      false,
    )
    .option("--interval-seconds <seconds>", "pause between passes", "60")
    .option(
      "--sweep-interval-seconds <seconds>",
      "minimum time between treasury sweeps",
      String(KEEPER_DEFAULTS.sweepIntervalSeconds),
    )
    .option(
      "--runway-seconds <seconds>",
      "warn when the newest epoch ends sooner than this",
      String(KEEPER_DEFAULTS.runwaySeconds),
    )
    .option(
      "--finalize-overdue-seconds <seconds>",
      "alert when an epoch stays unfinalized this long past its ticket deadline",
      String(KEEPER_DEFAULTS.finalizeOverdueSeconds),
    )
    .option(
      "--rpc-fallback <url...>",
      "additional RPC endpoints, tried in order when the primary fails",
    )
    .option(
      "--priority-fee-micro-lamports <price>",
      "compute-unit price attached to every keeper transaction",
    )
    .option(
      "--min-balance-lamports <amount>",
      "warn when the keeper wallet holds less than this",
      "50000000",
    )
    .action(async (options, command: Command) => {
      const contexts = [await getCliContext(command)];
      for (const url of options.rpcFallback ?? []) {
        contexts.push(await getCliContext(command, { rpc: url }));
      }
      const pool = new RpcPool(contexts);
      const send = options.priorityFeeMicroLamports
        ? {
            priorityMicroLamports: parseInteger(
              options.priorityFeeMicroLamports,
              "priority fee",
            ),
          }
        : {};
      const intervalMs =
        parseInteger(options.intervalSeconds, "interval") * 1000;
      const config: KeeperConfig = {
        runwaySeconds: parseInteger(options.runwaySeconds, "runway"),
        finalizeOverdueSeconds: parseInteger(
          options.finalizeOverdueSeconds,
          "finalize overdue",
        ),
        sweepIntervalSeconds: parseInteger(
          options.sweepIntervalSeconds,
          "sweep interval",
        ),
        lastSweepAt: null,
      };
      const minBalance = parseInteger(
        options.minBalanceLamports,
        "min balance",
      );
      log("info", "keeper-started", {
        keeper: pool.primary.wallet.publicKey.toBase58(),
        rpcEndpoints: contexts.length,
        once: Boolean(options.once),
      });

      let stopping = false;
      for (const signal of ["SIGINT", "SIGTERM"] as const) {
        process.once(signal, () => {
          stopping = true;
        });
      }

      let unhealthy = false;
      do {
        try {
          unhealthy = await pass(pool, config, send, minBalance);
        } catch (error) {
          unhealthy = true;
          log("error", "pass-failed", {
            error: String((error as Error).message ?? error),
          });
        }
        if (options.once || stopping) break;
        await new Promise((resolve) => setTimeout(resolve, intervalMs));
      } while (!stopping);
      if (options.once && unhealthy) process.exitCode = 2;
    });
}

/** One keeper pass. Returns true when an error-level health signal was raised. */
async function pass(
  pool: RpcPool,
  config: KeeperConfig,
  send: { priorityMicroLamports?: number },
  minBalance: number,
): Promise<boolean> {
  const { now, states } = await pool.run(async (ctx) => ({
    now: await clusterTime(ctx),
    states: (await (ctx.program.account as any).epochState.all()).map(
      ({ account }: { account: any }) => account,
    ),
  }));
  const views = states.map(epochView);
  const { actions, health } = planKeeper(views, now, config);

  let unhealthy = false;
  for (const signal of health) {
    log(signal.level, signal.code, { message: signal.message });
    if (signal.level === "error") unhealthy = true;
  }
  const balance = await pool.run((ctx) =>
    ctx.connection.getBalance(ctx.wallet.publicKey),
  );
  if (balance < minBalance) {
    log("warn", "keeper-balance-low", {
      lamports: balance,
      minimum: minBalance,
    });
  }

  const byEpoch = new Map<number, any>(
    states.map((s: any) => [Number(s.epoch), s]),
  );
  for (const action of actions) {
    try {
      const result = await execute(pool, action, byEpoch, send);
      if (action.kind === "sweep") config.lastSweepAt = now;
      log("info", `${action.kind}-done`, { epoch: action.epoch, ...result });
    } catch (error) {
      const message = String((error as Error).message ?? error);
      // Another keeper got there first, or there is nothing to do: not a failure.
      if (action.kind === "sweep" && /NothingToSweep/.test(message)) {
        config.lastSweepAt = now;
        log("info", "sweep-skipped", {
          epoch: action.epoch,
          reason: "nothing to sweep",
        });
        continue;
      }
      log("warn", `${action.kind}-failed`, {
        epoch: action.epoch,
        error: message.slice(0, 300),
      });
    }
  }
  return unhealthy;
}

async function execute(
  pool: RpcPool,
  action: KeeperAction,
  states: Map<number, any>,
  send: { priorityMicroLamports?: number },
): Promise<object> {
  switch (action.kind) {
    case "finalize":
      return {
        signature: await pool.run((ctx) =>
          sendFinalizeEpoch(ctx, action.epoch, send),
        ),
        created: action.epoch + 2,
      };
    case "credit": {
      const state = states.get(action.epoch);
      const signatures = await pool.run(async (ctx) => {
        const [address] = (await import("../lib/pdas")).findEpochPda(
          ctx.programId,
          action.epoch,
        );
        return sendCreditEpoch(
          ctx,
          action.epoch,
          address,
          state.roster,
          action.slots,
          send,
        );
      });
      return { slots: action.slots.length, signatures };
    }
    case "close": {
      const state = states.get(action.epoch);
      const rentPayer = new PublicKey(Uint8Array.from(state.rentPayer));
      return {
        signature: await pool.run((ctx) =>
          sendCloseEpoch(ctx, action.epoch, action.openEpoch, rentPayer, send),
        ),
        rolledInto: action.openEpoch,
      };
    }
    case "sweep":
      return {
        signature: await pool.run((ctx) =>
          sendSweepTreasury(ctx, action.epoch, send),
        ),
      };
  }
}
