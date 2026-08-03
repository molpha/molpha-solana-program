#!/usr/bin/env node
/* eslint-disable no-console */

// Compatibility entry point for the pre-registry-refactor benchmark name. The maintained
// benchmark exercises the same data-update verification path with the current Node accounts,
// registration ABI, and lookup-table setup.
require("./cu-bench-data-update-localnet.js");
