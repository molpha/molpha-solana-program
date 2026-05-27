"use strict";
var __importDefault = (this && this.__importDefault) || function (mod) {
    return (mod && mod.__esModule) ? mod : { "default": mod };
};
Object.defineProperty(exports, "__esModule", { value: true });
exports.getGlobalOptions = getGlobalOptions;
exports.getCliContext = getCliContext;
const anchor_1 = require("@coral-xyz/anchor");
const web3_js_1 = require("@solana/web3.js");
const fs_1 = __importDefault(require("fs"));
const config_1 = require("./config");
const utils_1 = require("./utils");
function getGlobalOptions(command) {
    return command.optsWithGlobals();
}
async function getCliContext(command) {
    const options = (0, config_1.resolveCliOptions)(getGlobalOptions(command));
    (0, config_1.ensureIdlPresent)(options.idlPath);
    const payer = (0, utils_1.loadKeypair)(options.walletPath);
    const wallet = new anchor_1.Wallet(payer);
    const connection = new web3_js_1.Connection(options.rpcUrl, options.commitment);
    const provider = new anchor_1.AnchorProvider(connection, wallet, {
        commitment: options.commitment,
        preflightCommitment: options.commitment,
    });
    const rawIdl = (0, utils_1.readJsonFile)(options.idlPath);
    rawIdl.address = options.programId;
    if (!fs_1.default.existsSync(options.idlPath)) {
        throw new Error(`Missing idl: ${options.idlPath}`);
    }
    const program = new anchor_1.Program(rawIdl, provider);
    return {
        options,
        payer,
        wallet,
        provider,
        connection,
        program,
        programId: new web3_js_1.PublicKey(options.programId),
        idl: rawIdl,
    };
}
