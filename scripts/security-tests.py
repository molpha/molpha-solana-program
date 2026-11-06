#!/usr/bin/env python3
"""Offline execution after dependency/tool setup; never connects to a Solana cluster.

Reports, mutation copies, fuzz artifacts, and failing inputs stay under ignored target/.
Native coverage is deliberately not presented as SBF source coverage.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import time
import tomllib

ROOT = Path(__file__).resolve().parents[1]
PRIVATE = ROOT / "target/security-review"
MATRIX = ROOT / "molpha-tests/security-matrix.json"
FUZZ_TARGETS = ("registry_bytes", "wire_bytes", "epoch_arithmetic")
CATEGORIES = {
    "authorization",
    "account_constraints",
    "validation",
    "state_transitions",
    "custody",
    "rollback",
    "replay",
    "arithmetic",
    "limits",
}
DISPOSITIONS = {"covered", "permitted", "unreachable", "finding", "missing"}

# A mutation counts only when a previously passing, exact test fails at runtime.
MUTATIONS = (
    ("authority", "programs/molpha/src/instructions/admin/update_config.rs",
     "        has_one = authority,\n", "",
     "protocol_config::update_config_rejects_a_caller_that_is_not_the_authority"),
    ("feed_order", "programs/molpha/src/state/feed.rs",
     "attestation.payload.timestamp > self.timestamp", "attestation.payload.timestamp >= self.timestamp",
     "attestation::submit_attestation_refuses_to_move_the_feed_backwards"),
    ("ticket_selection", "programs/molpha/src/instructions/epoch/submit_ticket.rs",
     "        signers.is_subset(&selection),", "        true,",
     "security_regressions::ticket_outside_selection_is_rejected_with_full_rollback"),
    ("duplicate_credit", "programs/molpha/src/instructions/epoch/credit_epoch_rewards.rs",
     "weight > 0 && !state.is_credited(slot)", "weight > 0",
     "security_regressions::duplicate_credit_in_one_batch_restores_rewards_and_the_credit_bitmap"),
    ("reward_custody", "programs/molpha/src/state/protocol_config.rs",
     "        self.reward_liability\n", "        0u64\n",
     "security_regressions::credited_rewards_are_never_recognized_as_new_income"),
    ("evidence_boundary", "programs/molpha/src/instructions/slash_offence.rs",
     "now < grace_active_until.saturating_add(window)", "now <= grace_active_until.saturating_add(window)",
     "security_properties::freshness_and_evidence_have_opposite_deadline_inclusivity"),
    ("reserve_cap", "programs/molpha/src/instructions/revenue/withdraw_protocol_reserve.rs",
     "amount > 0 && amount <= config.protocol_reserved", "amount > 0",
     "security_regressions::reserve_withdrawal_zero_and_one_over_do_not_touch_other_liabilities"),
    ("raw_value_binding", "programs/molpha/src/instructions/submit_attestation.rs",
     "raw_value.len() <= Feed::MAX_VALUE_LEN\n                    && hashv(&[raw_value.as_slice()]).to_bytes() == attestation.payload.value",
     "raw_value.len() <= Feed::MAX_VALUE_LEN\n                    && true",
     "attestation::a_raw_value_must_hash_to_the_signed_value"),
    ("gateway_status", "programs/molpha/src/instructions/epoch/submit_ticket.rs",
     "        constraint = gateway.status == GatewayStatus::Active @ MolphaError::GatewayNotActive,\n", "",
     "withdrawals::a_deactivated_gateway_can_no_longer_admit_tickets"),
    ("guilty_account_binding", "programs/molpha/src/utils/slash.rs",
     "            registry.node(bit_pos)?,\n", "            account_info.key(),\n",
     "security_regressions::a_wrong_late_guilty_account_rolls_back_prior_punishments_and_rent"),
)


def digest_files(root: Path, paths: list[Path]) -> str:
    digest = hashlib.sha256()
    for path in sorted(paths):
        digest.update(path.relative_to(root).as_posix().encode() + b"\0")
        digest.update(path.read_bytes())
        digest.update(b"\0")
    return digest.hexdigest()


def program_digest(root: Path) -> str:
    files = list((root / "programs/molpha/src").rglob("*.rs"))
    files += [root / name for name in (
        "Cargo.toml", "Cargo.lock", "Anchor.toml", "rust-toolchain.toml", "programs/molpha/Cargo.toml",
    )]
    return digest_files(root, files)


def test_inventory(root: Path) -> set[str]:
    tests = set()
    for path in (root / "molpha-tests/tests").glob("*.rs"):
        source = path.read_text()
        for name in re.findall(r"#\[test\]\s*(?:#\[[^\]]+\]\s*)*fn\s+(\w+)", source):
            tests.add(f"{path.stem}::{name}")
        # protocol_config's table macro emits one concrete #[test] per invocation.
        for name in re.findall(r"rejects_config!\(\s*(\w+)\s*,", source):
            tests.add(f"{path.stem}::{name}")
    return tests


def program_sources(root: Path) -> dict[str, str]:
    base = root / "programs/molpha/src"
    return {
        path.relative_to(root).as_posix(): hashlib.sha256(path.read_bytes()).hexdigest()
        for path in sorted(base.rglob("*.rs"))
    }


def public_instructions(root: Path) -> set[str]:
    source = (root / "programs/molpha/src/lib.rs").read_text()
    return set(re.findall(r"^\s{4}pub fn (\w+)", source, re.M))


def production_errors(root: Path) -> set[str]:
    errors = set()
    for path in (root / "programs/molpha/src").rglob("*.rs"):
        errors.update(re.findall(r"MolphaError::(\w+)", path.read_text()))
    return errors


def snake_case(name: str) -> str:
    return re.sub(r"(?<=[a-z0-9])(?=[A-Z])", "_", name).lower()


def account_context_inventory(root: Path) -> dict[tuple[str, str, str], dict]:
    """Inventory every generated Anchor account field and its security-relevant constraints."""
    inventory = {}
    derive = re.compile(
        r"#\[derive\(Accounts\)\]\s*(?:#\[[^\]]*\]\s*)*"
        r"pub struct\s+(\w+)[^{]*\{"
    )
    field = re.compile(r"^\s*pub\s+(\w+)\s*:\s*(.+),\s*$", re.M)
    for path in sorted((root / "programs/molpha/src/instructions").rglob("*.rs")):
        source = path.read_text()
        for match in derive.finditer(source):
            context = match.group(1)
            instruction = snake_case(context)
            depth = 1
            cursor = match.end()
            while cursor < len(source) and depth:
                depth += (source[cursor] == "{") - (source[cursor] == "}")
                cursor += 1
            if depth:
                raise ValueError(f"unterminated Accounts context: {path}: {context}")
            body = source[match.end():cursor - 1]
            previous = 0
            for declared in field.finditer(body):
                account = declared.group(1)
                account_type = declared.group(2)
                attributes = body[previous:declared.start()]
                previous = declared.end()
                account_attributes = "\n".join(
                    re.findall(r"#\[account\((.*?)\)\]", attributes, re.S)
                )
                kinds = set()
                if "Signer<" in account_type:
                    kinds.add("signer")
                if "Account<" in account_type or "AccountLoader<" in account_type:
                    kinds.add("owner")
                if "Program<" in account_type:
                    kinds.add("program_id")
                if re.search(r"Program<'info,\s*Token>", account_type):
                    kinds.add("token_program")
                if "Option<" in account_type:
                    kinds.add("optional_presence")
                if re.search(r"\binit(?:_if_needed)?\b", account_attributes):
                    kinds.add("initialization")
                if "payer =" in account_attributes:
                    kinds.add("payer_binding")
                if "seeds =" in account_attributes:
                    kinds.add("pda")
                if "has_one =" in account_attributes:
                    kinds.add("has_one")
                if "associated_token::mint" in account_attributes:
                    kinds.add("ata_mint")
                if "associated_token::authority" in account_attributes:
                    kinds.add("ata_authority")
                if "associated_token::token_program" in account_attributes:
                    kinds.add("token_program")
                if "token::mint" in account_attributes:
                    kinds.add("token_mint")
                if "constraint =" in account_attributes:
                    kinds.add("status" if ".status" in account_attributes else "custom_constraint")
                if re.search(r"\baddress\s*=(?!=)", account_attributes):
                    kinds.add("address")
                if "close =" in account_attributes or (
                    "rent_payer" in account_attributes
                    and re.search(r"\baddress\s*=(?!=)", account_attributes)
                ):
                    kinds.add("close_receiver")
                key = (instruction, context, account)
                inventory[key] = {
                    "source": path.relative_to(root).as_posix(),
                    "constraint_kinds": sorted(kinds),
                }
            if "ctx.remaining_accounts" in source:
                inventory[(instruction, context, "$remaining_accounts")] = {
                    "source": path.relative_to(root).as_posix(),
                    "constraint_kinds": ["ordered_remaining_accounts"],
                }
    return inventory


def validate_matrix(root: Path = ROOT) -> dict:
    matrix = json.loads((root / MATRIX.relative_to(ROOT)).read_text())
    if matrix.get("schema_version") != 2:
        raise ValueError("security matrix must use schema_version 2")

    sources = program_sources(root)
    reviewed = matrix.get("source_inventory", {})
    if set(reviewed) != set(sources):
        raise ValueError(
            "security matrix source inventory mismatch: "
            f"unreviewed={sorted(set(sources) - set(reviewed))}, "
            f"removed={sorted(set(reviewed) - set(sources))}"
        )
    for path, expected in reviewed.items():
        if not re.fullmatch(r"[0-9a-f]{64}", expected) or sources[path] != expected:
            raise ValueError(f"security matrix has stale source hash: {path}")

    public = public_instructions(root)
    entries = matrix["instructions"]
    names = [entry["instruction"] for entry in entries]
    if len(names) != len(set(names)) or set(names) != public:
        raise ValueError(f"matrix instruction mismatch: missing={sorted(public - set(names))}, extra={sorted(set(names) - public)}")
    tests = test_inventory(root)
    errors = production_errors(root)
    obligation_ids = set()
    all_obligations = []

    def validate_obligation(obligation: dict, owner: str):
        obligation_id = obligation.get("id", "")
        if not re.fullmatch(r"MOLPHA-[A-Z0-9-]+", obligation_id):
            raise ValueError(f"invalid obligation id for {owner}: {obligation_id!r}")
        if obligation_id in obligation_ids:
            raise ValueError(f"duplicate security obligation: {obligation_id}")
        obligation_ids.add(obligation_id)
        if obligation.get("category") not in CATEGORIES:
            raise ValueError(f"unknown obligation category: {obligation_id}")
        relevant = obligation.get("relevant_errors")
        if not isinstance(relevant, list) or not set(relevant) <= errors:
            raise ValueError(f"unknown relevant error for {obligation_id}: {relevant}")
        disposition = obligation.get("disposition")
        if disposition not in DISPOSITIONS:
            raise ValueError(f"unknown matrix disposition: {disposition}")
        rationale = obligation.get("rationale", "").strip()
        if not rationale:
            raise ValueError(f"missing obligation rationale: {obligation_id}")
        references = obligation.get("tests", [])
        if not isinstance(references, list) or len(references) != len(set(references)):
            raise ValueError(f"invalid exact test list: {obligation_id}")
        unknown = set(references) - tests
        if unknown:
            raise ValueError(f"unknown exact tests for {obligation_id}: {sorted(unknown)}")
        if disposition == "covered" and not references:
            raise ValueError(f"covered obligation has no exact test: {obligation_id}")
        if disposition in {"permitted", "unreachable"} and len(rationale) < 20:
            raise ValueError(f"unexplained {disposition} obligation: {obligation_id}")
        all_obligations.append(obligation)

    for entry in entries:
        if not entry.get("authority_policy") or not entry.get("obligations"):
            raise ValueError(f"empty matrix entry: {entry['instruction']}")
        entry_sources = entry.get("reviewed_sources", [])
        if not entry_sources or not set(entry_sources) <= set(sources):
            raise ValueError(f"invalid reviewed sources: {entry['instruction']}")
        for obligation in entry["obligations"]:
            validate_obligation(obligation, entry["instruction"])

    expected_accounts = account_context_inventory(root)
    account_rows = matrix.get("account_constraints", [])
    actual_accounts = {}
    for row in account_rows:
        key = (row.get("instruction"), row.get("context"), row.get("account"))
        if key in actual_accounts:
            raise ValueError(f"duplicate account audit row: {key}")
        actual_accounts[key] = row
        validate_obligation(row, ".".join(str(part) for part in key))
    if set(actual_accounts) != set(expected_accounts):
        raise ValueError(
            "account context audit mismatch: "
            f"missing={sorted(set(expected_accounts) - set(actual_accounts))}, "
            f"extra={sorted(set(actual_accounts) - set(expected_accounts))}"
        )
    entry_by_name = {entry["instruction"]: entry for entry in entries}
    for key, expected in expected_accounts.items():
        row = actual_accounts[key]
        if row.get("source") != expected["source"] or row.get("constraint_kinds") != expected["constraint_kinds"]:
            raise ValueError(f"stale account constraint inventory: {key}")
        if expected["source"] not in entry_by_name[key[0]]["reviewed_sources"]:
            raise ValueError(f"account context source not assigned to instruction: {key}")

    error_rows = matrix.get("error_dispositions", [])
    error_names = [row.get("error") for row in error_rows]
    if len(error_names) != len(set(error_names)) or set(error_names) != errors:
        raise ValueError(
            "MolphaError disposition mismatch: "
            f"missing={sorted(errors - set(error_names))}, extra={sorted(set(error_names) - errors)}"
        )
    for row in error_rows:
        if row.get("relevant_errors") != [row["error"]]:
            raise ValueError(f"error disposition must name itself: {row.get('error')}")
        validate_obligation(row, row["error"])

    unsafe = [
        obligation["id"]
        for obligation in all_obligations
        if obligation["disposition"] in {"missing", "finding"}
    ]
    if unsafe:
        raise ValueError(f"security matrix is not a clean sign-off: {unsafe}")

    return {
        "schema_version": 2,
        "instructions": len(public),
        "program_sources": len(sources),
        "production_errors": len(errors),
        "account_constraints": len(account_rows),
        "obligations": len(all_obligations),
        "referenced_tests": len({t for obligation in all_obligations for t in obligation.get("tests", [])}),
        "checks_by_status": {
            disposition: sum(o["disposition"] == disposition for o in all_obligations)
            for disposition in sorted(DISPOSITIONS)
        },
        "note": "Reviewed behavioral obligations; not measured SBF branch coverage.",
    }


def apply_mutation(source: str, before: str, after: str) -> str:
    if source.count(before) != 1:
        raise ValueError("mutation anchor must match exactly once; review the changed source")
    return source.replace(before, after, 1)


class Campaign:
    def __init__(self, args: argparse.Namespace):
        self.args = args
        self.env = os.environ.copy()
        self.env.pop("MOLPHA_WRITE_FIXTURES", None)
        self.env["MOLPHA_SECURITY_SEED"] = str(args.seed)
        self.env["PROPTEST_RNG_SEED"] = str(args.seed)
        self.env["MOLPHA_SECURITY_PROFILE"] = "extended" if args.command == "extended" else "pr"
        if args.replay:
            self.env["MOLPHA_SECURITY_REPLAY"] = str(Path(args.replay).resolve())
        else:
            self.env.pop("MOLPHA_SECURITY_REPLAY", None)
        self.rust = tomllib.loads((ROOT / "rust-toolchain.toml").read_text())["toolchain"]["channel"]
        self.out = PRIVATE / f"{args.command}-{time.time_ns()}"
        self.out.mkdir(parents=True)
        self.report = {"profile": args.command, "seed": str(args.seed), "steps": [], "status": "running"}
        self.report["inputs"] = {
            "program_sha256": program_digest(ROOT),
            "matrix_sha256": hashlib.sha256(MATRIX.read_bytes()).hexdigest(),
            "boundary_corpus_sha256": hashlib.sha256(
                (ROOT / "molpha-tests/tests/fixtures/security-boundary.json").read_bytes()
            ).hexdigest(),
            "fuzz_lock_sha256": hashlib.sha256((ROOT / "fuzz/Cargo.lock").read_bytes()).hexdigest(),
        }
        self.report["source_sha256"] = self.report["inputs"]["program_sha256"]
        self.report["matrix"] = validate_matrix()

    def run(self, name: str, command: list[str], *, cwd: Path = ROOT,
            env: dict | None = None, allow_failure: bool = False) -> subprocess.CompletedProcess:
        print(f"[{name}] {' '.join(command)}", flush=True)
        log = self.out / f"{len(self.report['steps']):02d}-{name}.log"
        start = time.monotonic()
        with log.open("w") as stream:
            result = subprocess.run(command, cwd=cwd, env=env or self.env, stdout=stream, stderr=subprocess.STDOUT)
        self.report["steps"].append({"name": name, "command": command, "cwd": str(cwd),
                                     "exit_code": result.returncode, "seconds": round(time.monotonic() - start, 3),
                                     "log": str(log)})
        self.write_report()
        if result.returncode and not allow_failure:
            raise RuntimeError(f"{name} failed; private log: {log}")
        return result

    def write_report(self):
        (self.out / "report.json").write_text(json.dumps(self.report, indent=2) + "\n")

    def command_output(self, name: str, command: list[str], hint: str) -> str:
        if shutil.which(command[0], path=self.env.get("PATH")) is None:
            raise RuntimeError(f"missing {name}; {hint}")
        try:
            result = subprocess.run(
                command, env=self.env, cwd=ROOT, capture_output=True, text=True, check=True
            )
        except FileNotFoundError as error:
            raise RuntimeError(f"missing {name}; {hint}") from error
        except subprocess.CalledProcessError as error:
            detail = (error.stderr or error.stdout or str(error)).strip().splitlines()[-1]
            raise RuntimeError(f"{name} is unavailable ({detail}); {hint}") from error
        return (result.stdout or result.stderr).strip()

    def preflight(self):
        anchor = tomllib.loads((ROOT / "Anchor.toml").read_text())["toolchain"]
        versions = {}
        needs_sbf = self.args.command in {"pr", "coverage"} or (
            self.args.command == "extended" and self.args.phase in {"all", "stateful", "mutations"}
        )
        needs_js = self.args.command == "pr"
        checks = [
            (
                "Rust toolchain",
                ["cargo", f"+{self.rust}", "--version"],
                self.rust,
                f"install it with `rustup toolchain install {self.rust}`",
            )
        ]
        if needs_sbf:
            checks.extend([
                (
                    "Anchor CLI",
                    ["anchor", "--version"],
                    anchor["anchor_version"],
                    f"install and select Anchor {anchor['anchor_version']} with AVM",
                ),
                (
                    "Solana CLI",
                    ["solana", "--version"],
                    anchor["solana_version"],
                    f"install Solana {anchor['solana_version']} and its SBF platform tools",
                ),
            ])
            self.command_output(
                "cargo-build-sbf",
                ["cargo-build-sbf", "--version"],
                "run `cargo-build-sbf --force-tools-install --install-only` from the pinned Solana CLI",
            )
        for name, command, expected, hint in checks:
            value = self.command_output(name, command, hint)
            if not re.search(rf"(?<!\d){re.escape(expected)}(?![\d.])", value):
                raise RuntimeError(f"{name} version mismatch: expected {expected}, found {value}")
            versions[name] = value
        if needs_js:
            versions["node"] = self.command_output(
                "Node.js", ["node", "--version"], "install Node.js 22 or newer"
            )
            versions["yarn"] = self.command_output(
                "Yarn", ["yarn", "--version"], "install Yarn 1.22 and the locked dependencies"
            )
            if not (ROOT / "node_modules").is_dir():
                raise RuntimeError("missing locked JS dependencies; run `yarn install --frozen-lockfile`")
        if self.args.command == "coverage":
            versions["cargo-llvm-cov"] = self.command_output(
                "cargo-llvm-cov",
                ["cargo", f"+{self.rust}", "llvm-cov", "--version"],
                "install it with `cargo install cargo-llvm-cov --locked`",
            )
        if self.args.command == "extended" and self.args.phase in {"all", "fuzz"}:
            versions["cargo-fuzz"] = self.command_output(
                "cargo-fuzz and fuzz toolchain",
                ["cargo", f"+{self.args.fuzz_toolchain}", "fuzz", "--version"],
                f"install cargo-fuzz and `rustup toolchain install {self.args.fuzz_toolchain}`",
            )
            self.command_output(
                "pinned fuzz dependency graph",
                ["cargo", f"+{self.rust}", "metadata", "--manifest-path", "fuzz/Cargo.toml", "--locked", "--no-deps"],
                "regenerate fuzz/Cargo.lock with the pinned Anchor dependencies",
            )
        if self.args.command == "extended" and self.args.phase in {"all", "mutations"}:
            versions["git"] = self.command_output(
                "Git", ["git", "--version"], "install Git so mutation worktrees can be inventoried"
            )
        self.report["toolchain"] = versions
        self.report["prerequisites"] = {
            "sbf": needs_sbf,
            "javascript": needs_js,
            "coverage": self.args.command == "coverage",
            "fuzz": self.args.command == "extended" and self.args.phase in {"all", "fuzz"},
            "mutations": self.args.command == "extended" and self.args.phase in {"all", "mutations"},
        }

    def build(self, root: Path = ROOT, name: str = "build", env: dict | None = None):
        before = program_digest(root)
        binary = root / "target/deploy/molpha.so"
        idl = root / "target/idl/molpha.json"
        for artifact in (binary, idl):
            artifact.unlink(missing_ok=True)
        self.run(name, ["anchor", "build", "--ignore-keys", "-p", "molpha"], cwd=root, env=env)
        if before != program_digest(root):
            raise RuntimeError("program inputs changed during build; discard this result")
        if not binary.is_file() or not idl.is_file():
            raise RuntimeError("build did not produce the SBF binary and IDL")
        return {
            "source_sha256": before,
            "executable_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
            "executable_bytes": binary.stat().st_size,
            "idl_sha256": hashlib.sha256(idl.read_bytes()).hexdigest(),
            "idl_bytes": idl.stat().st_size,
            "fresh_outputs": True,
        }

    def tests(self):
        self.run("cli", ["yarn", "test:cli"])
        self.run("rust", ["cargo", f"+{self.rust}", "test", "--workspace", "--locked"])

    def stateful(self):
        self.run(
            "stateful-trace",
            ["cargo", f"+{self.rust}", "test", "-p", "molpha-tests", "--test", "suite", "--locked",
             "security_stateful::deterministic_trace_executes_every_modeled_action", "--", "--exact"],
        )
        self.run(
            "stateful-generated",
            ["cargo", f"+{self.rust}", "test", "-p", "molpha-tests", "--test", "suite", "--locked",
             "security_stateful::generated_protocol_sequences", "--", "--exact"],
        )

    def coverage(self):
        report = self.out / "native-coverage.json"
        self.run("native-coverage", ["cargo", f"+{self.rust}", "llvm-cov", "--workspace", "--locked",
                 "--json", "--output-path", str(report), "--ignore-filename-regex", r"(/molpha-tests/|/\.cargo/)"])
        self.run("native-html", ["cargo", f"+{self.rust}", "llvm-cov", "report", "--html", "--output-dir", str(self.out / "html"),
                 "--ignore-filename-regex", r"(/molpha-tests/|/\.cargo/)"])
        data = json.loads(report.read_text())
        self.report["native_coverage"] = data["data"][0]["totals"]
        self.report["native_coverage_note"] = "Host execution only. SBF handlers loaded in LiteSVM are not instrumented."

    def fuzz(self):
        for target in FUZZ_TARGETS:
            corpus = PRIVATE / "fuzz-corpus" / target
            corpus.mkdir(parents=True, exist_ok=True)
            for seed in (ROOT / "fuzz/seeds" / target).glob("*"):
                shutil.copy2(seed, corpus / seed.name)
            artifacts = PRIVATE / "fuzz-artifacts" / target
            artifacts.mkdir(parents=True, exist_ok=True)
            self.run(f"fuzz-{target}", ["cargo", f"+{self.args.fuzz_toolchain}", "fuzz", "run", target, str(corpus),
                     "--", f"-max_total_time={self.args.fuzz_seconds}", f"-seed={self.args.seed % 2**32}",
                     "-max_len=32768", "-rss_limit_mb=4096", f"-artifact_prefix={artifacts}/"], cwd=ROOT / "fuzz")

    def mutations(self):
        tracked = subprocess.check_output(["git", "ls-files", "--cached", "--others", "--exclude-standard", "-z"], cwd=ROOT)
        files = [Path(os.fsdecode(name)) for name in tracked.split(b"\0") if name]
        control_artifact = self.report.get("artifact")
        if not control_artifact:
            raise RuntimeError("mutation phase requires a fresh control SBF build")
        results = []
        for name, path, before, after, test in MUTATIONS:
            root = self.out / "mutations" / name
            for relative in files:
                original = ROOT / relative
                if original.is_file():
                    destination = root / relative
                    destination.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copy2(original, destination)
            env = dict(self.env, CARGO_TARGET_DIR=str(root / "target"))
            isolated_control = self.build(root, f"build-control-{name}", env)
            self.run(
                f"control-{name}",
                ["cargo", f"+{self.rust}", "test", "-p", "molpha-tests", "--test", "suite", "--locked", test, "--", "--exact"],
                cwd=root,
                env=env,
            )
            source = root / path
            source.write_text(apply_mutation(source.read_text(), before, after))
            # Every mutant gets an isolated Cargo/SBF target. Reusing a deployment directory can
            # make a test load another mutant's binary and produce a false kill.
            artifact = self.build(root, f"build-mutant-{name}", env)
            if artifact["executable_sha256"] == isolated_control["executable_sha256"]:
                raise RuntimeError(f"mutation {name} produced the control binary; mutation was not compiled")
            result = self.run(f"mutant-{name}", ["cargo", f"+{self.rust}", "test", "-p", "molpha-tests", "--test", "suite", "--locked", test, "--", "--exact"],
                              cwd=root, env=env, allow_failure=True)
            log = Path(self.report["steps"][-1]["log"]).read_text()
            exact_failure = re.search(rf"^test {re.escape(test)} \.\.\. FAILED$", log, re.M)
            failed_summary = re.search(
                r"test result: FAILED\. 0 passed; 1 failed; 0 ignored; 0 measured; \d+ filtered out",
                log,
            )
            killed = result.returncode != 0 and exact_failure is not None and failed_summary is not None
            results.append({
                "mutation": name,
                "test": test,
                "killed": killed,
                "campaign_control_executable_sha256": control_artifact["executable_sha256"],
                "isolated_control_executable_sha256": isolated_control["executable_sha256"],
                **artifact,
            })
            self.report["mutations"] = results
            self.write_report()
            if not killed:
                raise RuntimeError(f"mutation {name} survived or failed outside its test; inspect private logs")

    def execute(self):
        try:
            self.preflight()
            needs_sbf = self.args.command in {"pr", "coverage"} or (
                self.args.command == "extended"
                and self.args.phase in {"all", "stateful", "mutations"}
            )
            if needs_sbf:
                self.report["artifact"] = self.build()
            if self.args.command == "pr":
                self.tests()
            elif self.args.command == "coverage":
                self.coverage()
            else:
                if self.args.phase in {"all", "stateful"}:
                    self.stateful()
                if self.args.phase in {"all", "fuzz"}:
                    self.fuzz()
                if self.args.phase in {"all", "mutations"}:
                    self.mutations()
            if self.report["source_sha256"] != program_digest(ROOT):
                raise RuntimeError("program inputs changed during tests; discard this result")
            self.report["status"] = "passed"
        except (OSError, RuntimeError, ValueError, subprocess.SubprocessError) as error:
            self.report["status"] = "failed"
            self.report["error"] = str(error)
            raise
        finally:
            self.write_report()
            print(f"Private campaign report: {self.out / 'report.json'}", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("pr", "coverage", "extended", "matrix"))
    parser.add_argument("--seed", type=int, default=0x4D4F4C504841)
    parser.add_argument("--replay", help="Private stateful-replay.json; runs only that generated sequence within the suite")
    parser.add_argument("--phase", choices=("all", "stateful", "fuzz", "mutations"), default="all")
    parser.add_argument("--fuzz-seconds", type=int, default=600)
    parser.add_argument("--fuzz-toolchain", default="nightly-2026-08-06", help="Separate fuzz compiler; never changes the program toolchain")
    args = parser.parse_args()
    if not 0 <= args.seed < 2**64 or args.fuzz_seconds < 1:
        parser.error("seed must fit u64 and fuzz-seconds must be positive")
    if args.command == "matrix":
        print(json.dumps(validate_matrix(), indent=2))
    else:
        Campaign(args).execute()


if __name__ == "__main__":
    try:
        main()
    except (OSError, RuntimeError, ValueError, subprocess.SubprocessError) as error:
        print(str(error), file=sys.stderr)
        sys.exit(1)
