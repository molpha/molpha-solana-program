import { Connection, PublicKey } from "@solana/web3.js";

export const BPF_LOADER_UPGRADEABLE_PROGRAM_ID = new PublicKey(
  "BPFLoaderUpgradeab1e11111111111111111111111",
);

/** `UpgradeableLoaderState::ProgramData` enum tag. */
const PROGRAM_DATA_TAG = 3;
/** u32 tag, u64 deployment slot, then `Option<Pubkey>`. */
const AUTHORITY_OPTION_OFFSET = 12;
const PROGRAM_DATA_METADATA_LEN = AUTHORITY_OPTION_OFFSET + 1 + 32;

/** The upgradeable loader's ProgramData account, which records a program's upgrade authority. */
export function findProgramDataAddress(programId: PublicKey): PublicKey {
  return PublicKey.findProgramAddressSync(
    [programId.toBuffer()],
    BPF_LOADER_UPGRADEABLE_PROGRAM_ID,
  )[0];
}

/** Upgrade authority stored in a ProgramData account, or `null` for an immutable program. */
export function decodeUpgradeAuthority(data: Uint8Array): PublicKey | null {
  const buffer = Buffer.from(data);
  if (
    buffer.length < PROGRAM_DATA_METADATA_LEN ||
    buffer.readUInt32LE(0) !== PROGRAM_DATA_TAG
  ) {
    throw new Error("not a ProgramData account");
  }
  const option = buffer[AUTHORITY_OPTION_OFFSET];
  if (option === 0) {
    return null;
  }
  if (option !== 1) {
    throw new Error("not a ProgramData account");
  }
  return new PublicKey(
    buffer.subarray(AUTHORITY_OPTION_OFFSET + 1, PROGRAM_DATA_METADATA_LEN),
  );
}

/**
 * `initialize` only accepts the program's upgrade authority. Checked here so a wrong wallet
 * is named before a transaction is built, rather than surfacing as a simulation failure.
 */
export async function requireUpgradeAuthority(
  connection: Connection,
  programId: PublicKey,
  signer: PublicKey,
): Promise<PublicKey> {
  const programData = findProgramDataAddress(programId);
  const account = await connection.getAccountInfo(programData);
  if (
    account === null ||
    !account.owner.equals(BPF_LOADER_UPGRADEABLE_PROGRAM_ID)
  ) {
    throw new Error(
      `Program ${programId.toBase58()} has no ProgramData account on this cluster; deploy it with the upgradeable loader first`,
    );
  }
  const upgradeAuthority = decodeUpgradeAuthority(account.data);
  if (upgradeAuthority === null) {
    throw new Error(
      `Program ${programId.toBase58()} is immutable: it has no upgrade authority, so it can no longer be initialized`,
    );
  }
  if (!upgradeAuthority.equals(signer)) {
    throw new Error(
      `init must be signed by the program's upgrade authority ${upgradeAuthority.toBase58()}; --wallet is ${signer.toBase58()}`,
    );
  }
  return programData;
}
