#!/usr/bin/env python3
"""Check the dependency direction of the rpcstack framework crates.

The framework crates must know nothing about Masa or about the runtime that
schedules tasks, so that Masa, rajomon and any new policy are instantiations
built on top of them. The rules, over each crate's direct dependencies of any
kind (normal, dev, build):

- `rpcstack-wire` is a leaf: no tonic, http, hyper, tokio, Masa or other
  rpcstack crate.
- `rpcstack` and `rpcstack-tonic` depend on no Masa crate, no rajomon, no
  hyper and no tokio or scheduler crate.
- `rpcstack-sched` has no dependencies at all.
- `rajomon` depends on the framework (`rpcstack`; `rpcstack-tonic` only for
  tests) and generic libraries: no Masa crate, no hyper, no scheduler crate.
- No framework crate depends on `rajomon`.
"""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]

MASA_CRATES = {"masa", "masa-core", "masa-policy", "masa-semantics"}
RUNTIME_CRATES = {"hyper", "tokio", "rpcstack-sched", "multiqueue-prio-queue"}
RPCSTACK_CRATES = {"rpcstack", "rpcstack-wire", "rpcstack-tonic", "rpcstack-sched"}

FORBIDDEN: dict[str, set[str]] = {
    "rpcstack-wire": MASA_CRATES
    | {"rajomon", "tailclipper"}
    | RUNTIME_CRATES
    | (RPCSTACK_CRATES - {"rpcstack-wire"})
    | {"tonic", "http"},
    "rpcstack": MASA_CRATES | RUNTIME_CRATES | {"rajomon", "tailclipper"},
    "rpcstack-tonic": MASA_CRATES | RUNTIME_CRATES | {"rajomon", "tailclipper"},
    "rajomon": MASA_CRATES | (RUNTIME_CRATES - {"tokio"}),
    "tailclipper": MASA_CRATES | (RUNTIME_CRATES - {"tokio"}),
}


def direct_dependencies(repo_root: Path) -> dict[str, list[str]]:
    metadata = json.loads(
        subprocess.run(
            ["cargo", "metadata", "--format-version", "1", "--no-deps"],
            cwd=repo_root,
            check=True,
            capture_output=True,
            text=True,
        ).stdout
    )
    return {
        package["name"]: [dep["name"] for dep in package["dependencies"]]
        for package in metadata["packages"]
    }


def validate_rpcstack_boundary(repo_root: Path = REPO_ROOT) -> list[str]:
    dependencies = direct_dependencies(repo_root)
    errors: list[str] = []

    for crate, forbidden in FORBIDDEN.items():
        if crate not in dependencies:
            errors.append(f"workspace has no crate '{crate}'")
            continue
        for dep in sorted(set(dependencies[crate]) & forbidden):
            errors.append(f"{crate} must not depend on '{dep}'")

    if "rpcstack-sched" not in dependencies:
        errors.append("workspace has no crate 'rpcstack-sched'")
    else:
        for dep in sorted(set(dependencies["rpcstack-sched"])):
            errors.append(f"rpcstack-sched must not depend on '{dep}'")

    return errors


def main() -> int:
    errors = validate_rpcstack_boundary()
    if errors:
        print("rpcstack dependency boundary violated:")
        for error in errors:
            print(f"  - {error}")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
