"use strict";
var __createBinding = (this && this.__createBinding) || (Object.create ? (function(o, m, k, k2) {
    if (k2 === undefined) k2 = k;
    var desc = Object.getOwnPropertyDescriptor(m, k);
    if (!desc || ("get" in desc ? !m.__esModule : desc.writable || desc.configurable)) {
      desc = { enumerable: true, get: function() { return m[k]; } };
    }
    Object.defineProperty(o, k2, desc);
}) : (function(o, m, k, k2) {
    if (k2 === undefined) k2 = k;
    o[k2] = m[k];
}));
var __setModuleDefault = (this && this.__setModuleDefault) || (Object.create ? (function(o, v) {
    Object.defineProperty(o, "default", { enumerable: true, value: v });
}) : function(o, v) {
    o["default"] = v;
});
var __importStar = (this && this.__importStar) || (function () {
    var ownKeys = function(o) {
        ownKeys = Object.getOwnPropertyNames || function (o) {
            var ar = [];
            for (var k in o) if (Object.prototype.hasOwnProperty.call(o, k)) ar[ar.length] = k;
            return ar;
        };
        return ownKeys(o);
    };
    return function (mod) {
        if (mod && mod.__esModule) return mod;
        var result = {};
        if (mod != null) for (var k = ownKeys(mod), i = 0; i < k.length; i++) if (k[i] !== "default") __createBinding(result, mod, k[i]);
        __setModuleDefault(result, mod);
        return result;
    };
})();
Object.defineProperty(exports, "__esModule", { value: true });
exports.VIRTUAL_INDEX = exports.VERIFY_DATA_UPDATE_RETURN_LEN = void 0;
exports.evmUint256BitmapForFirstN = evmUint256BitmapForFirstN;
exports.evmBitmapBitSet = evmBitmapBitSet;
exports.bitmapPopcountEvm = bitmapPopcountEvm;
exports.deriveGroupBitmap = deriveGroupBitmap;
exports.deriveSelectionSeed = deriveSelectionSeed;
exports.effectiveSelectionSize = effectiveSelectionSize;
exports.deriveSignersBitmapForJob = deriveSignersBitmapForJob;
exports.generateE2eSignerMaterialFromSeed = generateE2eSignerMaterialFromSeed;
exports.signE2eDataUpdate = signE2eDataUpdate;
exports.buildSubmitArgsFromFixture = buildSubmitArgsFromFixture;
exports.expectedVerifyDataUpdateReturnBytes = expectedVerifyDataUpdateReturnBytes;
exports.signerRemainingAccounts = signerRemainingAccounts;
exports.expectedRegistryIndex = expectedRegistryIndex;
exports.signerMaterialFromFixture = signerMaterialFromFixture;
exports.describeSignedPayload = describeSignedPayload;
exports.buildBitmapFromFlag = buildBitmapFromFlag;
exports.evmSchnorrEcdsaInputs = evmSchnorrEcdsaInputs;
const anchor_1 = require("@coral-xyz/anchor");
const crypto = __importStar(require("crypto"));
const secp256k1 = __importStar(require("@noble/secp256k1"));
const secp256k1_1 = require("@noble/secp256k1");
const sha3_1 = require("@noble/hashes/sha3");
const pdas_1 = require("../pdas");
const utils_1 = require("../utils");
const MESSAGE_PREFIX = Buffer.from((0, sha3_1.keccak_256)(Buffer.from("MOLPHA_MESSAGE_V1")));
const SELECTION_SEED_PREFIX = Buffer.from((0, sha3_1.keccak_256)(Buffer.from("MOLPHA_SELECTION_V1")));
/** `keccak256("MOLPHA_SELECTION_DERIVE")` — EVM `NodeGroupBitmapLib.SELECTION_DOMAIN`. */
const SELECTION_DOMAIN = (0, sha3_1.keccak_256)(Buffer.from("MOLPHA_SELECTION_DERIVE"));
const DERIVE_GROUP_BITMAP_MAX_ROUNDS = 65536;
const SECP256K1_ORDER_N = BigInt("0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141");
exports.VERIFY_DATA_UPDATE_RETURN_LEN = 72;
exports.VIRTUAL_INDEX = 0xffffffff;
function modN(x) {
    const r = x % SECP256K1_ORDER_N;
    return r >= 0n ? r : r + SECP256K1_ORDER_N;
}
function bigintFromBytesBE(bytes) {
    return BigInt(`0x${Buffer.from(bytes).toString("hex")}`);
}
function bytes32FromBigintBE(value) {
    let normalized = value;
    if (normalized < 0n) {
        normalized = (normalized % SECP256K1_ORDER_N + SECP256K1_ORDER_N) % SECP256K1_ORDER_N;
    }
    normalized %= SECP256K1_ORDER_N;
    return Uint8Array.from(Buffer.from(normalized.toString(16).padStart(64, "0"), "hex"));
}
function keccakConcat(chunks) {
    return (0, sha3_1.keccak_256)(Buffer.concat(chunks.map((chunk) => Buffer.from(chunk))));
}
function u32be(value) {
    const out = new Uint8Array(4);
    new DataView(out.buffer).setUint32(0, value >>> 0, false);
    return out;
}
function u64be(value) {
    const out = new Uint8Array(8);
    new DataView(out.buffer).setBigUint64(0, BigInt(value), false);
    return out;
}
function u256BeFromU64(counter) {
    const out = new Uint8Array(32);
    new DataView(out.buffer).setBigUint64(24, BigInt(counter), false);
    return out;
}
function selectionHashRound(seed, counter) {
    return (0, sha3_1.keccak_256)(Buffer.concat([
        Buffer.from(seed),
        Buffer.from(SELECTION_DOMAIN),
        Buffer.from(u256BeFromU64(counter)),
    ]));
}
function fullMaskBigInt(nodeCount) {
    return nodeCount === 256 ? (1n << 256n) - 1n : (1n << BigInt(nodeCount)) - 1n;
}
function bigintToEvmBytes32(value) {
    const hex = value.toString(16).padStart(64, "0");
    return Uint8Array.from(Buffer.from(hex, "hex"));
}
function sampleWithoutReplacement(seed, nodeCount, groupSize) {
    const limit = BigInt(Math.floor(0xffffffff / nodeCount) * nodeCount);
    let bitmap = 0n;
    let selected = 0;
    let counter = 0;
    while (selected < groupSize) {
        if (counter >= DERIVE_GROUP_BITMAP_MAX_ROUNDS) {
            throw new Error("deriveGroupBitmap exceeded Keccak-256 round limit");
        }
        const digest = selectionHashRound(seed, counter);
        counter += 1;
        let word = bigintFromBytesBE(digest);
        for (let w = 0; w < 8 && selected < groupSize; w += 1) {
            const limb = Number(word >> 224n);
            word = (word << 32n) & ((1n << 256n) - 1n);
            if (BigInt(limb) < limit) {
                const pos = limb % nodeCount;
                const bit = 1n << BigInt(pos);
                if ((bitmap & bit) === 0n) {
                    bitmap |= bit;
                    selected += 1;
                }
            }
        }
    }
    return bitmap;
}
function modReduceOnce32(hash) {
    return bytes32FromBigintBE(bigintFromBytesBE(hash));
}
function isZero32(bytes) {
    return bytes.every((byte) => byte === 0);
}
function subBE256(a, b) {
    const av = bigintFromBytesBE(a);
    const bv = bigintFromBytesBE(b);
    const mod = 1n << 256n;
    return bytes32FromBigintBE((av - bv + mod) % mod);
}
function mulMod256(a, b) {
    const mod = 1n << 256n;
    const av = bigintFromBytesBE(a) % mod;
    const bv = bigintFromBytesBE(b) % mod;
    return bytes32FromBigintBE((av * bv) % mod);
}
function evmUint256BitmapForFirstN(count) {
    const bitmap = new Uint8Array(32);
    for (let index = 0; index < count; index += 1) {
        const byteIndex = 31 - (index >> 3);
        bitmap[byteIndex] |= 1 << (index & 7);
    }
    return bitmap;
}
function evmBitmapBitSet(signersBitmap, bit) {
    const byteIndex = 31 - Math.floor(bit / 8);
    const bitInByte = bit % 8;
    const byte = signersBitmap[byteIndex] ?? 0;
    return ((byte >> bitInByte) & 1) === 1;
}
function bitmapPopcountEvm(signersBitmap) {
    let count = 0;
    for (let bit = 0; bit < 256; bit += 1) {
        if (evmBitmapBitSet(signersBitmap, bit)) {
            count += 1;
        }
    }
    return count;
}
/** Port of `NodeGroupBitmapLib.derive` / `derive_group_bitmap` in the on-chain program. */
function deriveGroupBitmap(seed, nodeCount, groupSize) {
    if (nodeCount <= 0) {
        throw new Error("nodeCount must be > 0");
    }
    if (nodeCount > 256) {
        throw new Error("nodeCount cannot exceed 256");
    }
    if (groupSize > nodeCount) {
        throw new Error("groupSize cannot exceed nodeCount");
    }
    if (groupSize === 0) {
        return new Uint8Array(32);
    }
    if (groupSize === nodeCount) {
        return bigintToEvmBytes32(fullMaskBigInt(nodeCount));
    }
    const bitmap = groupSize > nodeCount / 2
        ? fullMaskBigInt(nodeCount) ^
            sampleWithoutReplacement(seed, nodeCount, nodeCount - groupSize)
        : sampleWithoutReplacement(seed, nodeCount, groupSize);
    return bigintToEvmBytes32(bitmap);
}
function deriveSelectionSeed(jobId, registryVersion, canonicalTimestamp) {
    return (0, sha3_1.keccak_256)(Buffer.concat([
        SELECTION_SEED_PREFIX,
        Buffer.from(jobId),
        Buffer.from(u32be(registryVersion)),
        Buffer.from(u64be(canonicalTimestamp)),
    ]));
}
/** Mirrors `effective_selection_size` in `programs/molpha/src/utils/bitmap.rs`. */
function effectiveSelectionSize(signaturesRequired, redundancyBuffer, nodeCount) {
    return Math.min(nodeCount, signaturesRequired + redundancyBuffer);
}
function deriveSignersBitmapForJob(jobId, registryVersion, nodeCount, signaturesRequired, redundancyBuffer = 2, canonicalTimestamp = 0) {
    const selectionSize = effectiveSelectionSize(signaturesRequired, redundancyBuffer, nodeCount);
    return deriveGroupBitmap(deriveSelectionSeed(jobId, registryVersion, canonicalTimestamp), nodeCount, selectionSize);
}
function generateE2eSignerMaterialFromSeed(seed, signerCount) {
    const privKeys = [];
    const compressedPubkeys = [];
    for (let index = 0; index < signerCount; index += 1) {
        const skSeed = (0, sha3_1.keccak_256)(Buffer.concat([Buffer.from(seed), Buffer.from([signerCount]), Buffer.from([index])]));
        const ecdh = crypto.createECDH("secp256k1");
        ecdh.setPrivateKey(Buffer.from(skSeed));
        const compressed = ecdh.getPublicKey(undefined, "compressed");
        privKeys.push(bigintFromBytesBE(skSeed));
        compressedPubkeys.push(Uint8Array.from(compressed));
    }
    return { privKeys, compressedPubkeys };
}
function computeMessageHash(params) {
    return keccakConcat([
        MESSAGE_PREFIX,
        params.jobId,
        u32be(params.registryVersion),
        u32be(params.signaturesRequired),
        params.signersBitmap,
        params.value,
        u64be(params.canonicalTimestamp),
    ]);
}
function commitmentAddrFromPoint(point) {
    const uncompressed = point.toRawBytes(false);
    const digest = (0, sha3_1.keccak_256)(uncompressed.subarray(1));
    return digest.subarray(12, 32);
}
function schnorrSignForVerifier(params) {
    const px = params.coalitionPubkeyCompressed.subarray(1, 33);
    const recoveryId = params.coalitionPubkeyCompressed[0] & 1;
    let k = 0n;
    while (k === 0n) {
        k = modN(bigintFromBytesBE(secp256k1.utils.randomPrivateKey()));
    }
    const commitmentPoint = secp256k1_1.Point.BASE.multiply(k);
    const commitmentAddr = commitmentAddrFromPoint(commitmentPoint);
    const challengeHash = keccakConcat([
        px,
        Uint8Array.from([recoveryId]),
        params.messageHash,
        commitmentAddr,
    ]);
    const challenge = modN(bigintFromBytesBE(challengeHash));
    const s = modN(k + challenge * modN(params.coalitionPrivkey));
    return { s: bytes32FromBigintBE(s), commitmentAddr };
}
function signE2eDataUpdate(registryVersion, signerMaterial, signerCountOrBitmap, jobId, dataUpdate, signaturesRequired) {
    const signersBitmap = typeof signerCountOrBitmap === "number"
        ? evmUint256BitmapForFirstN(signerCountOrBitmap)
        : signerCountOrBitmap;
    const signerCount = bitmapPopcountEvm(signersBitmap);
    const required = signaturesRequired ?? signerCount;
    const messageHash = computeMessageHash({
        jobId,
        registryVersion,
        signaturesRequired: required,
        signersBitmap,
        value: dataUpdate.value,
        canonicalTimestamp: dataUpdate.canonicalTimestamp,
    });
    let coalitionPoint = secp256k1_1.Point.ZERO;
    let coalitionSecretKey = 0n;
    for (let bit = 0; bit < 256; bit += 1) {
        if (!evmBitmapBitSet(signersBitmap, bit)) {
            continue;
        }
        if (bit >= signerMaterial.privKeys.length) {
            throw new Error(`Missing signer material for registry index ${bit}`);
        }
        coalitionSecretKey = modN(coalitionSecretKey + signerMaterial.privKeys[bit]);
        coalitionPoint = coalitionPoint.add(secp256k1_1.Point.fromHex(Buffer.from(signerMaterial.compressedPubkeys[bit]).toString("hex")));
    }
    const coalitionCompressed = coalitionPoint.toRawBytes(true);
    const { s, commitmentAddr } = schnorrSignForVerifier({
        coalitionPubkeyCompressed: coalitionCompressed,
        messageHash,
        coalitionPrivkey: coalitionSecretKey,
    });
    return {
        submitArgs: {
            jobId: Array.from(jobId),
            registryVersion,
            signersBitmap: Array.from(signersBitmap),
            value: Array.from(dataUpdate.value),
            canonicalTimestamp: new anchor_1.BN(dataUpdate.canonicalTimestamp),
            aggSigR: Array.from(new Uint8Array(32)),
            aggSigS: Array.from(s),
            commitmentAddr: Array.from(commitmentAddr),
        },
        signersBitmap,
        aggSigS: s,
        commitmentAddr,
    };
}
function buildSubmitArgsFromFixture(fixture) {
    const signersBitmap = uint256BE(BigInt(fixture.schnorrSignature.signersBitmap));
    return {
        jobId: Array.from((0, utils_1.hexToBytes)(fixture.dataUpdate.jobId, 32, "job id")),
        registryVersion: fixture.dataUpdate.registryVersion,
        signersBitmap: Array.from(signersBitmap),
        value: Array.from((0, utils_1.hexToBytes)(fixture.dataUpdate.value, 32, "value")),
        canonicalTimestamp: new anchor_1.BN(fixture.dataUpdate.canonicalTimestamp),
        aggSigR: Array.from(new Uint8Array(32)),
        aggSigS: Array.from((0, utils_1.hexToBytes)(fixture.schnorrSignature.signature, 32, "signature")),
        commitmentAddr: Array.from((0, utils_1.hexToBytes)(fixture.schnorrSignature.commitment.toLowerCase(), 20, "commitment")),
    };
}
function uint256BE(value) {
    const out = new Uint8Array(32);
    let next = value;
    for (let index = 31; index >= 0 && next > 0n; index -= 1) {
        out[index] = Number(next & 0xffn);
        next >>= 8n;
    }
    return out;
}
function expectedVerifyDataUpdateReturnBytes(value, canonicalTimestamp) {
    const out = new Uint8Array(exports.VERIFY_DATA_UPDATE_RETURN_LEN);
    out.set(value.subarray(0, 32), 0);
    out.set(u64be(canonicalTimestamp), 32);
    return out;
}
function signerRemainingAccounts(programId, signersBitmap, transition) {
    const remaining = [];
    for (let bit = 0; bit < 256; bit += 1) {
        if (!evmBitmapBitSet(signersBitmap, bit)) {
            continue;
        }
        const effectiveIndex = expectedRegistryIndex(bit, transition);
        remaining.push({
            pubkey: (0, pdas_1.findRegistryIndexPda)(programId, effectiveIndex)[0],
            isSigner: false,
            isWritable: false,
        });
    }
    return remaining;
}
function expectedRegistryIndex(bit, transition) {
    if (!transition) {
        return bit;
    }
    if (bit === transition.removedOldIndex) {
        return exports.VIRTUAL_INDEX;
    }
    if (transition.transitionType === "removeSwap" &&
        transition.movedOldIndex !== undefined &&
        bit === transition.movedOldIndex) {
        return transition.removedOldIndex;
    }
    return bit;
}
function signerMaterialFromFixture(fixture) {
    const compressedPubkeys = fixture.nodePubkeys.map((value) => (0, utils_1.hexToBytes)(value, 33, "node pubkey"));
    const scalarStrings = fixture.secretKeys ?? fixture.nodePrivkeys;
    if (!scalarStrings || scalarStrings.length === 0) {
        throw new Error("Fixture must contain secretKeys or nodePrivkeys");
    }
    const privKeys = scalarStrings.map((value) => value.startsWith("0x") ? bigintFromBytesBE((0, utils_1.hexToBytes)(value, 32, "node privkey")) : (0, utils_1.parseBigIntString)(value, "secret key"));
    return { privKeys, compressedPubkeys };
}
function describeSignedPayload(signed) {
    return {
        submitArgs: {
            ...signed.submitArgs,
            canonicalTimestamp: signed.submitArgs.canonicalTimestamp.toString(),
        },
        signersBitmapHex: (0, utils_1.bytesToHex)(signed.signersBitmap),
        aggSigSHex: (0, utils_1.bytesToHex)(signed.aggSigS),
        commitmentAddrHex: (0, utils_1.bytesToHex)(signed.commitmentAddr),
    };
}
function buildBitmapFromFlag(value) {
    return (0, utils_1.hexToBytes)(value, 32, "signers bitmap");
}
const SECP256K1_ORDER_BYTES = Uint8Array.from(Buffer.from("fffffffffffffffffffffffffffffffebaaedce6af48a03bbfd25e8cd0364141", "hex"));
function evmSchnorrEcdsaInputs(params) {
    const parityPrefix = params.coalitionPubkeyCompressed[0];
    if (parityPrefix !== 0x02 && parityPrefix !== 0x03) {
        throw new Error("invalid compressed pubkey prefix");
    }
    const recoveryId = parityPrefix & 1;
    const px = params.coalitionPubkeyCompressed.subarray(1, 33);
    const pxModN = modReduceOnce32(px);
    if (isZero32(pxModN)) {
        throw new Error("px mod n is zero");
    }
    const challengeHash = keccakConcat([
        px,
        Uint8Array.from([recoveryId]),
        params.messageHash,
        params.commitment,
    ]);
    const challenge = modReduceOnce32(challengeHash);
    const sigReduced = modReduceOnce32(params.signature);
    const msgMul = mulMod256(sigReduced, pxModN);
    const eEcdsa = subBE256(SECP256K1_ORDER_BYTES, msgMul);
    const ecdsaSMul = mulMod256(challenge, pxModN);
    const sEcdsa = subBE256(SECP256K1_ORDER_BYTES, ecdsaSMul);
    if (isZero32(sEcdsa)) {
        throw new Error("ecdsa s is zero");
    }
    const ecdsaSig = new Uint8Array(64);
    ecdsaSig.set(pxModN, 0);
    ecdsaSig.set(sEcdsa, 32);
    return { recoveryId, ecdsaSig, ecdsaHash: eEcdsa };
}
