import { PublicKey, Keypair, SystemProgram } from "@solana/web3.js";
import { SYSVAR_CLOCK_PUBKEY, SYSVAR_RENT_PUBKEY } from "@solana/web3.js";

/**
 * Mock Wormhole account addresses for testing
 * These are deterministic addresses that won't conflict with real accounts
 */
/** Must match `mock_wormhole_bridge` in Anchor.toml — a real deployed .so in bankrun tests. */
export const MOCK_WORMHOLE_BRIDGE_PROGRAM = new PublicKey(
  "HSaVLzvssvTMroUHvB4sDGvThHK7RmpZLQPr8W8z61jo"
);

/**
 * Interface for Wormhole accounts used in node registration
 */
export interface WormholeAccounts {
  wormholeBridgeProgram: PublicKey;
  wormholeBridge: PublicKey;
  wormholeMessage: Keypair;
  wormholeEmitter: PublicKey;
  wormholeSequence: PublicKey;
  wormholeFeeCollector: PublicKey;
  clock: PublicKey;
  rent: PublicKey;
}

/**
 * Derive Wormhole bridge config PDA
 * In real Wormhole, this is derived from the bridge program
 * For testing, we use a deterministic derivation
 */
export function getWormholeBridgePDA(wormholeProgram: PublicKey): PublicKey {
  const [pda] = PublicKey.findProgramAddressSync(
    [Buffer.from("Bridge")],
    wormholeProgram
  );
  return pda;
}

/**
 * Derive Wormhole sequence PDA for an emitter
 * In real Wormhole, this tracks message sequence numbers
 */
export function getWormholeSequencePDA(
  wormholeProgram: PublicKey,
  emitter: PublicKey
): PublicKey {
  const [pda] = PublicKey.findProgramAddressSync(
    [Buffer.from("Sequence"), emitter.toBuffer()],
    wormholeProgram
  );
  return pda;
}

/**
 * Get Wormhole fee collector address
 * In real Wormhole, this is a known address
 * For testing, we use a deterministic derivation
 */
export function getWormholeFeeCollector(wormholeProgram: PublicKey): PublicKey {
  const [pda] = PublicKey.findProgramAddressSync(
    [Buffer.from("fee_collector")],
    wormholeProgram
  );
  return pda;
}

/**
 * Create mock Wormhole accounts for testing
 * These accounts satisfy the account constraints but don't need to be valid Wormhole accounts
 * since the program checks if Wormhole is configured before using them
 */
export function createMockWormholeAccounts(
  protocolConfigPDA: PublicKey,
  wormholeBridgeProgram: PublicKey = MOCK_WORMHOLE_BRIDGE_PROGRAM
): WormholeAccounts {
  const wormholeBridge = getWormholeBridgePDA(wormholeBridgeProgram);
  const wormholeEmitter = protocolConfigPDA; // Protocol config is the emitter
  const wormholeSequence = getWormholeSequencePDA(
    wormholeBridgeProgram,
    wormholeEmitter
  );
  const wormholeFeeCollector = getWormholeFeeCollector(wormholeBridgeProgram);
  const wormholeMessage = Keypair.generate(); // Fresh keypair for each message

  return {
    wormholeBridgeProgram,
    wormholeBridge,
    wormholeMessage,
    wormholeEmitter,
    wormholeSequence,
    wormholeFeeCollector,
    clock: SYSVAR_CLOCK_PUBKEY,
    rent: SYSVAR_RENT_PUBKEY,
  };
}

/**
 * Create default mock Wormhole accounts using SystemProgram
 * Useful when Wormhole is not configured (default Pubkey)
 */
export function createDefaultMockWormholeAccounts(
  protocolConfigPDA: PublicKey
): WormholeAccounts {
  // Use SystemProgram as a placeholder when Wormhole is disabled
  const defaultProgram = SystemProgram.programId;
  return createMockWormholeAccounts(protocolConfigPDA, defaultProgram);
}
