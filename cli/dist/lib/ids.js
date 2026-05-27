"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.JOB_ID_PREFIX = exports.NODE_ID_PREFIX = void 0;
exports.deriveNodeId = deriveNodeId;
exports.deriveJobId = deriveJobId;
const sha3_1 = require("@noble/hashes/sha3");
exports.NODE_ID_PREFIX = Buffer.from("MOLPHA_NODE_ID_V1");
exports.JOB_ID_PREFIX = Buffer.from("MOLPHA_JOB_V1");
function deriveNodeId(owner, secp256k1PubkeyCompressed, registrationSalt) {
    return (0, sha3_1.keccak_256)(Buffer.concat([
        exports.NODE_ID_PREFIX,
        owner.toBuffer(),
        Buffer.from(secp256k1PubkeyCompressed),
        Buffer.from(registrationSalt),
    ]));
}
function deriveJobId(owner, apiConfigHash) {
    return (0, sha3_1.keccak_256)(Buffer.concat([exports.JOB_ID_PREFIX, owner.toBuffer(), Buffer.from(apiConfigHash)]));
}
