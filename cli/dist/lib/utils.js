"use strict";
var __importDefault = (this && this.__importDefault) || function (mod) {
    return (mod && mod.__esModule) ? mod : { "default": mod };
};
Object.defineProperty(exports, "__esModule", { value: true });
exports.expandHome = expandHome;
exports.resolveFromCwd = resolveFromCwd;
exports.readUtf8 = readUtf8;
exports.readJsonFile = readJsonFile;
exports.writeJson = writeJson;
exports.normalizeForJson = normalizeForJson;
exports.loadKeypair = loadKeypair;
exports.parsePublicKey = parsePublicKey;
exports.parseInteger = parseInteger;
exports.parseBigIntString = parseBigIntString;
exports.bn = bn;
exports.optionalNumber = optionalNumber;
exports.hexToBytes = hexToBytes;
exports.maybeHexToBytes = maybeHexToBytes;
exports.bytesToHex = bytesToHex;
exports.fixedBytes = fixedBytes;
exports.readHexFile = readHexFile;
exports.parseHexOrFile = parseHexOrFile;
exports.u32Le = u32Le;
exports.requireValue = requireValue;
exports.defaultWalletPath = defaultWalletPath;
exports.optionOrConfig = optionOrConfig;
exports.isDefaultPubkey = isDefaultPubkey;
const anchor_1 = require("@coral-xyz/anchor");
const web3_js_1 = require("@solana/web3.js");
const fs_1 = __importDefault(require("fs"));
const os_1 = __importDefault(require("os"));
const path_1 = __importDefault(require("path"));
function expandHome(input) {
    if (input === "~") {
        return os_1.default.homedir();
    }
    if (input.startsWith("~/")) {
        return path_1.default.join(os_1.default.homedir(), input.slice(2));
    }
    return input;
}
function resolveFromCwd(input) {
    const expanded = expandHome(input);
    return path_1.default.isAbsolute(expanded) ? expanded : path_1.default.resolve(process.cwd(), expanded);
}
function readUtf8(filePath) {
    return fs_1.default.readFileSync(resolveFromCwd(filePath), "utf8");
}
function readJsonFile(filePath) {
    return JSON.parse(readUtf8(filePath));
}
function writeJson(value) {
    process.stdout.write(`${JSON.stringify(normalizeForJson(value), null, 2)}\n`);
}
function normalizeForJson(value) {
    if (value === null || value === undefined) {
        return null;
    }
    if (typeof value === "string" || typeof value === "number" || typeof value === "boolean") {
        return value;
    }
    if (typeof value === "bigint") {
        return value.toString();
    }
    if (value instanceof web3_js_1.PublicKey) {
        return value.toBase58();
    }
    if (anchor_1.BN.isBN(value)) {
        return value.toString(10);
    }
    if (value instanceof Uint8Array || Buffer.isBuffer(value)) {
        return `0x${Buffer.from(value).toString("hex")}`;
    }
    if (Array.isArray(value)) {
        return value.map((item) => normalizeForJson(item));
    }
    if (typeof value === "object") {
        const out = {};
        for (const [key, entry] of Object.entries(value)) {
            out[key] = normalizeForJson(entry);
        }
        return out;
    }
    return String(value);
}
function loadKeypair(filePath) {
    const secret = readJsonFile(filePath);
    return web3_js_1.Keypair.fromSecretKey(Uint8Array.from(secret));
}
function parsePublicKey(value, label = "public key") {
    try {
        return new web3_js_1.PublicKey(value);
    }
    catch (error) {
        throw new Error(`Invalid ${label}: ${value}`);
    }
}
function parseInteger(value, label) {
    const parsed = Number(value);
    if (!Number.isFinite(parsed) || !Number.isInteger(parsed)) {
        throw new Error(`Invalid ${label}: ${value}`);
    }
    return parsed;
}
function parseBigIntString(value, label) {
    try {
        return BigInt(value);
    }
    catch (error) {
        throw new Error(`Invalid ${label}: ${value}`);
    }
}
function bn(value, label) {
    try {
        return new anchor_1.BN(typeof value === "string" ? value : value.toString());
    }
    catch (error) {
        throw new Error(`Invalid ${label}: ${value}`);
    }
}
function optionalNumber(value, fallback) {
    if (value === undefined) {
        return fallback;
    }
    return parseInteger(value, "number");
}
function hexToBytes(value, expectedLength, label = "hex") {
    const clean = value.startsWith("0x") ? value.slice(2) : value;
    if (!/^[0-9a-fA-F]*$/.test(clean) || clean.length % 2 !== 0) {
        throw new Error(`Invalid ${label}: ${value}`);
    }
    const bytes = Uint8Array.from(Buffer.from(clean, "hex"));
    if (expectedLength !== undefined && bytes.length !== expectedLength) {
        throw new Error(`${label} must be ${expectedLength} bytes, got ${bytes.length}`);
    }
    return bytes;
}
function maybeHexToBytes(value, expectedLength, label) {
    if (!value) {
        return undefined;
    }
    return hexToBytes(value, expectedLength, label);
}
function bytesToHex(bytes) {
    return `0x${Buffer.from(bytes).toString("hex")}`;
}
function fixedBytes(bytes, length, label) {
    if (bytes.length !== length) {
        throw new Error(`${label} must be ${length} bytes, got ${bytes.length}`);
    }
    return bytes;
}
function readHexFile(filePath, expectedLength, label) {
    return hexToBytes(readUtf8(filePath).trim(), expectedLength, label);
}
function parseHexOrFile(value, filePath, expectedLength, label) {
    if (value) {
        return hexToBytes(value, expectedLength, label);
    }
    if (filePath) {
        return readHexFile(filePath, expectedLength, label);
    }
    throw new Error(`Missing ${label}`);
}
function u32Le(value) {
    const out = Buffer.alloc(4);
    out.writeUInt32LE(value >>> 0, 0);
    return out;
}
function requireValue(value, message) {
    if (value === undefined || value === null) {
        throw new Error(message);
    }
    return value;
}
function defaultWalletPath() {
    return process.env.ANCHOR_WALLET || path_1.default.join(os_1.default.homedir(), ".config/solana/id.json");
}
function optionOrConfig(optionValue, configValue) {
    return optionValue !== undefined ? optionValue : configValue;
}
function isDefaultPubkey(value) {
    return value.equals(web3_js_1.PublicKey.default);
}
