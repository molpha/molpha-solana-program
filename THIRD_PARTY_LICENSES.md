# Third-party licenses

Reviewed 8 October 2026 against `Cargo.lock` (resolved with locked offline Cargo metadata) and the installed JavaScript dependency tree. This is a source-release inventory; dependencies keep their own licenses and are not relicensed under ISC.

## Distribution notes

- `rpc-websockets` 9.1.2 declares LGPL-3.0-only and ships a notice referencing [LGPLv3](https://www.gnu.org/licenses/lgpl-3.0.html). It is a transitive runtime dependency of `@solana/web3.js`. A bundled CLI release needs a review of its LGPL distribution requirements and notices. This source release does not vendor `node_modules` or ship a bundled CLI.
- `text-encoding-utf-8` 1.0.2 omits package license metadata; its installed `LICENSE.md` contains the Unlicense and notes material derived from the WHATWG Encoding Standard. Preserve that notice when redistributing it.
- Rust dependencies include Unicode-3.0 and Apache LLVM exceptions, as well as permissive licenses. Choose a permissible alternative where a dependency offers a license choice; do not treat an `OR` expression as requiring every alternative.
- Preserve original copyright, attribution and license files when distributing dependencies or compiled artifacts. Dependency upgrades require a fresh inventory.

## Rust dependency inventory

| Package | Version | Declared license |
| --- | --- | --- |
| `aead` | `0.5.2` | MIT OR Apache-2.0 |
| `aes` | `0.8.4` | MIT OR Apache-2.0 |
| `aes-gcm-siv` | `0.11.1` | MIT OR Apache-2.0 |
| `agave-feature-set` | `4.2.1` | Apache-2.0 |
| `agave-precompiles` | `4.2.1` | Apache-2.0 |
| `agave-reserved-account-keys` | `4.2.1` | Apache-2.0 |
| `agave-transaction-view` | `4.2.1` | Apache-2.0 |
| `ahash` | `0.8.12` | MIT OR Apache-2.0 |
| `allocator-api2` | `0.2.21` | MIT OR Apache-2.0 |
| `anchor-attribute-access-control` | `1.2.0` | Apache-2.0 |
| `anchor-attribute-account` | `1.2.0` | Apache-2.0 |
| `anchor-attribute-constant` | `1.2.0` | Apache-2.0 |
| `anchor-attribute-error` | `1.2.0` | Apache-2.0 |
| `anchor-attribute-event` | `1.2.0` | Apache-2.0 |
| `anchor-attribute-program` | `1.2.0` | Apache-2.0 |
| `anchor-derive-accounts` | `1.2.0` | Apache-2.0 |
| `anchor-derive-serde` | `1.2.0` | Apache-2.0 |
| `anchor-derive-space` | `1.2.0` | Apache-2.0 |
| `anchor-lang` | `1.2.0` | Apache-2.0 |
| `anchor-lang-error` | `1.2.0` | Apache-2.0 |
| `anchor-lang-idl` | `0.1.4` | Apache-2.0 |
| `anchor-lang-idl-spec` | `0.1.0` | Apache-2.0 |
| `anchor-spl` | `1.2.0` | Apache-2.0 |
| `anchor-syn` | `1.2.0` | Apache-2.0 |
| `ansi_term` | `0.12.1` | MIT |
| `anyhow` | `1.0.102` | MIT OR Apache-2.0 |
| `ark-bn254` | `0.4.0` | MIT/Apache-2.0 |
| `ark-bn254` | `0.5.0` | MIT OR Apache-2.0 |
| `ark-ec` | `0.4.2` | MIT/Apache-2.0 |
| `ark-ec` | `0.5.0` | MIT OR Apache-2.0 |
| `ark-ff` | `0.4.2` | MIT/Apache-2.0 |
| `ark-ff` | `0.5.0` | MIT OR Apache-2.0 |
| `ark-ff-asm` | `0.4.2` | MIT/Apache-2.0 |
| `ark-ff-asm` | `0.5.0` | MIT OR Apache-2.0 |
| `ark-ff-macros` | `0.4.2` | MIT/Apache-2.0 |
| `ark-ff-macros` | `0.5.0` | MIT OR Apache-2.0 |
| `ark-poly` | `0.4.2` | MIT/Apache-2.0 |
| `ark-poly` | `0.5.0` | MIT OR Apache-2.0 |
| `ark-serialize` | `0.4.2` | MIT/Apache-2.0 |
| `ark-serialize` | `0.5.0` | MIT OR Apache-2.0 |
| `ark-serialize-derive` | `0.4.2` | MIT/Apache-2.0 |
| `ark-serialize-derive` | `0.5.0` | MIT OR Apache-2.0 |
| `ark-std` | `0.4.0` | MIT/Apache-2.0 |
| `ark-std` | `0.5.0` | MIT/Apache-2.0 |
| `arrayref` | `0.3.9` | BSD-2-Clause |
| `arrayvec` | `0.7.8` | MIT OR Apache-2.0 |
| `ascii` | `0.9.3` | Apache-2.0 / MIT |
| `autocfg` | `1.5.1` | Apache-2.0 OR MIT |
| `base16ct` | `0.2.0` | Apache-2.0 OR MIT |
| `base64` | `0.21.7` | MIT OR Apache-2.0 |
| `base64` | `0.22.1` | MIT OR Apache-2.0 |
| `base64` | `0.23.1` | MIT OR Apache-2.0 |
| `base64ct` | `1.8.3` | Apache-2.0 OR MIT |
| `bincode` | `1.3.3` | MIT |
| `bitflags` | `2.13.0` | MIT OR Apache-2.0 |
| `bitvec` | `1.1.1` | MIT |
| `blake3` | `1.8.7` | CC0-1.0 OR Apache-2.0 OR Apache-2.0 WITH LLVM-exception |
| `block-buffer` | `0.10.4` | MIT OR Apache-2.0 |
| `block-buffer` | `0.12.1` | MIT OR Apache-2.0 |
| `block-buffer` | `0.9.0` | MIT OR Apache-2.0 |
| `blst` | `0.3.17` | Apache-2.0 |
| `blstrs` | `0.7.1` | MIT/Apache-2.0 |
| `borsh` | `1.7.0` | MIT OR Apache-2.0 |
| `borsh-derive` | `1.7.0` | Apache-2.0 |
| `bs58` | `0.5.1` | MIT/Apache-2.0 |
| `bumpalo` | `3.20.3` | MIT OR Apache-2.0 |
| `bv` | `0.11.1` | MIT/Apache-2.0 |
| `byte-slice-cast` | `1.2.3` | MIT |
| `bytemuck` | `1.25.0` | Zlib OR Apache-2.0 OR MIT |
| `bytemuck_derive` | `1.10.2` | Zlib OR Apache-2.0 OR MIT |
| `byteorder` | `1.5.0` | Unlicense OR MIT |
| `bytes` | `1.12.0` | MIT |
| `cc` | `1.4.4` | MIT OR Apache-2.0 |
| `cfg-if` | `1.0.4` | MIT OR Apache-2.0 |
| `cfg_aliases` | `0.2.1` | MIT |
| `cfg_eval` | `0.1.2` | Zlib OR MIT OR Apache-2.0 |
| `cipher` | `0.4.4` | MIT OR Apache-2.0 |
| `cmov` | `0.5.4` | Apache-2.0 OR MIT |
| `combine` | `3.8.1` | MIT |
| `const-crypto` | `0.3.0` | MIT OR Apache-2.0 |
| `const-oid` | `0.10.2` | Apache-2.0 OR MIT |
| `const-oid` | `0.9.6` | Apache-2.0 OR MIT |
| `constant_time_eq` | `0.4.2` | CC0-1.0 OR MIT-0 OR Apache-2.0 |
| `cpufeatures` | `0.2.17` | MIT OR Apache-2.0 |
| `cpufeatures` | `0.3.0` | MIT OR Apache-2.0 |
| `crunchy` | `0.2.4` | MIT |
| `crypto-bigint` | `0.5.5` | Apache-2.0 OR MIT |
| `crypto-common` | `0.1.6` | MIT OR Apache-2.0 |
| `crypto-common` | `0.2.1` | MIT OR Apache-2.0 |
| `crypto-mac` | `0.8.0` | MIT OR Apache-2.0 |
| `ctr` | `0.9.2` | MIT OR Apache-2.0 |
| `ctutils` | `0.4.2` | Apache-2.0 OR MIT |
| `curve25519-dalek` | `3.2.0` | BSD-3-Clause |
| `curve25519-dalek` | `4.1.3` | BSD-3-Clause |
| `curve25519-dalek-derive` | `0.1.1` | MIT/Apache-2.0 |
| `darling` | `0.23.0` | MIT |
| `darling_core` | `0.23.0` | MIT |
| `darling_macro` | `0.23.0` | MIT |
| `der` | `0.7.10` | Apache-2.0 OR MIT |
| `derivation-path` | `0.2.0` | MIT OR Apache-2.0 |
| `derivative` | `2.2.0` | MIT/Apache-2.0 |
| `digest` | `0.10.7` | MIT OR Apache-2.0 |
| `digest` | `0.11.3` | MIT OR Apache-2.0 |
| `digest` | `0.9.0` | MIT OR Apache-2.0 |
| `eager` | `0.1.0` | MIT |
| `ecdsa` | `0.16.9` | Apache-2.0 OR MIT |
| `ed25519` | `1.5.3` | Apache-2.0 OR MIT |
| `ed25519` | `2.2.3` | Apache-2.0 OR MIT |
| `ed25519-dalek` | `1.0.1` | BSD-3-Clause |
| `ed25519-dalek` | `2.2.0` | BSD-3-Clause |
| `educe` | `0.6.0` | MIT |
| `either` | `1.16.0` | MIT OR Apache-2.0 |
| `elliptic-curve` | `0.13.8` | Apache-2.0 OR MIT |
| `enum-iterator` | `2.3.0` | 0BSD |
| `enum-iterator-derive` | `1.5.0` | 0BSD |
| `enum-ordinalize` | `4.4.2` | MIT |
| `enum-ordinalize-derive` | `4.4.2` | MIT |
| `equivalent` | `1.0.2` | Apache-2.0 OR MIT |
| `ethnum` | `1.5.3` | MIT OR Apache-2.0 |
| `feature-probe` | `0.1.1` | MIT/Apache-2.0 |
| `ff` | `0.13.1` | MIT/Apache-2.0 |
| `fiat-crypto` | `0.2.9` | MIT OR Apache-2.0 OR BSD-1-Clause |
| `find-msvc-tools` | `0.1.11` | MIT OR Apache-2.0 |
| `five8` | `1.0.0` | MIT |
| `five8_const` | `1.0.0` | MIT |
| `five8_core` | `1.0.0` | MIT |
| `fnv` | `1.0.7` | Apache-2.0 / MIT |
| `foreign-types` | `0.3.2` | MIT/Apache-2.0 |
| `foreign-types-shared` | `0.1.1` | MIT/Apache-2.0 |
| `funty` | `2.0.0` | MIT |
| `futures-core` | `0.3.32` | MIT OR Apache-2.0 |
| `futures-task` | `0.3.32` | MIT OR Apache-2.0 |
| `futures-util` | `0.3.32` | MIT OR Apache-2.0 |
| `generic-array` | `0.14.9` | MIT |
| `getrandom` | `0.1.16` | MIT OR Apache-2.0 |
| `getrandom` | `0.2.17` | MIT OR Apache-2.0 |
| `getrandom` | `0.3.4` | MIT OR Apache-2.0 |
| `glob` | `0.3.4` | MIT OR Apache-2.0 |
| `group` | `0.13.0` | MIT/Apache-2.0 |
| `hash32` | `0.3.1` | MIT OR Apache-2.0 |
| `hashbrown` | `0.13.2` | MIT OR Apache-2.0 |
| `hashbrown` | `0.15.5` | MIT OR Apache-2.0 |
| `hashbrown` | `0.17.1` | MIT OR Apache-2.0 |
| `heck` | `0.3.3` | MIT OR Apache-2.0 |
| `hermit-abi` | `0.5.2` | MIT OR Apache-2.0 |
| `hmac` | `0.12.1` | MIT OR Apache-2.0 |
| `hmac` | `0.8.1` | MIT OR Apache-2.0 |
| `hmac-drbg` | `0.3.0` | Apache-2.0 |
| `hybrid-array` | `0.4.12` | MIT OR Apache-2.0 |
| `ident_case` | `1.0.1` | MIT/Apache-2.0 |
| `indexmap` | `2.14.0` | Apache-2.0 OR MIT |
| `inout` | `0.1.4` | MIT OR Apache-2.0 |
| `itertools` | `0.10.5` | MIT/Apache-2.0 |
| `itertools` | `0.12.1` | MIT OR Apache-2.0 |
| `itertools` | `0.13.0` | MIT OR Apache-2.0 |
| `itertools` | `0.14.0` | MIT OR Apache-2.0 |
| `itertools` | `0.15.0` | MIT OR Apache-2.0 |
| `itoa` | `1.0.18` | MIT OR Apache-2.0 |
| `js-sys` | `0.3.102` | MIT OR Apache-2.0 |
| `k256` | `0.13.4` | Apache-2.0 OR MIT |
| `keccak` | `0.1.6` | Apache-2.0 OR MIT |
| `keccak-const` | `0.2.0` | MIT OR Apache-2.0 |
| `lazy_static` | `1.5.0` | MIT OR Apache-2.0 |
| `libc` | `0.2.186` | MIT OR Apache-2.0 |
| `libsecp256k1` | `0.7.2` | Apache-2.0 |
| `libsecp256k1-core` | `0.3.0` | Apache-2.0 |
| `libsecp256k1-gen-ecmult` | `0.3.0` | Apache-2.0 |
| `libsecp256k1-gen-genmult` | `0.3.0` | Apache-2.0 |
| `light-poseidon` | `0.2.0` | Apache-2.0 |
| `light-poseidon` | `0.4.0` | Apache-2.0 |
| `litesvm` | `0.16.0` | Apache-2.0 |
| `lock_api` | `0.4.14` | MIT OR Apache-2.0 |
| `log` | `0.4.32` | MIT OR Apache-2.0 |
| `memchr` | `2.8.2` | Unlicense OR MIT |
| `merlin` | `3.0.0` | MIT |
| `molpha-verifier` | `0.3.0-dev.15` | MIT |
| `num` | `0.2.1` | MIT/Apache-2.0 |
| `num-bigint` | `0.2.6` | MIT/Apache-2.0 |
| `num-bigint` | `0.4.8` | MIT OR Apache-2.0 |
| `num-complex` | `0.2.4` | MIT/Apache-2.0 |
| `num-derive` | `0.4.2` | MIT OR Apache-2.0 |
| `num-integer` | `0.1.47` | MIT OR Apache-2.0 |
| `num-iter` | `0.1.46` | MIT OR Apache-2.0 |
| `num-rational` | `0.2.4` | MIT/Apache-2.0 |
| `num-traits` | `0.2.19` | MIT OR Apache-2.0 |
| `num_cpus` | `1.17.0` | MIT OR Apache-2.0 |
| `num_enum` | `0.7.6` | BSD-3-Clause OR MIT OR Apache-2.0 |
| `num_enum_derive` | `0.7.6` | BSD-3-Clause OR MIT OR Apache-2.0 |
| `once_cell` | `1.21.4` | MIT OR Apache-2.0 |
| `opaque-debug` | `0.3.1` | MIT OR Apache-2.0 |
| `openssl` | `0.10.81` | Apache-2.0 |
| `openssl-macros` | `0.1.1` | MIT/Apache-2.0 |
| `openssl-src` | `300.6.1+3.6.3` | MIT/Apache-2.0 |
| `openssl-sys` | `0.9.117` | MIT |
| `pairing` | `0.23.0` | MIT/Apache-2.0 |
| `parking_lot` | `0.12.5` | MIT OR Apache-2.0 |
| `parking_lot_core` | `0.9.12` | MIT OR Apache-2.0 |
| `paste` | `1.0.15` | MIT OR Apache-2.0 |
| `pastey` | `0.2.3` | MIT OR Apache-2.0 |
| `pbkdf2` | `0.11.0` | MIT OR Apache-2.0 |
| `percent-encoding` | `2.3.2` | MIT OR Apache-2.0 |
| `percentage` | `0.1.0` | MIT OR Apache-2.0 |
| `pin-project-lite` | `0.2.17` | Apache-2.0 OR MIT |
| `pkcs8` | `0.10.2` | Apache-2.0 OR MIT |
| `pkg-config` | `0.3.34` | MIT OR Apache-2.0 |
| `polyval` | `0.6.2` | Apache-2.0 OR MIT |
| `ppv-lite86` | `0.2.21` | MIT OR Apache-2.0 |
| `proc-macro-crate` | `3.5.0` | MIT OR Apache-2.0 |
| `proc-macro2` | `1.0.106` | MIT OR Apache-2.0 |
| `qstring` | `0.7.2` | MIT |
| `qualifier_attr` | `0.2.2` | MIT OR Apache-2.0 |
| `quote` | `1.0.45` | MIT OR Apache-2.0 |
| `r-efi` | `5.3.0` | MIT OR Apache-2.0 OR LGPL-2.1-or-later |
| `radium` | `0.7.0` | MIT |
| `rand` | `0.7.3` | MIT OR Apache-2.0 |
| `rand` | `0.8.6` | MIT OR Apache-2.0 |
| `rand` | `0.9.5` | MIT OR Apache-2.0 |
| `rand_chacha` | `0.2.2` | MIT OR Apache-2.0 |
| `rand_chacha` | `0.3.1` | MIT OR Apache-2.0 |
| `rand_chacha` | `0.9.0` | MIT OR Apache-2.0 |
| `rand_core` | `0.5.1` | MIT OR Apache-2.0 |
| `rand_core` | `0.6.4` | MIT OR Apache-2.0 |
| `rand_core` | `0.9.5` | MIT OR Apache-2.0 |
| `rand_hc` | `0.2.0` | MIT/Apache-2.0 |
| `rand_xorshift` | `0.3.0` | MIT OR Apache-2.0 |
| `redox_syscall` | `0.5.18` | MIT |
| `rfc6979` | `0.4.0` | Apache-2.0 OR MIT |
| `rustc-demangle` | `0.1.28` | MIT/Apache-2.0 |
| `rustc_version` | `0.4.1` | MIT OR Apache-2.0 |
| `rustversion` | `1.0.22` | MIT OR Apache-2.0 |
| `scopeguard` | `1.2.0` | MIT OR Apache-2.0 |
| `sec1` | `0.7.3` | Apache-2.0 OR MIT |
| `semver` | `1.0.28` | MIT OR Apache-2.0 |
| `serde` | `1.0.229` | MIT OR Apache-2.0 |
| `serde-big-array` | `0.5.1` | MIT OR Apache-2.0 |
| `serde_bytes` | `0.11.19` | MIT OR Apache-2.0 |
| `serde_core` | `1.0.229` | MIT OR Apache-2.0 |
| `serde_derive` | `1.0.229` | MIT OR Apache-2.0 |
| `serde_json` | `1.0.150` | MIT OR Apache-2.0 |
| `serde_with` | `3.22.0` | MIT OR Apache-2.0 |
| `serde_with_macros` | `3.22.0` | MIT OR Apache-2.0 |
| `sha2` | `0.10.9` | MIT OR Apache-2.0 |
| `sha2` | `0.11.0` | MIT OR Apache-2.0 |
| `sha2` | `0.9.9` | MIT OR Apache-2.0 |
| `sha2-const-stable` | `0.1.0` | MIT OR Apache-2.0 |
| `sha3` | `0.10.9` | MIT OR Apache-2.0 |
| `shlex` | `2.0.1` | MIT OR Apache-2.0 |
| `signature` | `1.6.4` | Apache-2.0 OR MIT |
| `signature` | `2.2.0` | Apache-2.0 OR MIT |
| `slab` | `0.4.12` | MIT |
| `smallvec` | `1.15.2` | MIT OR Apache-2.0 |
| `solana-account` | `4.3.2` | Apache-2.0 |
| `solana-account-info` | `3.1.1` | Apache-2.0 |
| `solana-address` | `1.1.0` | Apache-2.0 |
| `solana-address` | `2.6.1` | Apache-2.0 |
| `solana-address-lookup-table-interface` | `3.1.0` | Apache-2.0 |
| `solana-atomic-u64` | `3.0.1` | Apache-2.0 |
| `solana-bincode` | `3.1.0` | Apache-2.0 |
| `solana-blake3-hasher` | `3.1.0` | Apache-2.0 |
| `solana-bls-signatures` | `3.3.0` | Apache-2.0 |
| `solana-bls12-381-syscall` | `0.1.1` | Apache-2.0 |
| `solana-bn254` | `3.2.1` | Apache-2.0 |
| `solana-borsh` | `3.0.2` | Apache-2.0 |
| `solana-bpf-loader-program` | `4.2.1` | Apache-2.0 |
| `solana-builtins` | `4.2.1` | Apache-2.0 |
| `solana-builtins-default-costs` | `4.2.1` | Apache-2.0 |
| `solana-clock` | `3.1.1` | Apache-2.0 |
| `solana-compute-budget` | `4.2.1` | Apache-2.0 |
| `solana-compute-budget-instruction` | `4.2.1` | Apache-2.0 |
| `solana-compute-budget-interface` | `3.0.0` | Apache-2.0 |
| `solana-compute-budget-program` | `4.2.1` | Apache-2.0 |
| `solana-cpi` | `3.1.0` | Apache-2.0 |
| `solana-curve25519` | `3.1.14` | Apache-2.0 |
| `solana-curve25519` | `4.0.1` | Apache-2.0 |
| `solana-define-syscall` | `3.0.0` | Apache-2.0 |
| `solana-define-syscall` | `4.0.1` | Apache-2.0 |
| `solana-define-syscall` | `5.1.0` | Apache-2.0 |
| `solana-derivation-path` | `3.0.0` | Apache-2.0 |
| `solana-ed25519-program` | `3.0.0` | Apache-2.0 |
| `solana-epoch-rewards` | `3.1.0` | Apache-2.0 |
| `solana-epoch-schedule` | `3.2.0` | Apache-2.0 |
| `solana-feature-gate-interface` | `3.1.0` | Apache-2.0 |
| `solana-feature-gate-interface` | `4.0.0` | Apache-2.0 |
| `solana-fee` | `4.2.1` | Apache-2.0 |
| `solana-fee-calculator` | `3.2.2` | Apache-2.0 |
| `solana-fee-structure` | `3.0.0` | Apache-2.0 |
| `solana-get-sysvar` | `1.0.0` | Apache-2.0 |
| `solana-hash` | `4.5.0` | Apache-2.0 |
| `solana-hash-512` | `1.1.0` | Apache-2.0 |
| `solana-instruction` | `3.4.0` | Apache-2.0 |
| `solana-instruction-error` | `2.4.0` | Apache-2.0 |
| `solana-instructions-sysvar` | `3.0.1` | Apache-2.0 |
| `solana-instructions-sysvar` | `4.0.0` | Apache-2.0 |
| `solana-invoke` | `0.5.0` | MIT OR Apache-2.0 |
| `solana-keccak-hasher` | `3.1.0` | Apache-2.0 |
| `solana-keypair` | `3.1.2` | Apache-2.0 |
| `solana-last-restart-slot` | `3.1.0` | Apache-2.0 |
| `solana-loader-v3-interface` | `6.1.1` | Apache-2.0 |
| `solana-loader-v3-interface` | `7.0.0` | Apache-2.0 |
| `solana-loader-v3-interface` | `8.0.1` | Apache-2.0 |
| `solana-loader-v4-interface` | `3.1.0` | Apache-2.0 |
| `solana-message` | `4.4.1` | Apache-2.0 |
| `solana-msg` | `3.1.0` | Apache-2.0 |
| `solana-native-token` | `3.0.0` | Apache-2.0 |
| `solana-nonce` | `3.2.1` | Apache-2.0 |
| `solana-nonce-account` | `4.1.1` | Apache-2.0 |
| `solana-nullable` | `1.2.0` | Apache-2.0 |
| `solana-packet` | `4.2.0` | Apache-2.0 |
| `solana-poseidon` | `4.0.0` | Apache-2.0 |
| `solana-precompile-error` | `3.0.0` | Apache-2.0 |
| `solana-program-entrypoint` | `3.1.1` | Apache-2.0 |
| `solana-program-error` | `3.0.1` | Apache-2.0 |
| `solana-program-memory` | `3.1.0` | Apache-2.0 |
| `solana-program-option` | `3.1.0` | Apache-2.0 |
| `solana-program-pack` | `3.1.0` | Apache-2.0 |
| `solana-program-runtime` | `4.2.1` | Apache-2.0 |
| `solana-pubkey` | `3.0.0` | Apache-2.0 |
| `solana-pubkey` | `4.2.0` | Apache-2.0 |
| `solana-rent` | `3.1.0` | Apache-2.0 |
| `solana-rent` | `4.3.0` | Apache-2.0 |
| `solana-runtime-transaction` | `4.2.1` | Apache-2.0 |
| `solana-sanitize` | `3.0.1` | Apache-2.0 |
| `solana-sbpf` | `0.21.1` | Apache-2.0 |
| `solana-sdk-ids` | `3.1.0` | Apache-2.0 |
| `solana-sdk-macro` | `3.0.1` | Apache-2.0 |
| `solana-secp256k1-program` | `3.0.1` | Apache-2.0 |
| `solana-secp256k1-recover` | `3.3.0` | Apache-2.0 |
| `solana-secp256r1-program` | `3.0.0` | Apache-2.0 |
| `solana-seed-derivable` | `3.0.0` | Apache-2.0 |
| `solana-seed-phrase` | `3.0.0` | Apache-2.0 |
| `solana-serde-varint` | `3.0.1` | Apache-2.0 |
| `solana-serialize-utils` | `3.1.2` | Apache-2.0 |
| `solana-sha256-hasher` | `3.1.0` | Apache-2.0 |
| `solana-sha512-hasher` | `1.0.1` | Apache-2.0 |
| `solana-short-vec` | `3.2.2` | Apache-2.0 |
| `solana-signature` | `3.4.1` | Apache-2.0 |
| `solana-signer` | `3.0.1` | Apache-2.0 |
| `solana-slot-hashes` | `3.1.0` | Apache-2.0 |
| `solana-slot-history` | `3.1.0` | Apache-2.0 |
| `solana-stable-layout` | `3.0.1` | Apache-2.0 |
| `solana-stake-history` | `1.0.0` | Apache-2.0 |
| `solana-stake-interface` | `2.0.2` | Apache-2.0 |
| `solana-stake-interface` | `4.3.0` | Apache-2.0 |
| `solana-svm-callback` | `4.2.1` | Apache-2.0 |
| `solana-svm-feature-set` | `4.2.1` | Apache-2.0 |
| `solana-svm-log-collector` | `4.2.1` | Apache-2.0 |
| `solana-svm-measure` | `4.2.1` | Apache-2.0 |
| `solana-svm-timings` | `4.2.1` | Apache-2.0 |
| `solana-svm-transaction` | `4.2.0` | Apache-2.0 |
| `solana-svm-type-overrides` | `4.2.1` | Apache-2.0 |
| `solana-syscalls` | `4.2.1` | Apache-2.0 |
| `solana-system-interface` | `2.0.0` | Apache-2.0 |
| `solana-system-interface` | `3.2.0` | Apache-2.0 |
| `solana-system-program` | `4.2.1` | Apache-2.0 |
| `solana-sysvar` | `3.1.1` | Apache-2.0 |
| `solana-sysvar` | `4.1.0` | Apache-2.0 |
| `solana-sysvar-id` | `3.1.0` | Apache-2.0 |
| `solana-transaction` | `4.1.6` | Apache-2.0 |
| `solana-transaction-context` | `4.2.1` | Apache-2.0 |
| `solana-transaction-error` | `3.3.1` | Apache-2.0 |
| `solana-vote-interface` | `6.0.3` | Apache-2.0 |
| `solana-vote-program` | `4.2.1` | Apache-2.0 |
| `solana-zero-copy` | `1.1.1` | Apache-2.0 |
| `solana-zk-elgamal-proof-program` | `4.2.1` | Apache-2.0 |
| `solana-zk-sdk` | `4.0.0` | Apache-2.0 |
| `solana-zk-sdk` | `5.0.1` | Apache-2.0 |
| `solana-zk-token-proof-program` | `4.2.1` | Apache-2.0 |
| `spki` | `0.7.3` | Apache-2.0 OR MIT |
| `spl-associated-token-account-interface` | `2.0.0` | Apache-2.0 |
| `spl-discriminator` | `0.5.2` | Apache-2.0 |
| `spl-discriminator-derive` | `0.2.0` | Apache-2.0 |
| `spl-discriminator-syn` | `0.2.1` | Apache-2.0 |
| `spl-pod` | `0.7.3` | Apache-2.0 |
| `spl-token-2022-interface` | `2.1.0` | Apache-2.0 |
| `spl-token-confidential-transfer-proof-extraction` | `0.5.1` | Apache-2.0 |
| `spl-token-confidential-transfer-proof-generation` | `0.5.1` | Apache-2.0 |
| `spl-token-group-interface` | `0.7.2` | Apache-2.0 |
| `spl-token-interface` | `2.0.0` | Apache-2.0 |
| `spl-token-metadata-interface` | `0.8.0` | Apache-2.0 |
| `spl-type-length-value` | `0.9.1` | Apache-2.0 |
| `strsim` | `0.11.1` | MIT |
| `subtle` | `2.6.1` | BSD-3-Clause |
| `syn` | `1.0.109` | MIT OR Apache-2.0 |
| `syn` | `2.0.118` | MIT OR Apache-2.0 |
| `syn` | `3.0.4` | MIT OR Apache-2.0 |
| `tap` | `1.0.1` | MIT |
| `thiserror` | `1.0.69` | MIT OR Apache-2.0 |
| `thiserror` | `2.0.20` | MIT OR Apache-2.0 |
| `thiserror-impl` | `1.0.69` | MIT OR Apache-2.0 |
| `thiserror-impl` | `2.0.20` | MIT OR Apache-2.0 |
| `threadpool` | `1.8.1` | MIT/Apache-2.0 |
| `tinyvec` | `1.11.0` | Zlib OR Apache-2.0 OR MIT |
| `tinyvec_macros` | `0.1.1` | MIT OR Apache-2.0 OR Zlib |
| `toml_datetime` | `1.1.1+spec-1.1.0` | MIT OR Apache-2.0 |
| `toml_edit` | `0.25.12+spec-1.1.0` | MIT OR Apache-2.0 |
| `toml_parser` | `1.1.2+spec-1.1.0` | MIT OR Apache-2.0 |
| `typenum` | `1.20.1` | MIT OR Apache-2.0 |
| `unicode-ident` | `1.0.24` | (MIT OR Apache-2.0) AND Unicode-3.0 |
| `unicode-segmentation` | `1.13.3` | MIT OR Apache-2.0 |
| `universal-hash` | `0.5.1` | MIT OR Apache-2.0 |
| `unreachable` | `1.0.0` | MIT / Apache-2.0 |
| `uriparse` | `0.6.4` | MIT |
| `vcpkg` | `0.2.15` | MIT/Apache-2.0 |
| `version_check` | `0.9.5` | MIT/Apache-2.0 |
| `void` | `1.0.2` | MIT |
| `wasi` | `0.11.1+wasi-snapshot-preview1` | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT |
| `wasi` | `0.9.0+wasi-snapshot-preview1` | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT |
| `wasip2` | `1.0.4+wasi-0.2.12` | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT |
| `wasm-bindgen` | `0.2.125` | MIT OR Apache-2.0 |
| `wasm-bindgen-macro` | `0.2.125` | MIT OR Apache-2.0 |
| `wasm-bindgen-macro-support` | `0.2.125` | MIT OR Apache-2.0 |
| `wasm-bindgen-shared` | `0.2.125` | MIT OR Apache-2.0 |
| `winapi` | `0.3.9` | MIT/Apache-2.0 |
| `winapi-i686-pc-windows-gnu` | `0.4.0` | MIT/Apache-2.0 |
| `winapi-x86_64-pc-windows-gnu` | `0.4.0` | MIT/Apache-2.0 |
| `wincode` | `0.5.5` | Apache-2.0 |
| `wincode-derive` | `0.4.6` | Apache-2.0 |
| `windows-link` | `0.2.1` | MIT OR Apache-2.0 |
| `winnow` | `1.0.3` | MIT |
| `wit-bindgen` | `0.57.1` | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT |
| `wyz` | `0.5.1` | MIT |
| `zerocopy` | `0.8.52` | BSD-2-Clause OR Apache-2.0 OR MIT |
| `zerocopy-derive` | `0.8.52` | BSD-2-Clause OR Apache-2.0 OR MIT |
| `zeroize` | `1.8.2` | Apache-2.0 OR MIT |
| `zeroize_derive` | `1.5.0` | Apache-2.0 OR MIT |
| `zmij` | `1.0.21` | MIT |

## Installed JavaScript dependency inventory

This inventory includes development dependencies and locally installed transitive packages. A clean lockfile installation must be checked again for binary/package releases.

| Package | Version | Declared license |
| --- | --- | --- |
| `@babel/runtime` | `7.28.2` | MIT |
| `@coral-xyz/anchor` | `0.32.1` | (MIT OR Apache-2.0) |
| `@coral-xyz/anchor-errors` | `0.31.1` | Apache-2.0 |
| `@coral-xyz/borsh` | `0.31.1` | Apache-2.0 |
| `@cspotcode/source-map-support` | `0.8.1` | MIT |
| `@jridgewell/resolve-uri` | `3.1.2` | MIT |
| `@jridgewell/sourcemap-codec` | `1.5.5` | MIT |
| `@jridgewell/trace-mapping` | `0.3.9` | MIT |
| `@noble/curves` | `1.9.4` | MIT |
| `@noble/hashes` | `1.8.0` | MIT |
| `@noble/hashes` | `2.4.0` | MIT |
| `@noble/secp256k1` | `2.3.0` | MIT |
| `@solana-program/compute-budget` | `0.18.1` | Apache-2.0 |
| `@solana-program/program-metadata` | `0.9.3` | MIT |
| `@solana-program/system` | `0.14.1` | Apache-2.0 |
| `@solana/accounts` | `8.2.0` | MIT |
| `@solana/addresses` | `8.2.0` | MIT |
| `@solana/assertions` | `8.2.0` | MIT |
| `@solana/buffer-layout` | `4.0.1` | MIT |
| `@solana/buffer-layout-utils` | `0.3.0` | Apache-2.0 |
| `@solana/codecs` | `2.0.0-rc.1` | MIT |
| `@solana/codecs` | `8.2.0` | MIT |
| `@solana/codecs-core` | `2.0.0-rc.1` | MIT |
| `@solana/codecs-core` | `2.3.0` | MIT |
| `@solana/codecs-core` | `8.2.0` | MIT |
| `@solana/codecs-data-structures` | `2.0.0-rc.1` | MIT |
| `@solana/codecs-data-structures` | `8.2.0` | MIT |
| `@solana/codecs-numbers` | `2.0.0-rc.1` | MIT |
| `@solana/codecs-numbers` | `2.3.0` | MIT |
| `@solana/codecs-numbers` | `8.2.0` | MIT |
| `@solana/codecs-strings` | `2.0.0-rc.1` | MIT |
| `@solana/codecs-strings` | `8.2.0` | MIT |
| `@solana/errors` | `2.0.0-rc.1` | MIT |
| `@solana/errors` | `2.3.0` | MIT |
| `@solana/errors` | `8.2.0` | MIT |
| `@solana/fast-stable-stringify` | `8.2.0` | MIT |
| `@solana/fixed-points` | `8.2.0` | MIT |
| `@solana/functional` | `8.2.0` | MIT |
| `@solana/instruction-plans` | `8.2.0` | MIT |
| `@solana/instructions` | `8.2.0` | MIT |
| `@solana/keys` | `8.2.0` | MIT |
| `@solana/kit` | `8.2.0` | MIT |
| `@solana/nominal-types` | `8.2.0` | MIT |
| `@solana/offchain-messages` | `8.2.0` | MIT |
| `@solana/options` | `2.0.0-rc.1` | MIT |
| `@solana/options` | `8.2.0` | MIT |
| `@solana/plugin-core` | `8.2.0` | MIT |
| `@solana/plugin-interfaces` | `8.2.0` | MIT |
| `@solana/program-client-core` | `8.2.0` | MIT |
| `@solana/programs` | `8.2.0` | MIT |
| `@solana/promises` | `8.2.0` | MIT |
| `@solana/rpc` | `8.2.0` | MIT |
| `@solana/rpc-api` | `8.2.0` | MIT |
| `@solana/rpc-parsed-types` | `8.2.0` | MIT |
| `@solana/rpc-spec` | `8.2.0` | MIT |
| `@solana/rpc-spec-types` | `8.2.0` | MIT |
| `@solana/rpc-subscriptions` | `8.2.0` | MIT |
| `@solana/rpc-subscriptions-api` | `8.2.0` | MIT |
| `@solana/rpc-subscriptions-channel-websocket` | `8.2.0` | MIT |
| `@solana/rpc-subscriptions-spec` | `8.2.0` | MIT |
| `@solana/rpc-transformers` | `8.2.0` | MIT |
| `@solana/rpc-transport-http` | `8.2.0` | MIT |
| `@solana/rpc-types` | `8.2.0` | MIT |
| `@solana/signers` | `8.2.0` | MIT |
| `@solana/spl-token` | `0.4.15` | Apache-2.0 |
| `@solana/spl-token-group` | `0.0.7` | Apache-2.0 |
| `@solana/spl-token-metadata` | `0.1.6` | Apache-2.0 |
| `@solana/subscribable` | `8.2.0` | MIT |
| `@solana/sysvars` | `8.2.0` | MIT |
| `@solana/transaction-confirmation` | `8.2.0` | MIT |
| `@solana/transaction-introspection` | `8.2.0` | MIT |
| `@solana/transaction-messages` | `8.2.0` | MIT |
| `@solana/transactions` | `8.2.0` | MIT |
| `@solana/web3.js` | `1.98.4` | MIT |
| `@swc/helpers` | `0.5.17` | Apache-2.0 |
| `@tsconfig/node10` | `1.0.12` | MIT |
| `@tsconfig/node12` | `1.0.11` | MIT |
| `@tsconfig/node14` | `1.0.3` | MIT |
| `@tsconfig/node16` | `1.0.4` | MIT |
| `@types/bn.js` | `5.2.0` | MIT |
| `@types/connect` | `3.4.38` | MIT |
| `@types/node` | `12.20.55` | MIT |
| `@types/node` | `20.19.9` | MIT |
| `@types/node` | `22.20.1` | MIT |
| `@types/uuid` | `8.3.4` | MIT |
| `@types/ws` | `7.4.7` | MIT |
| `@types/ws` | `8.18.1` | MIT |
| `@typescript/typescript-darwin-arm64` | `7.0.2` | Apache-2.0 |
| `acorn` | `8.16.0` | MIT |
| `acorn-walk` | `8.3.5` | MIT |
| `agentkeepalive` | `4.6.0` | MIT |
| `arg` | `4.1.3` | MIT |
| `base-x` | `3.0.11` | MIT |
| `base64-js` | `1.5.1` | MIT |
| `bigint-buffer` | `1.1.5` | Apache-2.0 |
| `bignumber.js` | `9.3.1` | MIT |
| `bindings` | `1.5.0` | MIT |
| `bn.js` | `5.2.4` | MIT |
| `borsh` | `0.7.0` | Apache-2.0 |
| `bs58` | `4.0.1` | MIT |
| `buffer` | `6.0.3` | MIT |
| `buffer-layout` | `1.2.2` | MIT |
| `bufferutil` | `4.0.9` | MIT |
| `camelcase` | `6.3.0` | MIT |
| `chalk` | `5.4.1` | MIT |
| `chalk` | `5.6.2` | MIT |
| `commander` | `12.1.0` | MIT |
| `commander` | `13.1.0` | MIT |
| `commander` | `14.0.1` | MIT |
| `commander` | `15.0.0` | MIT |
| `commander` | `2.20.3` | MIT |
| `create-require` | `1.1.1` | MIT |
| `cross-fetch` | `3.2.0` | MIT |
| `cross-fetch-polyfill` | `0.0.0` | MIT |
| `delay` | `5.0.0` | MIT |
| `diff` | `4.0.4` | BSD-3-Clause |
| `es6-promise` | `4.2.8` | MIT |
| `es6-promisify` | `5.0.0` | MIT |
| `eventemitter3` | `4.0.7` | MIT |
| `eventemitter3` | `5.0.1` | MIT |
| `eyes` | `0.1.8` | MIT |
| `fast-stable-stringify` | `1.0.0` | MIT |
| `file-uri-to-path` | `1.0.0` | MIT |
| `humanize-ms` | `1.2.1` | MIT |
| `ieee754` | `1.2.1` | BSD-3-Clause |
| `isomorphic-ws` | `4.0.1` | MIT |
| `jayson` | `4.2.0` | MIT |
| `json-stringify-safe` | `5.0.1` | ISC |
| `json5` | `2.2.3` | MIT |
| `make-error` | `1.3.6` | ISC |
| `minimist` | `1.2.8` | MIT |
| `ms` | `2.1.3` | MIT |
| `node-fetch` | `2.7.0` | MIT |
| `node-gyp-build` | `4.8.4` | MIT |
| `pako` | `2.1.0` | (MIT AND Zlib) |
| `pako` | `2.2.0` | (MIT AND Zlib) |
| `picocolors` | `1.1.1` | ISC |
| `prettier` | `3.9.6` | MIT |
| `rpc-websockets` | `9.1.2` | LGPL-3.0-only |
| `safe-buffer` | `5.2.1` | MIT |
| `smol-toml` | `1.8.0` | BSD-3-Clause |
| `stream-chain` | `2.2.5` | BSD-3-Clause |
| `stream-json` | `1.9.1` | BSD-3-Clause |
| `strip-bom` | `3.0.0` | MIT |
| `superstruct` | `0.15.5` | MIT |
| `superstruct` | `2.0.2` | MIT |
| `text-encoding-utf-8` | `1.0.2` | Unlicense (installed LICENSE.md; Encoding Standard attribution noted above) |
| `toml` | `3.0.0` | MIT |
| `tr46` | `0.0.3` | MIT |
| `ts-node` | `10.9.2` | MIT |
| `tsconfig-paths` | `4.2.0` | MIT |
| `tslib` | `2.8.1` | 0BSD |
| `typescript` | `7.0.2` | Apache-2.0 |
| `undici-types` | `6.21.0` | MIT |
| `undici-types` | `8.10.2` | MIT |
| `utf-8-validate` | `5.0.10` | MIT |
| `uuid` | `8.3.2` | MIT |
| `v8-compile-cache-lib` | `3.0.1` | MIT |
| `vscode-jsonrpc` | `9.0.0` | MIT |
| `webidl-conversions` | `3.0.1` | BSD-2-Clause |
| `whatwg-url` | `5.0.0` | MIT |
| `ws` | `7.5.10` | MIT |
| `ws` | `8.18.3` | MIT |
| `ws` | `8.21.3` | MIT |
| `yaml` | `2.9.0` | ISC |
| `yn` | `3.1.1` | MIT |
