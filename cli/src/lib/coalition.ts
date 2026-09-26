import { Program } from "@coral-xyz/anchor";
import { etc, Point } from "@noble/secp256k1";
import { bytesField, offenceCoalitionBitmaps } from "./slash";
import {
  CoalitionKeyArgs,
  signerRemainingAccounts,
} from "./signing/attestation";

export function coalitionKeyFromPoint(point: Point): CoalitionKeyArgs {
  if (point.is0())
    throw new Error("coalition key sum is the point at infinity");
  const { x, y } = point.toAffine();
  return {
    x: Array.from(etc.numberToBytesBE(x)),
    y: Array.from(etc.numberToBytesBE(y)),
  };
}

/** Parse a user-supplied `{ x, y }` (hex strings or byte arrays). */
export function parseCoalitionKey(
  value: unknown,
  label = "coalition key",
): CoalitionKeyArgs {
  const key = value as { x?: unknown; y?: unknown } | undefined;
  if (!key || key.x === undefined || key.y === undefined) {
    throw new Error(`${label} must be { x, y }`);
  }
  return {
    x: Array.from(bytesField(key.x, `${label}.x`, 32)),
    y: Array.from(bytesField(key.y, `${label}.y`, 32)),
  };
}

/**
 * Coalition key for `signersBitmap`, summed from the signers' `Node` accounts.
 *
 * Every verifying instruction takes it as an untrusted input and checks it against its own sum,
 * so it only has to be the plain point sum (any order, any library).
 */
export async function fetchCoalitionKey(
  program: Program,
  registry: { nodeCount: number; nodes: Array<number[] | Uint8Array> },
  signersBitmap: number[] | Uint8Array,
): Promise<CoalitionKeyArgs> {
  const pdas = signerRemainingAccounts(registry, signersBitmap).map(
    (meta) => meta.pubkey,
  );
  if (pdas.length === 0) throw new Error("signers bitmap is empty");
  const nodes = await (program.account as any).node.fetchMultiple(pdas);
  let sum = Point.ZERO;
  nodes.forEach((node: any, index: number) => {
    if (!node) {
      throw new Error(`signer node ${pdas[index].toBase58()} is missing`);
    }
    sum = sum.add(
      Point.fromAffine({
        x: etc.bytesToNumberBE(Uint8Array.from(node.secp256K1PubkeyX)),
        y: etc.bytesToNumberBE(Uint8Array.from(node.secp256K1PubkeyY)),
      }),
    );
  });
  return coalitionKeyFromPoint(sum);
}

/**
 * `offence` with every coalition key it needs, fetching the ones it does not carry.
 *
 * Keys are verification inputs, not evidence: they do not enter the evidence hash.
 */
export async function withOffenceCoalitionKeys(
  program: Program,
  registry: { nodeCount: number; nodes: Array<number[] | Uint8Array> },
  offence: Record<string, unknown>,
): Promise<Record<string, unknown>> {
  const needed = offenceCoalitionBitmaps(offence);
  if (needed.length === 0) return offence;
  const [variantName, body] = Object.entries(offence)[0] as [
    string,
    Record<string, unknown>,
  ];
  const filled: Record<string, unknown> = { ...body };
  for (const [field, bitmap] of needed) {
    const snake = field.replace(/[A-Z]/g, (c) => `_${c.toLowerCase()}`);
    const supplied = filled[field] ?? filled[snake];
    delete filled[snake];
    filled[field] =
      supplied !== undefined
        ? parseCoalitionKey(supplied, field)
        : await fetchCoalitionKey(program, registry, bitmap);
  }
  return { [variantName]: filled };
}
