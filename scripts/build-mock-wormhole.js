#!/usr/bin/env node
/**
 * Build mock_wormhole_bridge.so into ./target/deploy for bankrun tests.
 * Forces CARGO_TARGET_DIR to the repo target dir (avoids broken sandbox overrides).
 */
const { execSync } = require("child_process");
const path = require("path");

const root = path.join(__dirname, "..");
const targetDir = path.join(root, "target");
execSync("cargo build-sbf -- --package mock_wormhole_bridge", {
  stdio: "inherit",
  cwd: root,
  env: { ...process.env, CARGO_TARGET_DIR: targetDir },
});
