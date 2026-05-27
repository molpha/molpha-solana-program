"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.buildRegisterNodeArgs = buildRegisterNodeArgs;
const VALID_POP_SIG_S = Uint8Array.from(Buffer.concat([Buffer.alloc(31, 0), Buffer.from([1])]));
function buildRegisterNodeArgs(params) {
    const secp = params.secp256k1PubkeyCompressed;
    return {
        newIndex: params.newIndex,
        registrationSalt: Array.from(params.registrationSalt),
        secp256K1PubkeyCompressed: Array.from(secp),
        encryptionPubkey: Array.from(params.encryptionPubkey ?? secp),
        peerIdHash: Array.from(params.peerIdHash ?? Buffer.alloc(32, params.newIndex + 2)),
        metadataHash: Array.from(params.metadataHash ?? Buffer.alloc(32, params.newIndex + 3)),
        popSigR: Array.from(Buffer.alloc(32, 0)),
        popSigS: Array.from(VALID_POP_SIG_S),
        securityDeposit: params.securityDeposit,
    };
}
