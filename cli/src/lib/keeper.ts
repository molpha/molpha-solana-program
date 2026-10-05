import { creditableSlots } from "./ticket";

/** The part of an `EpochState` the keeper plans from. */
export type EpochView = {
  epoch: number;
  start: number;
  end: number;
  ticketGrace: number;
  finalized: boolean;
  weightedCount: number;
  creditedCount: number;
  /** Weighted roster slots still awaiting `credit_epoch_rewards`. */
  creditSlots: number[];
};

export type KeeperAction =
  | { kind: "finalize"; epoch: number }
  | { kind: "credit"; epoch: number; slots: number[] }
  | { kind: "close"; epoch: number; openEpoch: number }
  | { kind: "sweep"; epoch: number };

export type KeeperHealth = {
  level: "warn" | "error";
  code: "no-open-epoch" | "epoch-runway-low" | "finalize-overdue";
  message: string;
};

export type KeeperConfig = {
  /** Warn when the newest epoch ends within this many seconds. */
  runwaySeconds: number;
  /** Alert when an epoch has gone unfinalized this long past its ticket deadline. */
  finalizeOverdueSeconds: number;
  /** Sweep at most this often, unix seconds of the previous sweep in `lastSweepAt`. */
  sweepIntervalSeconds: number;
  lastSweepAt: number | null;
};

export const KEEPER_DEFAULTS = {
  runwaySeconds: 7_200,
  finalizeOverdueSeconds: 900,
  sweepIntervalSeconds: 3_600,
} as const;

export function epochView(state: any): EpochView {
  return {
    epoch: Number(state.epoch),
    start: Number(state.start),
    end: Number(state.end),
    ticketGrace: Number(state.ticketGrace),
    finalized: state.finalized !== 0,
    weightedCount: Number(state.weightedCount),
    creditedCount: Number(state.creditedCount),
    creditSlots: state.finalized !== 0 ? creditableSlots(state) : [],
  };
}

const ticketDeadline = (e: EpochView) => e.end + e.ticketGrace;

/**
 * What a permissionless keeper should do now, and what is wrong with the epoch lifecycle.
 *
 * Epochs advance only when somebody calls `finalize_epoch` after an epoch's ticket deadline: that
 * creates the epoch two ahead and sets the ticket threshold the lottery runs on. If nobody does,
 * the newest epoch eventually ends, no epoch is open, and rounds can no longer be ticketed or
 * income swept. The plan finalizes in order (each finalize creates the account the next one needs),
 * credits finalized epochs, closes the ones past their evidence window, and sweeps the open epoch.
 */
export function planKeeper(
  epochs: EpochView[],
  now: number,
  config: KeeperConfig,
): { actions: KeeperAction[]; health: KeeperHealth[] } {
  const sorted = [...epochs].sort((a, b) => a.epoch - b.epoch);
  const actions: KeeperAction[] = [];
  const health: KeeperHealth[] = [];
  const known = new Set(sorted.map((e) => e.epoch));

  const open = sorted.find(
    (e) => !e.finalized && e.start <= now && now < e.end,
  );
  if (!open) {
    health.push({
      level: "error",
      code: "no-open-epoch",
      message: `no epoch is open at cluster time ${now}; rounds cannot be ticketed and income cannot be swept until an overdue epoch is finalized`,
    });
  }
  const newestEnd = sorted.reduce((m, e) => Math.max(m, e.end), 0);
  if (open && newestEnd - now < config.runwaySeconds) {
    health.push({
      level: "warn",
      code: "epoch-runway-low",
      message: `the newest epoch ends in ${newestEnd - now}s (below ${config.runwaySeconds}s); finalize_epoch is what creates more`,
    });
  }

  for (const e of sorted) {
    if (e.finalized || now < ticketDeadline(e)) continue;
    const overdue = now - ticketDeadline(e);
    if (overdue > config.finalizeOverdueSeconds) {
      health.push({
        level: "error",
        code: "finalize-overdue",
        message: `epoch ${e.epoch} has been past its ticket deadline for ${overdue}s without finalization; the ticket threshold is no longer adapting`,
      });
    }
    // Finalizing e needs e+1 to exist and creates e+2.
    if (known.has(e.epoch + 1) && !known.has(e.epoch + 2)) {
      actions.push({ kind: "finalize", epoch: e.epoch });
      known.add(e.epoch + 2);
    }
  }

  for (const e of sorted) {
    if (e.finalized && e.creditSlots.length > 0) {
      actions.push({ kind: "credit", epoch: e.epoch, slots: e.creditSlots });
    }
    const evidenceOver =
      now >= ticketDeadline(e) + Math.max(e.end - e.start, 0);
    if (
      e.finalized &&
      e.creditSlots.length === 0 &&
      e.creditedCount >= e.weightedCount &&
      evidenceOver &&
      open
    ) {
      actions.push({ kind: "close", epoch: e.epoch, openEpoch: open.epoch });
    }
  }

  const sweepDue =
    config.lastSweepAt === null ||
    now - config.lastSweepAt >= config.sweepIntervalSeconds;
  if (open && sweepDue) actions.push({ kind: "sweep", epoch: open.epoch });

  return { actions, health };
}
