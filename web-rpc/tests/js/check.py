#!/usr/bin/env python3
"""Extract the Javascript endpoints rendered into the test binaries and check them.

This does what an embedder does: build for wasm32, dump every `__web_rpc_*` custom section
with objcopy, and use the files. The browser tests load them from tests/js/generated/;
node syntax-checks each module and runs the shell's codec against the byte fixtures in
codec.test.mjs; and the output is compared with tests/js/expected/, which `--update`
rewrites.

Needs `rust-objcopy` (cargo-binutils with the llvm-tools component) or `llvm-objcopy`.
"""

import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

CRATE = Path(__file__).resolve().parents[2]
GENERATED = CRATE / "tests" / "js" / "generated"
EXPECTED = CRATE / "tests" / "js" / "expected"
PREFIX = "__web_rpc_"


def objcopy():
    if "OBJCOPY" in os.environ:
        return os.environ["OBJCOPY"]
    for candidate in ("rust-objcopy", "llvm-objcopy"):
        if shutil.which(candidate):
            return candidate
    sys.exit("check.py: neither rust-objcopy nor llvm-objcopy is on the PATH")


def test_binaries():
    """The wasm test binaries, from cargo's own report of what it built."""
    report = subprocess.run(
        ["cargo", "build", "--tests", "--target", "wasm32-unknown-unknown",
         "--message-format=json-render-diagnostics"],
        cwd=CRATE, check=True, capture_output=True, text=True,
    ).stdout
    for line in report.splitlines():
        message = json.loads(line)
        if message.get("reason") == "compiler-artifact" and message.get("executable"):
            yield Path(message["executable"])


def read_varint(data, offset):
    value = 0
    shift = 0
    while True:
        byte = data[offset]
        offset += 1
        value |= (byte & 0x7F) << shift
        if byte & 0x80 == 0:
            return value, offset
        shift += 7


def custom_section_names(binary):
    """The names of the custom sections of a wasm binary, from its section headers."""
    data = binary.read_bytes()
    offset = 8
    while offset < len(data):
        section_id = data[offset]
        size, offset = read_varint(data, offset + 1)
        if section_id == 0:
            length, name_offset = read_varint(data, offset)
            yield data[name_offset:name_offset + length].decode()
        offset += size


def extract(binary):
    classes = [
        name[len(PREFIX):-len("_js")]
        for name in custom_section_names(binary)
        if name.startswith(PREFIX) and name.endswith("_js")
    ]
    for class_name in classes:
        with tempfile.NamedTemporaryFile() as discarded:
            subprocess.run(
                [objcopy(),
                 f"--dump-section={PREFIX}{class_name}_js={GENERATED / class_name}.mjs",
                 f"--dump-section={PREFIX}{class_name}_d_ts={GENERATED / class_name}.d.ts",
                 binary, discarded.name],
                check=True,
            )
        print(f"{binary.name}: {class_name}")


def main():
    if GENERATED.exists():
        shutil.rmtree(GENERATED)
    GENERATED.mkdir()
    for binary in test_binaries():
        extract(binary)
    for module in sorted(GENERATED.glob("*.mjs")):
        subprocess.run(["node", "--check", module], check=True)
    subprocess.run(["node", "--test", CRATE / "tests" / "js" / "codec.test.mjs"], check=True)
    if "--update" in sys.argv[1:]:
        if EXPECTED.exists():
            shutil.rmtree(EXPECTED)
        shutil.copytree(GENERATED, EXPECTED)
        print(f"updated {EXPECTED}")
    else:
        subprocess.run(["diff", "-ru", EXPECTED, GENERATED], check=True)
        print(f"generated endpoints match {EXPECTED}")


if __name__ == "__main__":
    main()
