"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.fetchAccountOrNull = fetchAccountOrNull;
exports.accountExists = accountExists;
exports.registryTransitionKind = registryTransitionKind;
exports.maybePublicKey = maybePublicKey;
exports.resolveTransitionForRegistryVersion = resolveTransitionForRegistryVersion;
exports.finalizeTransitionAccounts = finalizeTransitionAccounts;
exports.accountJson = accountJson;
exports.decodeVerifyReturnData = decodeVerifyReturnData;
exports.toAccountMetaList = toAccountMetaList;
exports.bnToNumber = bnToNumber;
const web3_js_1 = require("@solana/web3.js");
const pdas_1 = require("./pdas");
const utils_1 = require("./utils");
async function fetchAccountOrNull(program, accountName, address) {
    try {
        return (await program.account[accountName].fetch(address));
    }
    catch (error) {
        return null;
    }
}
async function accountExists(program, address) {
    const info = await program.provider.connection.getAccountInfo(address);
    return info !== null;
}
function registryTransitionKind(value) {
    if (!value || typeof value !== "object") {
        return "none";
    }
    if ("add" in value) {
        return "add";
    }
    if ("removeTail" in value) {
        return "removeTail";
    }
    if ("removeSwap" in value) {
        return "removeSwap";
    }
    return "none";
}
function maybePublicKey(value) {
    if (!value) {
        return null;
    }
    if (value instanceof web3_js_1.PublicKey) {
        return (0, utils_1.isDefaultPubkey)(value) ? null : value;
    }
    try {
        const parsed = new web3_js_1.PublicKey(value);
        return (0, utils_1.isDefaultPubkey)(parsed) ? null : parsed;
    }
    catch (error) {
        return null;
    }
}
async function resolveTransitionForRegistryVersion(program, registryState, registryVersion) {
    if (registryVersion === registryState.currentVersion) {
        return { transitionAccount: null, remainingTransition: undefined };
    }
    if (registryVersion !== registryState.previousVersion) {
        throw new Error(`Registry version ${registryVersion} is not current (${registryState.currentVersion}) or previous (${registryState.previousVersion})`);
    }
    const transitionKind = registryTransitionKind(registryState.lastTransitionType);
    if (transitionKind === "none" || transitionKind === "add") {
        return { transitionAccount: null, remainingTransition: undefined };
    }
    const transitionAddress = maybePublicKey(registryState.transitionAccount) ??
        (0, pdas_1.findRegistryTransitionPda)(program.programId, Number(registryState.previousVersion))[0];
    const transition = await fetchAccountOrNull(program, "registryTransitionAccount", transitionAddress);
    if (!transition) {
        throw new Error(`Missing registry transition account at ${transitionAddress.toBase58()}`);
    }
    const info = transitionKind === "removeTail"
        ? {
            removedOldIndex: Number(transition.removedOldIndex),
            transitionType: "removeTail",
        }
        : {
            removedOldIndex: Number(transition.removedOldIndex),
            transitionType: "removeSwap",
            movedOldIndex: Number(transition.movedOldIndex) === 0xffffffff
                ? undefined
                : Number(transition.movedOldIndex),
        };
    return { transitionAccount: transitionAddress, remainingTransition: info };
}
async function finalizeTransitionAccounts(program, registryState) {
    const transitionKind = registryTransitionKind(registryState.lastTransitionType);
    const transitionAccount = maybePublicKey(registryState.transitionAccount);
    if (transitionKind === "removeTail" || transitionKind === "removeSwap") {
        const virtualRegistryIndex = (0, pdas_1.findVirtualRegistryIndexPda)(program.programId)[0];
        return {
            transitionAccount,
            virtualRegistryIndex: (await accountExists(program, virtualRegistryIndex))
                ? virtualRegistryIndex
                : null,
        };
    }
    return {
        transitionAccount,
        virtualRegistryIndex: null,
    };
}
function accountJson(address, data) {
    return {
        address: address.toBase58(),
        account: (0, utils_1.normalizeForJson)(data),
    };
}
function decodeVerifyReturnData(bytes) {
    if (bytes.length !== 72) {
        throw new Error(`Expected 72 bytes of return data, got ${bytes.length}`);
    }
    const timestamp = Buffer.from(bytes.slice(32, 40)).readBigInt64BE(0);
    return {
        raw: `0x${Buffer.from(bytes).toString("hex")}`,
        value: `0x${Buffer.from(bytes.slice(0, 32)).toString("hex")}`,
        canonicalTimestamp: timestamp.toString(),
        roundId: `0x${Buffer.from(bytes.slice(40, 72)).toString("hex")}`,
    };
}
function toAccountMetaList(addresses) {
    return addresses.map((pubkey) => ({
        pubkey,
        isSigner: false,
        isWritable: false,
    }));
}
function bnToNumber(value) {
    return typeof value === "number" ? value : value.toNumber();
}
