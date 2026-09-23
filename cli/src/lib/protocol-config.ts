import { BN } from "@coral-xyz/anchor";

/** Mirrors `MAX_SETTLEMENT_DELAY_CAP_SECONDS` in the program (7 days). */
export const MAX_SETTLEMENT_DELAY_CAP_SECONDS = 604_800;

type ProtocolConfigArgs = {
  withdrawalCooldownSlots: BN;
  minimumNodeDeposit: BN;
  rewardPerSignature: BN;
  maxSettlementDelaySeconds: BN;
  challengerBountyBps: number;
  livelinessFreezeThreshold: number;
  livelinessPenaltyPerMiss: number;
  livelinessRecoveryPerSign: number;
  x402RoundBase: BN;
  minSigners: number;
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
  for (const [field, min, max] of [
    ["challengerBountyBps", 0, 10000],
    ["livelinessFreezeThreshold", 1, 10000],
    ["livelinessPenaltyPerMiss", 1, 10000],
    ["livelinessRecoveryPerSign", 1, 10000],
    ["minSigners", 3, 255],
    ["redundancyBuffer", 1, 8],
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
