import { AnchorProvider, Idl, Program, Wallet } from "@coral-xyz/anchor";
import { Connection, Keypair, PublicKey } from "@solana/web3.js";
import fs from "fs";
import { Command } from "commander";
import {
  CliGlobalOptions,
  ResolvedCliOptions,
  ensureIdlPresent,
  resolveCliOptions,
} from "./config";
import { loadKeypair, readJsonFile } from "./utils";

export type CliContext = {
  options: ResolvedCliOptions;
  payer: Keypair;
  wallet: Wallet;
  provider: AnchorProvider;
  connection: Connection;
  program: Program;
  programId: PublicKey;
  idl: Idl;
};

export function getGlobalOptions(command: Command): CliGlobalOptions {
  return command.optsWithGlobals<CliGlobalOptions>();
}

export async function getCliContext(command: Command): Promise<CliContext> {
  const options = resolveCliOptions(getGlobalOptions(command));
  ensureIdlPresent(options.idlPath);

  const payer = loadKeypair(options.walletPath);
  const wallet = new Wallet(payer);
  const connection = new Connection(options.rpcUrl, options.commitment);
  const provider = new AnchorProvider(connection, wallet, {
    commitment: options.commitment,
    preflightCommitment: options.commitment,
  });

  const rawIdl = readJsonFile<Idl & { address?: string }>(options.idlPath);
  rawIdl.address = options.programId;
  if (!fs.existsSync(options.idlPath)) {
    throw new Error(`Missing idl: ${options.idlPath}`);
  }

  const program = new Program(rawIdl as Idl, provider);
  return {
    options,
    payer,
    wallet,
    provider,
    connection,
    program,
    programId: new PublicKey(options.programId),
    idl: rawIdl as Idl,
  };
}
