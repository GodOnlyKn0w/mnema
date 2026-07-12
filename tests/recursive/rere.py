#!/usr/bin/env python3
# Copyright 2024 Alexey Kutepov <reximkut@gmail.com>
#
# Permission is hereby granted, free of charge, to any person obtaining
# a copy of this software and associated documentation files (the
# "Software"), to deal in the Software without restriction, including
# without limitation the rights to use, copy, modify, merge, publish,
# distribute, sublicense, and/or sell copies of the Software, and to
# permit persons to whom the Software is furnished to do so, subject to
# the following conditions:
#
# The above copyright notice and this permission notice shall be
# included in all copies or substantial portions of the Software.
#
# THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND,
# EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF
# MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND
# NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE
# LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION
# OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION
# WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
#
# Vendored into mnema (tests/recursive/). See SOURCE for pin and policy.
# Local adaptations vs upstream:
# - resolve a POSIX shell (Git sh on Windows) instead of hard-coding "sh"
# - refuse accidental record when MNEMA_RERE_REPLAY_ONLY=1 (CI/async gate)

import os
import shutil
import subprocess
import sys
from difflib import unified_diff
from typing import BinaryIO


def resolve_shell() -> str:
    for name in ("sh", "bash"):
        path = shutil.which(name)
        if path:
            return path
    raise SystemExit(
        "ERROR: no POSIX shell (sh/bash) on PATH; install Git for Windows or similar"
    )


def read_blob_field(f: BinaryIO, name: bytes) -> bytes:
    line = f.readline()
    field = b":b " + name + b" "
    assert line.startswith(field), field
    assert line.endswith(b"\n")
    size = int(line[len(field) : -1])
    blob = f.read(size)
    assert f.read(1) == b"\n"
    return blob


def read_int_field(f: BinaryIO, name: bytes) -> int:
    line = f.readline()
    field = b":i " + name + b" "
    assert line.startswith(field)
    assert line.endswith(b"\n")
    return int(line[len(field) : -1])


def write_int_field(f: BinaryIO, name: bytes, value: int) -> None:
    f.write(b":i %s %d\n" % (name, value))


def write_blob_field(f: BinaryIO, name: bytes, blob: bytes) -> None:
    f.write(b":b %s %d\n" % (name, len(blob)))
    f.write(blob)
    f.write(b"\n")


def capture(shell: str, sh_path: str) -> dict:
    print(f"CAPTURING: {shell}")
    process = subprocess.run([sh_path, "-c", shell], capture_output=True)
    return {
        "shell": shell,
        "returncode": process.returncode,
        "stdout": process.stdout,
        "stderr": process.stderr,
    }


def load_list(file_path: str) -> list[str]:
    with open(file_path, encoding="utf-8") as f:
        return [
            line.strip()
            for line in f
            if line.strip() and not line.lstrip().startswith("#")
        ]


def dump_snapshots(file_path: str, snapshots: list[dict]) -> None:
    with open(file_path, "wb") as f:
        write_int_field(f, b"count", len(snapshots))
        for snapshot in snapshots:
            write_blob_field(f, b"shell", bytes(snapshot["shell"], "utf-8"))
            write_int_field(f, b"returncode", snapshot["returncode"])
            write_blob_field(f, b"stdout", snapshot["stdout"])
            write_blob_field(f, b"stderr", snapshot["stderr"])


def load_snapshots(file_path: str) -> list[dict]:
    snapshots = []
    with open(file_path, "rb") as f:
        count = read_int_field(f, b"count")
        for _ in range(count):
            shell = read_blob_field(f, b"shell")
            returncode = read_int_field(f, b"returncode")
            stdout = read_blob_field(f, b"stdout")
            stderr = read_blob_field(f, b"stderr")
            snapshots.append(
                {
                    "shell": shell,
                    "returncode": returncode,
                    "stdout": stdout,
                    "stderr": stderr,
                }
            )
    return snapshots


def main() -> None:
    program_name, *argv = sys.argv

    if len(argv) == 0:
        print(f"Usage: {program_name} <record|replay> <test.list>")
        print("ERROR: no subcommand is provided")
        raise SystemExit(1)
    subcommand, *argv = argv

    if subcommand == "record":
        if os.environ.get("MNEMA_RERE_REPLAY_ONLY") == "1":
            print(
                "ERROR: record is forbidden while MNEMA_RERE_REPLAY_ONLY=1 "
                "(automated gates must only replay)"
            )
            raise SystemExit(2)
        if len(argv) == 0:
            print(f"Usage: {program_name} {subcommand} <test.list>")
            print("ERROR: no test.list is provided")
            raise SystemExit(1)
        test_list_path, *argv = argv
        sh_path = resolve_shell()
        snapshots = [capture(shell, sh_path) for shell in load_list(test_list_path)]
        dump_snapshots(f"{test_list_path}.bi", snapshots)
        print(f"WROTE: {test_list_path}.bi ({len(snapshots)} snapshots)")
    elif subcommand == "replay":
        if len(argv) == 0:
            print(f"Usage: {program_name} {subcommand} <test.list>")
            print("ERROR: no test.list is provided")
            raise SystemExit(1)
        test_list_path, *argv = argv
        bi_path = f"{test_list_path}.bi"
        if not os.path.isfile(bi_path):
            print(f"ERROR: missing snapshot artifact {bi_path}")
            print(
                f"NOTE: maintainer must run `{program_name} record {test_list_path}` "
                "explicitly; CI never records"
            )
            raise SystemExit(1)
        sh_path = resolve_shell()
        shells = load_list(test_list_path)
        snapshots = load_snapshots(bi_path)

        if len(shells) != len(snapshots):
            print(f"UNEXPECTED: amount of shell commands in {test_list_path}")
            print(f"    EXPECTED: {len(snapshots)}")
            print(f"    ACTUAL:   {len(shells)}")
            print(
                f"NOTE: You may want to do `{program_name} record {test_list_path}` "
                f"to update {bi_path}"
            )
            raise SystemExit(1)

        for shell, snapshot in zip(shells, snapshots):
            print(f"REPLAYING: {shell}")
            snapshot_shell = snapshot["shell"].decode("utf-8")
            if shell != snapshot_shell:
                print("UNEXPECTED: shell command")
                print(f"    EXPECTED: {snapshot_shell}")
                print(f"    ACTUAL:   {shell}")
                print(
                    f"NOTE: You may want to do `{program_name} record {test_list_path}` "
                    f"to update {bi_path}"
                )
                raise SystemExit(1)
            process = subprocess.run([sh_path, "-c", shell], capture_output=True)
            failed = False
            if process.returncode != snapshot["returncode"]:
                print("UNEXPECTED: return code")
                print(f"    EXPECTED: {snapshot['returncode']}")
                print(f"    ACTUAL:   {process.returncode}")
                failed = True
            if process.stdout != snapshot["stdout"]:
                a = snapshot["stdout"].decode("utf-8", errors="replace").splitlines(
                    keepends=True
                )
                b = process.stdout.decode("utf-8", errors="replace").splitlines(
                    keepends=True
                )
                print("UNEXPECTED: stdout")
                for line in unified_diff(a, b, fromfile="expected", tofile="actual"):
                    print(line, end="")
                failed = True
            if process.stderr != snapshot["stderr"]:
                a = snapshot["stderr"].decode("utf-8", errors="replace").splitlines(
                    keepends=True
                )
                b = process.stderr.decode("utf-8", errors="replace").splitlines(
                    keepends=True
                )
                print("UNEXPECTED: stderr")
                for line in unified_diff(a, b, fromfile="expected", tofile="actual"):
                    print(line, end="")
                failed = True
            if failed:
                raise SystemExit(1)
        print("OK")
    else:
        print(f"ERROR: unknown subcommand {subcommand}")
        raise SystemExit(1)


if __name__ == "__main__":
    main()
