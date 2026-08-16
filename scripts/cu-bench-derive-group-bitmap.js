/* eslint-disable no-console */
/**
 * Formerly measured on-chain `bench_derive_group_bitmap` / `bench_noop` CU deltas.
 *
 * Those feature-gated probe instructions were removed when the registry moved to
 * immutable versioned snapshots. Off-chain selection math still lives in
 * `scripts/lib/cu-bench-data-update-common.js` (`deriveGroupBitmap`).
 *
 * Use `yarn cu:node-registry:bankrun` for registry snapshot / resolution CU gates,
 * or `yarn cu:data-update:bankrun` for end-to-end verify/submit CU.
 */
console.error(
  [
    "cu-bench-derive-group-bitmap: on-chain probe instructions are no longer in the program.",
    "Use yarn cu:data-update:bankrun or yarn cu:node-registry:bankrun instead.",
  ].join("\n")
);
process.exit(1);
