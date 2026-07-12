#!/usr/bin/env python3
"""Isolated recursive-scope scenario driver for rere.py lists.

Owns:
  - fresh temp project per scenario (never touches repo .mnema/)
  - fixed presentation env (NO_COLOR, TZ)
  - release CLI discovery
  - stable structural reports (slugs/membership/flags), not raw IDs

rere.py only compares shell stdout/stderr/returncode; this driver is the
semantic surface for recursive Journal-root vs strand-root claims.
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any, Callable


REPO_ROOT = Path(__file__).resolve().parents[2]


def find_mnema() -> Path:
    env = os.environ.get("CARGO_BIN_EXE_mnema") or os.environ.get("MNEMA_BIN")
    if env:
        path = Path(env)
        if path.is_file():
            return path
    candidates: list[Path] = []
    cargo_target = os.environ.get("CARGO_TARGET_DIR")
    if cargo_target:
        root = Path(cargo_target)
        candidates.extend(
            [
                root / "release" / "mnema.exe",
                root / "release" / "mnema",
                root / "debug" / "mnema.exe",
                root / "debug" / "mnema",
            ]
        )
    candidates.extend(
        [
            REPO_ROOT / "target" / "release" / "mnema.exe",
            REPO_ROOT / "target" / "release" / "mnema",
            REPO_ROOT / "target" / "debug" / "mnema.exe",
            REPO_ROOT / "target" / "debug" / "mnema",
        ]
    )
    for candidate in candidates:
        if candidate.is_file():
            return candidate
    which = shutil.which("mnema")
    if which:
        return Path(which)
    raise SystemExit(
        "ERROR: mnema binary not found; build with `cargo build --release` "
        "or set MNEMA_BIN / CARGO_BIN_EXE_mnema"
    )


class Project:
    def __init__(self, root: Path, binary: Path) -> None:
        self.root = root
        self.binary = binary
        self.env = os.environ.copy()
        self.env["NO_COLOR"] = "1"
        self.env["TZ"] = "UTC"
        # Keep journal discovery inside the temp tree only.
        self.env.pop("MNEMA_HOME", None)

    def run(
        self, args: list[str], stdin: str | None = None, check: bool = True
    ) -> subprocess.CompletedProcess[str]:
        cmd = [str(self.binary), "-C", str(self.root), *args]
        proc = subprocess.run(
            cmd,
            input=stdin,
            text=True,
            capture_output=True,
            env=self.env,
            cwd=str(self.root),
        )
        if check and proc.returncode != 0:
            raise SystemExit(
                f"mnema {args!r} failed ({proc.returncode})\n"
                f"stdout:\n{proc.stdout}\nstderr:\n{proc.stderr}"
            )
        return proc

    def run_json(self, args: list[str], stdin: str | None = None) -> Any:
        proc = self.run([*args, "--format", "json"], stdin=stdin)
        text = proc.stdout.strip()
        # Drop optional progress lines before the JSON object/array.
        start = text.find("{")
        if start < 0:
            start = text.find("[")
        if start < 0:
            raise SystemExit(f"no JSON in stdout for {args!r}:\n{proc.stdout}")
        return json.loads(text[start:])

    def init(self) -> None:
        self.run(["init"])

    def add(
        self,
        body: str,
        *,
        slug: str | None = None,
        parent: str | None = None,
        ref: str | None = None,
    ) -> str:
        args = ["add"]
        if slug is not None:
            args.extend(["--slug", slug])
        if parent is not None:
            args.extend(["--parent", parent])
        if ref is not None:
            args.extend(["--ref", ref])
        payload = self.run_json(args, stdin=body if body.endswith("\n") else body + "\n")
        return payload["id"]

    def append(self, strand_id: str, body: str, *, ref: str | None = None) -> None:
        args = ["append", "--id", strand_id]
        if ref is not None:
            args.extend(["--ref", ref])
        self.run(args, stdin=body if body.endswith("\n") else body + "\n")

    def link_belongs(self, child: str, parent: str) -> None:
        self.run(["link", child, parent, "--edge-type", "belongs-to"])

    def unlink_belongs(self, child: str, parent: str) -> None:
        self.run(["unlink", child, parent, "--edge-type", "belongs-to"])

    def orient(self, under: str | None = None, *, limit: int = 256) -> Any:
        # Adaptive menu defaults can hide older strands; membership claims need
        # an explicit high limit so depth chains stay complete.
        args = ["orient", "--limit", str(limit)]
        if under is not None:
            args.extend(["--id", under])
        return self.run_json(args)

    def list_strands(self, under: str | None = None) -> Any:
        args = ["list"]
        if under is not None:
            args.extend(["--under", under])
        return self.run_json(args)

    def timeline(self, under: str | None = None) -> Any:
        args = ["timeline"]
        if under is not None:
            args.extend(["--under", under])
        return self.run_json(args)


def active_slugs(orient: Any) -> list[str]:
    rows = orient.get("active") or []
    slugs = []
    for row in rows:
        slug = row.get("slug")
        if slug:
            slugs.append(slug)
        else:
            # Fallback label from summary prefix for unlabeled strands.
            summary = (row.get("summary") or "").strip()
            slugs.append(summary or row.get("id", "")[:12])
    return sorted(slugs)


def scope_kind(orient: Any) -> str:
    scope = orient.get("scope") or {}
    return str(scope.get("kind") or "unknown")


def report(lines: list[str]) -> None:
    sys.stdout.write("\n".join(lines) + "\n")


def scenario_smoke_fresh_journal(p: Project) -> None:
    p.init()
    orient = p.orient()
    report(
        [
            "scenario: smoke/fresh-journal-orient",
            f"scope_kind: {scope_kind(orient)}",
            f"active_count: {len(orient.get('active') or [])}",
            f"closed_count: {orient.get('closed_count')}",
            f"hidden_count: {orient.get('hidden_count')}",
            "status: ok",
        ]
    )


def scenario_smoke_journal_vs_subtree(p: Project) -> None:
    p.init()
    root = p.add("[task] fixture root\n", slug="root")
    child = p.add("[task] fixture child\n", slug="child", parent=root)
    p.add("[task] fixture grandchild\n", slug="grandchild", parent=child)
    p.add("[task] fixture outsider\n", slug="outsider")
    journal = p.orient()
    subtree = p.orient(under=root)
    journal_slugs = active_slugs(journal)
    subtree_slugs = active_slugs(subtree)
    # JournalScope = downward closure of the virtual root (whole forest).
    # Subtree(root) = root + belongs-to descendants only; outsider stays out.
    report(
        [
            "scenario: smoke/journal-vs-subtree",
            f"journal_scope_kind: {scope_kind(journal)}",
            f"subtree_scope_kind: {scope_kind(subtree)}",
            f"journal_member_slugs: {','.join(journal_slugs)}",
            f"subtree_member_slugs: {','.join(subtree_slugs)}",
            f"outsider_in_journal: {'yes' if 'outsider' in journal_slugs else 'no'}",
            f"outsider_in_subtree: {'yes' if 'outsider' in subtree_slugs else 'no'}",
            f"child_in_journal: {'yes' if 'child' in journal_slugs else 'no'}",
            f"child_in_subtree: {'yes' if 'child' in subtree_slugs else 'no'}",
            "status: ok",
        ]
    )
    if "outsider" not in journal_slugs:
        raise SystemExit("FAIL: journal scope missing outsider top-level strand")
    if "outsider" in subtree_slugs:
        raise SystemExit("FAIL: outsider leaked into subtree orient")
    if "child" not in journal_slugs or "child" not in subtree_slugs:
        raise SystemExit("FAIL: child missing from journal or subtree membership")


def scenario_smoke_refs_do_not_expand(p: Project) -> None:
    p.init()
    root = p.add("[task] fixture root\n", slug="root")
    child = p.add("[task] fixture child\n", slug="child", parent=root)
    evidence = p.add("[evidence] fixture rationale\n", slug="evidence")
    p.append(child, "[progress] cited out-of-tree evidence\n", ref=evidence)
    subtree = p.orient(under=root)
    slugs = active_slugs(subtree)
    report(
        [
            "scenario: smoke/refs-do-not-expand-scope",
            f"subtree_member_slugs: {','.join(slugs)}",
            f"evidence_in_subtree: {'yes' if 'evidence' in slugs else 'no'}",
            "status: ok",
        ]
    )
    if "evidence" in slugs:
        raise SystemExit("FAIL: ref target expanded subtree membership")


def scenario_full_depth_chain(p: Project) -> None:
    p.init()
    depth = 10
    # Avoid pure-hex slugs (CLI rejects them as hash-prefix collisions).
    def depth_slug(i: int) -> str:
        return f"depth-{i}"

    parent = p.add("[task] depth-0\n", slug=depth_slug(0))
    ids = [parent]
    for i in range(1, depth + 1):
        parent = p.add(f"[task] depth-{i}\n", slug=depth_slug(i), parent=parent)
        ids.append(parent)
    # Mid-depth strand root must see only itself + descendants.
    mid = ids[5]
    mid_orient = p.orient(under=mid)
    mid_slugs = active_slugs(mid_orient)
    journal = p.orient()
    report(
        [
            "scenario: full/depth-chain-orient",
            f"chain_depth: {depth}",
            f"journal_member_slugs: {','.join(active_slugs(journal))}",
            f"mid_depth5_scope_kind: {scope_kind(mid_orient)}",
            f"mid_depth5_member_slugs: {','.join(mid_slugs)}",
            f"depth0_in_mid: {'yes' if 'depth-0' in mid_slugs else 'no'}",
            f"depth4_in_mid: {'yes' if 'depth-4' in mid_slugs else 'no'}",
            f"depth5_in_mid: {'yes' if 'depth-5' in mid_slugs else 'no'}",
            f"depth10_in_mid: {'yes' if 'depth-10' in mid_slugs else 'no'}",
            "status: ok",
        ]
    )
    if "depth-0" in mid_slugs or "depth-4" in mid_slugs:
        raise SystemExit("FAIL: ancestor leaked into mid-depth subtree")
    if "depth-5" not in mid_slugs or "depth-10" not in mid_slugs:
        raise SystemExit("FAIL: mid-depth subtree missing self/descendant")


def scenario_full_reparent_join_leave(p: Project) -> None:
    p.init()
    root = p.add("[task] root\n", slug="root")
    child = p.add("[task] child\n", slug="child", parent=root)
    joiner = p.add("[task] joiner\n", slug="joiner")
    before = active_slugs(p.orient(under=root))
    p.link_belongs(joiner, root)
    after_join = active_slugs(p.orient(under=root))
    p.unlink_belongs(child, root)
    after_leave = active_slugs(p.orient(under=root))
    report(
        [
            "scenario: full/reparent-join-leave",
            f"before_slugs: {','.join(before)}",
            f"after_join_slugs: {','.join(after_join)}",
            f"after_leave_slugs: {','.join(after_leave)}",
            f"joiner_before: {'yes' if 'joiner' in before else 'no'}",
            f"joiner_after_join: {'yes' if 'joiner' in after_join else 'no'}",
            f"child_after_leave: {'yes' if 'child' in after_leave else 'no'}",
            "status: ok",
        ]
    )
    if "joiner" in before or "joiner" not in after_join:
        raise SystemExit("FAIL: join did not add joiner to subtree")
    if "child" in after_leave:
        raise SystemExit("FAIL: leave did not remove child from subtree")


def scenario_crash_complete_batch_readable(p: Project) -> None:
    """Skeleton durable-state claim: a finished parent+refs write is fully readable.

    Mid-write abort/failpoint atomicity stays in the Rust crash_atomicity suite;
    this scenario only freezes the recursive post-condition for complete batches.
    """
    p.init()
    root = p.add("[task] root\n", slug="root")
    child = p.add("[task] child\n", slug="child", parent=root)
    evidence = p.add("[evidence] note\n", slug="evidence")
    p.append(child, "[progress] with ref\n", ref=evidence)
    doctor = p.run(["doctor", "journal"])
    orient = p.orient(under=root)
    slugs = active_slugs(orient)
    report(
        [
            "scenario: crash/complete-batch-strict-readable",
            f"doctor_exit: {doctor.returncode}",
            f"subtree_member_slugs: {','.join(slugs)}",
            f"evidence_in_subtree: {'yes' if 'evidence' in slugs else 'no'}",
            "status: ok",
        ]
    )
    if doctor.returncode != 0:
        raise SystemExit("FAIL: doctor journal failed after complete batch")
    if "evidence" in slugs:
        raise SystemExit("FAIL: ref expanded scope after complete batch")


SCENARIOS: dict[str, Callable[[Project], None]] = {
    "smoke/fresh-journal-orient": scenario_smoke_fresh_journal,
    "smoke/journal-vs-subtree": scenario_smoke_journal_vs_subtree,
    "smoke/refs-do-not-expand-scope": scenario_smoke_refs_do_not_expand,
    "full/depth-chain-orient": scenario_full_depth_chain,
    "full/reparent-join-leave": scenario_full_reparent_join_leave,
    "crash/complete-batch-strict-readable": scenario_crash_complete_batch_readable,
}


def main(argv: list[str]) -> None:
    if len(argv) != 1 or argv[0] in ("-h", "--help"):
        names = "\n  ".join(sorted(SCENARIOS))
        print(f"Usage: {sys.argv[0]} <scenario-id>")
        print("Scenarios:")
        print(f"  {names}")
        raise SystemExit(0 if argv and argv[0] in ("-h", "--help") else 1)
    name = argv[0]
    if name not in SCENARIOS:
        print(f"ERROR: unknown scenario {name!r}")
        raise SystemExit(1)
    binary = find_mnema()
    tmp = Path(tempfile.mkdtemp(prefix="mnema-rere-"))
    try:
        project = Project(tmp, binary)
        SCENARIOS[name](project)
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    main(sys.argv[1:])
