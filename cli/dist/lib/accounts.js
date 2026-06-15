"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.fetchAccountOrNull = fetchAccountOrNull;
exports.accountExists = accountExists;
exports.registryTransitionKind = registryTransitionKind;
exports.removeTransitionInfoFromRegistry = removeTransitionInfoFromRegistry;
exports.resolveTransitionForRegistryVersion = resolveTransitionForRegistryVersion;
exports.finalizeTransitionAccounts = finalizeTransitionAccounts;
exports.accountJson = accountJson;
exports.decodeVerifyReturnData = decodeVerifyReturnData;
exports.toAccountMetaList = toAccountMetaList;
exports.bnToNumber = bnToNumber;
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
function removeTransitionInfoFromRegistry(registryState) {
    const transitionKind = registryTransitionKind(registryState.lastTransitionType);
    if (transitionKind !== "removeTail" && transitionKind !== "removeSwap") {
        return undefined;
    }
    const removedOldIndex = Number(registryState.removedOldIndex);
    if (transitionKind === "removeTail") {
        return {
            removedOldIndex,
            transitionType: "removeTail",
        };
    }
    const movedOldIndex = Number(registryState.movedOldIndex);
    return {
        removedOldIndex,
        transitionType: "removeSwap",
        movedOldIndex: movedOldIndex === 0xffffffff ? undefined : movedOldIndex,
    };
}
async function resolveTransitionForRegistryVersion(program, registryState, registryVersion) {
    if (registryVersion === registryState.currentVersion) {
        return { remainingTransition: undefined };
    }
    if (registryVersion !== registryState.previousVersion) {
        throw new Error(`Registry version ${registryVersion} is not current (${registryState.currentVersion}) or previous (${registryState.previousVersion})`);
    }
    const transitionKind = registryTransitionKind(registryState.lastTransitionType);
    if (transitionKind === "none" || transitionKind === "add") {
        return { remainingTransition: undefined };
    }
    const remainingTransition = removeTransitionInfoFromRegistry(registryState);
    if (!remainingTransition) {
        throw new Error("Registry state is missing remove-transition metadata");
    }
    return { remainingTransition };
}
async function finalizeTransitionAccounts(program, registryState) {
    const transitionKind = registryTransitionKind(registryState.lastTransitionType);
    if (transitionKind === "removeTail" || transitionKind === "removeSwap") {
        const virtualRegistryIndex = (0, pdas_1.findVirtualRegistryIndexPda)(program.programId)[0];
        return {
            virtualRegistryIndex: (await accountExists(program, virtualRegistryIndex))
                ? virtualRegistryIndex
                : null,
        };
    }
    return {
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
