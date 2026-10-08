#!/usr/bin/env python3
"""Scan release source/index/history without printing credential values."""

import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parent.parent
ALLOWLIST = ROOT / "scripts/test-key-allowlist.json"
SECRET_FIELDS = {
    "privatekey", "privatekeys", "secretkey", "secretkeys", "secretkeybase58",
    "privatekey0x", "mnemonic", "seedphrase", "apikey", "accesstoken",
    "refreshtoken", "clientsecret", "password",
}
TOKEN_PATTERNS = (
    re.compile(r"-----BEGIN (?:RSA |EC |OPENSSH |ENCRYPTED )?PRIVATE KEY-----"),
    re.compile(r"\b(?:gh[pousr]_[A-Za-z0-9]{36,}|github_pat_[A-Za-z0-9_]{50,})\b"),
    re.compile(r"\bAKIA[A-Z0-9]{16}\b"),
    re.compile(r"\bxox[baprs]-[A-Za-z0-9-]{20,}\b"),
    re.compile(r"\bsk-(?:proj-|svcacct-)?[A-Za-z0-9_-]{32,}\b"),
)


def git(*args):
    return subprocess.check_output(["git", "-C", str(ROOT), *args])


def populated(value):
    if value is None or value == "" or value == [] or value == {}:
        return False
    if isinstance(value, str):
        text = value.strip().lower()
        return not (
            not text or text.startswith(("<", "${", "your_", "your-", "example", "replace"))
            or text in {"changeme", "placeholder", "redacted"}
        )
    return True


def structured_secret(value):
    if isinstance(value, dict):
        for key, child in value.items():
            normalized = re.sub(r"[^a-z0-9]", "", key.lower())
            if normalized in SECRET_FIELDS and populated(child):
                return True
            if structured_secret(child):
                return True
    elif isinstance(value, list):
        return any(structured_secret(child) for child in value)
    return False


def detect(path, raw):
    """Return categories only; callers must never log raw contents."""
    found = set()
    text = raw.decode("utf-8", errors="replace")
    if any(pattern.search(text) for pattern in TOKEN_PATTERNS):
        found.add("credential-pattern")
    try:
        value = json.loads(text)
    except (ValueError, RecursionError):
        value = None
    if (
        isinstance(value, list) and len(value) == 64
        and all(type(n) is int and 0 <= n <= 255 for n in value)
    ):
        found.add("solana-keypair")
    if structured_secret(value):
        found.add("structured-secret")
    name = Path(path).name
    if name == ".env" or name.startswith(".env."):
        for line in text.splitlines():
            line = line.strip()
            if not line or line.startswith("#") or "=" not in line:
                continue
            _, value = line.removeprefix("export ").split("=", 1)
            if populated(value.strip().strip("\"'")):
                found.add("populated-env")
                break
    return found


def load_allowlist():
    data = json.loads(ALLOWLIST.read_text())
    if data.get("version") != 1:
        raise ValueError("Unsupported test-key allowlist version")
    return data["entries"]


def allowed(entries, mode, path, raw, category):
    digest = hashlib.sha256(raw).hexdigest()
    return any(
        item["sha256"] == digest and category in item["categories"]
        and (mode == "history" or path in item.get("current_paths", []))
        for item in entries
    )


def files(mode):
    if mode == "worktree":
        names = git("ls-files", "--cached", "--others", "--exclude-standard", "-z")
        for name in sorted(set(names.split(b"\0")) - {b""}):
            path = name.decode("utf-8", errors="surrogateescape")
            file = ROOT / path
            if file.is_symlink():
                # Scan the link text, without reading files outside this checkout.
                import os
                yield path, os.readlink(file).encode()
            elif file.is_file():
                yield path, file.read_bytes()
    elif mode == "staged":
        for record in git("ls-files", "--stage", "-z").split(b"\0"):
            if not record:
                continue
            meta, path = record.split(b"\t", 1)
            permissions, oid, stage = meta.split()
            if stage != b"0":
                raise ValueError("Resolve index conflicts before scanning")
            if permissions == b"160000":
                continue  # Submodule commits are not blobs.
            yield path.decode("utf-8", errors="surrogateescape"), git("cat-file", "blob", oid.decode())
    else:
        # Each object is read once. Historical exceptions bind exact content,
        # regardless of its historical filename. Current exceptions also bind paths.
        records = [row.split(b" ", 1) for row in git("rev-list", "--objects", "--all").splitlines()]
        named = [(oid, path) for oid, *tail in records if tail for path in tail]
        request = b"".join(oid + b"\n" for oid, _ in named)
        types = subprocess.run(
            ["git", "-C", str(ROOT), "cat-file", "--batch-check=%(objecttype)"],
            input=request, stdout=subprocess.PIPE, check=True,
        ).stdout.splitlines()
        blobs = [(oid, path) for (oid, path), kind in zip(named, types) if kind == b"blob"]
        output = subprocess.run(
            ["git", "-C", str(ROOT), "cat-file", "--batch"],
            input=b"".join(oid + b"\n" for oid, _ in blobs),
            stdout=subprocess.PIPE, check=True,
        ).stdout
        offset = 0
        for oid, path in blobs:
            end = output.index(b"\n", offset)
            returned_oid, kind, size = output[offset:end].split()
            if returned_oid != oid or kind != b"blob":
                raise ValueError("Unexpected Git batch response")
            offset = end + 1
            size = int(size)
            raw = output[offset:offset + size]
            offset += size + 1
            yield path.decode("utf-8", errors="surrogateescape"), raw


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    modes = parser.add_mutually_exclusive_group()
    modes.add_argument("--history", action="store_true")
    modes.add_argument("--staged", action="store_true")
    args = parser.parse_args()
    mode = "history" if args.history else "staged" if args.staged else "worktree"
    try:
        entries = load_allowlist()
        count = exemptions = failures = 0
        for path, raw in files(mode):
            count += 1
            for category in sorted(detect(path, raw)):
                if allowed(entries, mode, path, raw, category):
                    exemptions += 1
                else:
                    print(f"FAIL {category}: {json.dumps(path)}")
                    failures += 1
        print(f"{mode}: {count} files/blobs; {exemptions} reviewed test exceptions; {failures} findings")
        return 1 if failures else 0
    except (OSError, ValueError, subprocess.CalledProcessError):
        print("Secret scan could not complete; check Git access and allowlist format.", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
