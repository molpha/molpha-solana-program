"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.registerDataCommands = registerDataCommands;
const anchor_1 = require("@coral-xyz/anchor");
const web3_js_1 = require("@solana/web3.js");
const accounts_1 = require("../lib/accounts");
const client_1 = require("../lib/client");
const config_1 = require("../lib/config");
const pdas_1 = require("../lib/pdas");
const data_update_1 = require("../lib/signing/data-update");
const fixtures_1 = require("../lib/signing/fixtures");
const utils_1 = require("../lib/utils");
function registerDataCommands(program) {
    const data = program.command("data").description("Sign and submit data updates");
    data
        .command("sign")
        .description("Produce signed SubmitDataUpdate args")
        .option("--fixture <path>", "fixture file")
        .option("--job-id <hex>", "32-byte job id")
        .option("--registry-version <number>", "registry version")
        .option("--config-hash <hex>", "32-byte config hash")
        .option("--value <hex>", "32-byte value")
        .option("--canonical-timestamp <number>", "unix timestamp")
        .option("--signer-count <number>", "number of signers to include", "8")
        .option("--signers-bitmap <hex>", "explicit 32-byte signers bitmap")
        .option("--signatures-required <number>", "override signatures required")
        .option("--signer-seed <string>", "deterministic signer seed", "molpha-cli-e2e-signers")
        .action(async (options, command) => {
        const programId = getProgramIdFromGlobals(command);
        const { payload, remaining } = buildSignedPayload(options, programId);
        (0, utils_1.writeJson)({
            ...(0, data_update_1.describeSignedPayload)(payload),
            remainingAccounts: remaining.map((entry) => entry.pubkey.toBase58()),
        });
    });
    data
        .command("submit")
        .description("Submit a signed data update")
        .option("--fixture <path>", "fixture file")
        .option("--args <path>", "json file with submit args")
        .option("--job-id <hex>", "32-byte job id")
        .option("--registry-version <number>", "registry version")
        .option("--config-hash <hex>", "32-byte config hash")
        .option("--value <hex>", "32-byte value")
        .option("--canonical-timestamp <number>", "unix timestamp")
        .option("--signer-count <number>", "number of signers to include", "8")
        .option("--signers-bitmap <hex>", "explicit 32-byte signers bitmap")
        .option("--signatures-required <number>", "override signatures required")
        .option("--signer-seed <string>", "deterministic signer seed", "molpha-cli-e2e-signers")
        .option("--compute-unit-limit <number>", "compute unit limit", "1400000")
        .action(async (options, command) => {
        const ctx = await (0, client_1.getCliContext)(command);
        const [protocolConfigPda] = (0, pdas_1.findProtocolConfigPda)(ctx.programId);
        const [registryStatePda] = (0, pdas_1.findRegistryStatePda)(ctx.programId);
        const registryState = await (0, accounts_1.fetchAccountOrNull)(ctx.program, "registryState", registryStatePda);
        if (!registryState) {
            throw new Error("Registry state is not initialized");
        }
        const submitArgs = resolveSubmitArgs(options, ctx.programId);
        const [jobPda] = (0, pdas_1.findJobPda)(ctx.programId, bytesFromField(submitArgs.jobId, 32, "job id"));
        const [feedPda] = (0, pdas_1.findFeedPda)(ctx.programId, bytesFromField(submitArgs.jobId, 32, "job id"));
        const { transitionAccount, remainingTransition } = await (0, accounts_1.resolveTransitionForRegistryVersion)(ctx.program, registryState, Number(submitArgs.registryVersion));
        const remaining = (0, data_update_1.signerRemainingAccounts)(ctx.programId, bytesFromField(submitArgs.signersBitmap, 32, "signers bitmap"), remainingTransition);
        const signature = await ctx.program.methods
            .submitDataUpdate(submitArgs)
            .accountsPartial({
            submitter: ctx.wallet.publicKey,
            protocolConfig: protocolConfigPda,
            registryState: registryStatePda,
            transitionAccount,
            job: jobPda,
            feed: feedPda,
        })
            .remainingAccounts(remaining)
            .preInstructions([
            web3_js_1.ComputeBudgetProgram.setComputeUnitLimit({
                units: (0, utils_1.parseInteger)(options.computeUnitLimit, "compute unit limit"),
            }),
        ])
            .rpc();
        const feed = await (0, accounts_1.fetchAccountOrNull)(ctx.program, "feed", feedPda);
        (0, utils_1.writeJson)({
            signature,
            job: jobPda.toBase58(),
            feed: feedPda.toBase58(),
            feedAccount: feed ? (0, accounts_1.accountJson)(feedPda, feed) : null,
        });
    });
    data
        .command("verify")
        .description("Simulate verify_data_update and print return data")
        .option("--fixture <path>", "fixture file")
        .option("--args <path>", "json file with submit args")
        .option("--job-id <hex>", "32-byte job id")
        .option("--registry-version <number>", "registry version")
        .option("--config-hash <hex>", "32-byte config hash")
        .option("--value <hex>", "32-byte value")
        .option("--canonical-timestamp <number>", "unix timestamp")
        .option("--signer-count <number>", "number of signers to include", "8")
        .option("--signers-bitmap <hex>", "explicit 32-byte signers bitmap")
        .option("--signatures-required <number>", "override signatures required")
        .option("--signer-seed <string>", "deterministic signer seed", "molpha-cli-e2e-signers")
        .option("--compute-unit-limit <number>", "compute unit limit", "1400000")
        .action(async (options, command) => {
        const ctx = await (0, client_1.getCliContext)(command);
        const [registryStatePda] = (0, pdas_1.findRegistryStatePda)(ctx.programId);
        const registryState = await (0, accounts_1.fetchAccountOrNull)(ctx.program, "registryState", registryStatePda);
        if (!registryState) {
            throw new Error("Registry state is not initialized");
        }
        const submitArgs = resolveSubmitArgs(options, ctx.programId);
        const [jobPda] = (0, pdas_1.findJobPda)(ctx.programId, bytesFromField(submitArgs.jobId, 32, "job id"));
        const { transitionAccount, remainingTransition } = await (0, accounts_1.resolveTransitionForRegistryVersion)(ctx.program, registryState, Number(submitArgs.registryVersion));
        const remaining = (0, data_update_1.signerRemainingAccounts)(ctx.programId, bytesFromField(submitArgs.signersBitmap, 32, "signers bitmap"), remainingTransition);
        const verifyIx = await ctx.program.methods
            .verifyDataUpdate(submitArgs)
            .accountsPartial({
            registryState: registryStatePda,
            transitionAccount,
            job: jobPda,
        })
            .remainingAccounts(remaining)
            .instruction();
        const tx = new web3_js_1.Transaction().add(web3_js_1.ComputeBudgetProgram.setComputeUnitLimit({
            units: (0, utils_1.parseInteger)(options.computeUnitLimit, "compute unit limit"),
        }), verifyIx);
        tx.feePayer = ctx.wallet.publicKey;
        tx.recentBlockhash = (await ctx.connection.getLatestBlockhash(ctx.options.commitment)).blockhash;
        tx.sign(ctx.payer);
        const simulation = await ctx.provider.simulate(tx, [ctx.payer], ctx.options.commitment);
        if (!simulation.returnData) {
            throw new Error("verify_data_update returned no return data");
        }
        const rawBytes = Buffer.from(simulation.returnData.data[0], "base64");
        (0, utils_1.writeJson)({
            job: jobPda.toBase58(),
            returnData: (0, accounts_1.decodeVerifyReturnData)(rawBytes),
            logs: simulation.logs ?? [],
        });
    });
}
function buildSignedPayload(options, programId) {
    if (options.fixture) {
        const fixture = (0, fixtures_1.loadVerifyFixture)(options.fixture);
        const submitArgs = (0, data_update_1.buildSubmitArgsFromFixture)(fixture);
        return {
            payload: {
                submitArgs,
                signersBitmap: bytesFromField(submitArgs.signersBitmap, 32, "signers bitmap"),
                aggSigS: bytesFromField(submitArgs.aggSigS, 32, "agg sig s"),
                commitmentAddr: bytesFromField(submitArgs.commitmentAddr, 20, "commitment"),
            },
            remaining: (0, data_update_1.signerRemainingAccounts)(programId, bytesFromField(submitArgs.signersBitmap, 32, "signers bitmap")),
        };
    }
    const signerCount = (0, utils_1.parseInteger)(options.signerCount ?? "8", "signer count");
    const signerSeed = options.signerSeed ?? "molpha-cli-e2e-signers";
    const signerMaterial = (0, data_update_1.generateE2eSignerMaterialFromSeed)(signerSeed, signerCount);
    const signersBitmap = options.signersBitmap
        ? (0, data_update_1.buildBitmapFromFlag)(options.signersBitmap)
        : undefined;
    const payload = (0, data_update_1.signE2eDataUpdate)((0, utils_1.parseInteger)((0, utils_1.requireValue)(options.registryVersion, "Missing --registry-version"), "registry version"), signerMaterial, signersBitmap ?? signerCount, (0, utils_1.hexToBytes)((0, utils_1.requireValue)(options.jobId, "Missing --job-id"), 32, "job id"), {
        configHash: (0, utils_1.hexToBytes)((0, utils_1.requireValue)(options.configHash, "Missing --config-hash"), 32, "config hash"),
        value: (0, utils_1.hexToBytes)((0, utils_1.requireValue)(options.value, "Missing --value"), 32, "value"),
        canonicalTimestamp: (0, utils_1.parseInteger)((0, utils_1.requireValue)(options.canonicalTimestamp, "Missing --canonical-timestamp"), "canonical timestamp"),
    }, options.signaturesRequired ? (0, utils_1.parseInteger)(options.signaturesRequired, "signatures required") : undefined);
    return {
        payload,
        remaining: (0, data_update_1.signerRemainingAccounts)(programId, payload.signersBitmap),
    };
}
function resolveSubmitArgs(options, programId) {
    if (options.args) {
        return normalizeSubmitArgs((0, utils_1.readJsonFile)(options.args));
    }
    if (options.fixture) {
        return (0, data_update_1.buildSubmitArgsFromFixture)((0, fixtures_1.loadVerifyFixture)(options.fixture));
    }
    return buildSignedPayload(options, programId).payload.submitArgs;
}
function normalizeSubmitArgs(value) {
    return {
        jobId: Array.from(bytesFromField(value.jobId, 32, "job id")),
        registryVersion: typeof value.registryVersion === "number"
            ? value.registryVersion
            : (0, utils_1.parseInteger)(value.registryVersion, "registry version"),
        roundId: Array.from(bytesFromField(value.roundId, 32, "round id")),
        signersBitmap: Array.from(bytesFromField(value.signersBitmap, 32, "signers bitmap")),
        value: Array.from(bytesFromField(value.value, 32, "value")),
        canonicalTimestamp: new anchor_1.BN(String(value.canonicalTimestamp)),
        aggSigR: Array.from(bytesFromField(value.aggSigR, 32, "agg sig r")),
        aggSigS: Array.from(bytesFromField(value.aggSigS, 32, "agg sig s")),
        commitmentAddr: Array.from(bytesFromField(value.commitmentAddr, 20, "commitment")),
    };
}
function bytesFromField(value, expectedLength, label) {
    if (typeof value === "string") {
        return (0, utils_1.hexToBytes)(value, expectedLength, label);
    }
    const bytes = Uint8Array.from(value);
    if (bytes.length !== expectedLength) {
        throw new Error(`${label} must be ${expectedLength} bytes, got ${bytes.length}`);
    }
    return bytes;
}
function getProgramIdFromGlobals(command) {
    const resolved = (0, config_1.resolveCliOptions)((0, client_1.getGlobalOptions)(command));
    return new web3_js_1.PublicKey(resolved.programId);
}
