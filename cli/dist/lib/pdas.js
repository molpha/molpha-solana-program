"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.VIRTUAL_INDEX = exports.FEED_SEED = exports.JOB_SEED = exports.SUBSCRIPTION_SEED = exports.PLAN_SEED = exports.NODE_SEED = exports.REGISTRY_INDEX_SEED = exports.REGISTRY_STATE_SEED = exports.PROTOCOL_CONFIG_SEED = void 0;
exports.findProtocolConfigPda = findProtocolConfigPda;
exports.findRegistryStatePda = findRegistryStatePda;
exports.findRegistryIndexPda = findRegistryIndexPda;
exports.findVirtualRegistryIndexPda = findVirtualRegistryIndexPda;
exports.findNodePda = findNodePda;
exports.findPlanPda = findPlanPda;
exports.findSubscriptionPda = findSubscriptionPda;
exports.findJobPda = findJobPda;
exports.findFeedPda = findFeedPda;
const web3_js_1 = require("@solana/web3.js");
const utils_1 = require("./utils");
exports.PROTOCOL_CONFIG_SEED = Buffer.from("molpha_config");
exports.REGISTRY_STATE_SEED = Buffer.from("molpha_registry");
exports.REGISTRY_INDEX_SEED = Buffer.from("molpha_registry_index");
exports.NODE_SEED = Buffer.from("molpha_node");
exports.PLAN_SEED = Buffer.from("molpha_plan");
exports.SUBSCRIPTION_SEED = Buffer.from("molpha_subscription");
exports.JOB_SEED = Buffer.from("molpha_job");
exports.FEED_SEED = Buffer.from("molpha_feed");
exports.VIRTUAL_INDEX = 0xffffffff;
function findProtocolConfigPda(programId) {
    return web3_js_1.PublicKey.findProgramAddressSync([exports.PROTOCOL_CONFIG_SEED], programId);
}
function findRegistryStatePda(programId) {
    return web3_js_1.PublicKey.findProgramAddressSync([exports.REGISTRY_STATE_SEED], programId);
}
function findRegistryIndexPda(programId, index) {
    return web3_js_1.PublicKey.findProgramAddressSync([exports.REGISTRY_INDEX_SEED, (0, utils_1.u32Le)(index)], programId);
}
function findVirtualRegistryIndexPda(programId) {
    return findRegistryIndexPda(programId, exports.VIRTUAL_INDEX);
}
function findNodePda(programId, nodeId) {
    return web3_js_1.PublicKey.findProgramAddressSync([exports.NODE_SEED, Buffer.from(nodeId)], programId);
}
function findPlanPda(programId, planId) {
    return web3_js_1.PublicKey.findProgramAddressSync([exports.PLAN_SEED, Buffer.from([planId])], programId);
}
function findSubscriptionPda(programId, owner) {
    return web3_js_1.PublicKey.findProgramAddressSync([exports.SUBSCRIPTION_SEED, owner.toBuffer()], programId);
}
function findJobPda(programId, jobId) {
    return web3_js_1.PublicKey.findProgramAddressSync([exports.JOB_SEED, Buffer.from(jobId)], programId);
}
function findFeedPda(programId, jobId) {
    return web3_js_1.PublicKey.findProgramAddressSync([exports.FEED_SEED, Buffer.from(jobId)], programId);
}
