import { BN } from "@coral-xyz/anchor";

/** Mirrors `MAX_SETTLEMENT_DELAY_CAP_SECONDS` in the program (7 days). */
export const MAX_SETTLEMENT_DELAY_CAP_SECONDS = 604_800;
/** Mirror `MIN_EPOCH_LEN_SECONDS` / `MAX_EPOCH_LEN_SECONDS` in the program. */
export const MIN_EPOCH_LEN_SECONDS = 3_600;
export const MAX_EPOCH_LEN_SECONDS = 30 * 86_400;
/** Mirrors `MAX_TICKETS_PER_EPOCH` in the program; the ticket target may be at most 1/8 of it. */
export const MAX_TICKETS_PER_EPOCH = 65_535;

/** Mirrors `TAU_MIN` in the program: the lowest ticket threshold, a sampling probability of 2^-32. */
export const TAU_MIN = new BN(1).shln(32);
export const U64_MAX = new BN("18446744073709551615");

/**
 * Ticket threshold for the genesis epochs: `2^64 * target / expected`, so that about `target`
 * of `expected` rounds become tickets. Expecting no more rounds than the target means every round
 * is sampled (`u64::MAX`); a tiny load is clamped to the floor.
 */
export function deriveInitialTau(
  targetTicketsPerEpoch: number,
  expectedRoundsPerEpoch: BN,
): BN {
  if (expectedRoundsPerEpoch.isNeg() || expectedRoundsPerEpoch.isZero()) {
    throw new Error("expectedRoundsPerEpoch must be a positive integer");
  }
  const target = new BN(targetTicketsPerEpoch);
  if (expectedRoundsPerEpoch.lte(target)) return U64_MAX.clone();
  const tau = new BN(1).shln(64).mul(target).div(expectedRoundsPerEpoch);
  return BN.max(tau, TAU_MIN);
}

export function validateInitialTau(tau: BN): void {
  if (tau.lt(TAU_MIN) || tau.gt(U64_MAX)) {
    throw new Error(
      `initialTau must be between ${TAU_MIN.toString()} (2^32) and ${U64_MAX.toString()} (u64 max)`,
    );
  }
}

/** Defaults for the epoch reward knobs when neither a flag nor a config file sets them. */
export const EPOCH_DEFAULTS = {
  epochLenSeconds: "86400",
  ticketGraceSeconds: "21600",
  targetTicketsPerEpoch: 2_000,
  minAvailabilityBps: 8_000,
  minAvailabilitySamples: 50,
} as const;

type ProtocolConfigArgs = {
  withdrawalCooldownSlots: BN;
  minimumNodeDeposit: BN;
  rewardPerSignature: BN;
  maxSettlementDelaySeconds: BN;
  challengerBountyBps: number;
  x402RoundBase: BN;
  minSigners: number;
  epochLenSeconds: BN;
  ticketGraceSeconds: BN;
  targetTicketsPerEpoch: number;
  minAvailabilityBps: number;
  minAvailabilitySamples: number;
  protocolFeeBps: number;
  redundancyBuffer?: number;
};

/** Mirror the program's static bounds before encoding or submitting a transaction.
 * Timestamp and slot deadlines are also checked against the on-chain clock.
 */
export function validateProtocolConfig(args: ProtocolConfigArgs): void {
  const maxU64 = new BN("18446744073709551615");
  for (const field of [
    "withdrawalCooldownSlots",
    "minimumNodeDeposit",
    "rewardPerSignature",
    "maxSettlementDelaySeconds",
    "x402RoundBase",
    "epochLenSeconds",
    "ticketGraceSeconds",
  ] as const) {
    const value = args[field];
    const allowZero =
      field === "rewardPerSignature" || field === "x402RoundBase";
    if (value.isNeg() || value.gt(maxU64) || (!allowZero && value.isZero())) {
      throw new Error(
        `${field} must be ${allowZero ? "a non-negative" : "a positive"} u64`,
      );
    }
  }
  // Mirrors MAX_SETTLEMENT_DELAY_CAP_SECONDS in the program (7 days).
  if (
    args.maxSettlementDelaySeconds.gt(new BN(MAX_SETTLEMENT_DELAY_CAP_SECONDS))
  ) {
    throw new Error(
      "maxSettlementDelaySeconds must not exceed 604800 (7 days)",
    );
  }
  if (
    args.epochLenSeconds.lt(new BN(MIN_EPOCH_LEN_SECONDS)) ||
    args.epochLenSeconds.gt(new BN(MAX_EPOCH_LEN_SECONDS))
  ) {
    throw new Error(
      `epochLenSeconds must be between ${MIN_EPOCH_LEN_SECONDS} and ${MAX_EPOCH_LEN_SECONDS}`,
    );
  }
  if (args.ticketGraceSeconds.gte(args.epochLenSeconds)) {
    throw new Error("ticketGraceSeconds must be shorter than epochLenSeconds");
  }
  for (const [field, min, max] of [
    ["challengerBountyBps", 0, 10000],
    ["minSigners", 3, 255],
    ["redundancyBuffer", 1, 8],
    ["targetTicketsPerEpoch", 1, Math.floor(MAX_TICKETS_PER_EPOCH / 8)],
    ["minAvailabilityBps", 0, 10000],
    ["minAvailabilitySamples", 1, 65535],
    ["protocolFeeBps", 0, 10000],
  ] as const) {
    if (field === "redundancyBuffer" && args[field] === undefined) continue;
    const value = args[field];
    if (
      value === undefined ||
      !Number.isInteger(value) ||
      value < min ||
      value > max
    ) {
      throw new Error(`${field} must be an integer between ${min} and ${max}`);
    }
  }
  if (args.rewardPerSignature.muln(256).add(args.x402RoundBase).gt(maxU64)) {
    throw new Error("base price plus rewards for 256 signers must fit u64");
  }
}
