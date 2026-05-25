#!/usr/bin/env python3
from __future__ import annotations

import re
import sys
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[1]
TONIC_CRATE = Path("libs/tonic/tonic")
TONIC_MANIFEST = TONIC_CRATE / "Cargo.toml"
TONIC_SRC = TONIC_CRATE / "src"
FORBIDDEN_PACKAGES = ("masa", "masa-core", "masa-policy")

TABLE_RE = re.compile(r"^\s*\[([^\[\]]+)]\s*(?:#.*)?$")
DEPENDENCY_KEY_RE = re.compile(r'^\s*["\']?([A-Za-z0-9_.-]+)["\']?\s*=')
PACKAGE_RE = re.compile(r'\bpackage\s*=\s*["\']([^"\']+)["\']')
EXTERN_CRATE_RE = re.compile(r"\bextern\s+crate\s+(masa|masa_core|masa_policy)\b")
CRATE_PATH_RE = re.compile(r"(?<![A-Za-z0-9_:])(?:masa|masa_core|masa_policy)::")


def _is_dependency_table(table_name: str) -> bool:
    parts = [part.strip().strip("\"'") for part in table_name.split(".")]
    return bool(parts) and parts[-1] in {
        "dependencies",
        "dev-dependencies",
        "build-dependencies",
    }


def _strip_line_comment(line: str) -> str:
    in_single = False
    in_double = False
    escaped = False

    for index, char in enumerate(line):
        if escaped:
            escaped = False
            continue
        if char == "\\" and in_double:
            escaped = True
            continue
        if char == '"' and not in_single:
            in_double = not in_double
            continue
        if char == "'" and not in_double:
            in_single = not in_single
            continue
        if char == "#" and not in_single and not in_double:
            return line[:index]
    return line


def _manifest_dependency_errors(manifest_path: Path) -> list[str]:
    errors: list[str] = []
    current_table: str | None = None

    for line_number, line in enumerate(manifest_path.read_text().splitlines(), start=1):
        table_match = TABLE_RE.match(line)
        if table_match:
            current_table = table_match.group(1)
            continue

        if current_table is None or not _is_dependency_table(current_table):
            continue

        uncommented = _strip_line_comment(line)
        key_match = DEPENDENCY_KEY_RE.match(uncommented)
        if key_match and key_match.group(1) in FORBIDDEN_PACKAGES:
            errors.append(
                f"{manifest_path}:{line_number}: dependency '{key_match.group(1)}' is not allowed"
            )
            continue

        package_match = PACKAGE_RE.search(uncommented)
        if package_match and package_match.group(1) in FORBIDDEN_PACKAGES:
            errors.append(
                f"{manifest_path}:{line_number}: package '{package_match.group(1)}' is not allowed"
            )

    return errors


def _source_dependency_errors(src_dir: Path) -> list[str]:
    errors: list[str] = []

    for source_path in sorted(src_dir.rglob("*.rs")):
        for line_number, line in enumerate(
            source_path.read_text().splitlines(), start=1
        ):
            uncommented = line.split("//", 1)[0]
            extern_match = EXTERN_CRATE_RE.search(uncommented)
            path_match = CRATE_PATH_RE.search(uncommented)

            if extern_match:
                errors.append(
                    f"{source_path}:{line_number}: extern crate '{extern_match.group(1)}' is not allowed"
                )
            elif path_match:
                errors.append(
                    f"{source_path}:{line_number}: direct Masa crate path '{path_match.group(0)}' is not allowed"
                )

    return errors


def validate_tonic_masa_boundary(repo_root: Path = REPO_ROOT) -> list[str]:
    """Vendored tonic library code must keep Masa crates outside its boundary.

    This intentionally scopes the invariant to libs/tonic/tonic/Cargo.toml and
    libs/tonic/tonic/src. Vendored tonic tests, examples, interop crates, and
    tonic-web integration tests may use Masa crates as external integration
    coverage without making the tonic library crate depend on Masa policy or
    wire-metadata ownership.
    """
    manifest_path = repo_root / TONIC_MANIFEST
    src_dir = repo_root / TONIC_SRC

    errors = _manifest_dependency_errors(manifest_path)
    errors.extend(_source_dependency_errors(src_dir))
    return errors


def main() -> int:
    errors = validate_tonic_masa_boundary()
    if errors:
        print("Vendored tonic must not depend on Masa crates:")
        for error in errors:
            print(f"  - {error}")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
