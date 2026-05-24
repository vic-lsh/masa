#!/usr/bin/env python3
from __future__ import annotations

from collections.abc import Mapping, Sequence
from pathlib import Path
import re
import sys
from typing import Any

try:
    import tomllib
except ModuleNotFoundError:  # pragma: no cover - exercised only on Python 3.10.
    import tomli as tomllib  # type: ignore[no-redef]


REPO_ROOT = Path(__file__).resolve().parents[1]
MATRIX_PATH = "policy_matrix.toml"
POLICY_FEATURE_PREFIXES = ("sched_", "abort_", "ac_", "est_")
POLICY_FEATURE_NAMES = {"estimator", "signal_slack", "trace_queue_latency"}


def _load_toml(path: Path) -> dict[str, Any]:
    with path.open("rb") as handle:
        return tomllib.load(handle)


def _as_str_list(value: Any, name: str) -> list[str]:
    if not isinstance(value, list) or not all(isinstance(item, str) for item in value):
        raise TypeError(f"{name} must be a list of strings")
    return value


def _as_str_map(value: Any, name: str) -> dict[str, str]:
    if not isinstance(value, dict) or not all(
        isinstance(key, str) and isinstance(item, str) for key, item in value.items()
    ):
        raise TypeError(f"{name} must be a string-to-string table")
    return dict(value)


def _as_str_list_map(value: Any, name: str) -> dict[str, list[str]]:
    if not isinstance(value, dict):
        raise TypeError(f"{name} must be a table")
    result: dict[str, list[str]] = {}
    for key, item in value.items():
        if not isinstance(key, str):
            raise TypeError(f"{name} has a non-string key")
        result[key] = _as_str_list(item, f"{name}.{key}")
    return result


def _feature_set(items: Sequence[str]) -> set[str]:
    return set(items)


def _is_policy_feature(name: str) -> bool:
    return name in POLICY_FEATURE_NAMES or name.startswith(POLICY_FEATURE_PREFIXES)


def _split_combo(combo: str) -> list[str]:
    return [flag.strip() for flag in combo.split(",") if flag.strip()]


def _expand_implied_flags(
    flags: set[str], implies: Mapping[str, Sequence[str]]
) -> set[str]:
    expanded = set(flags)
    changed = True
    while changed:
        changed = False
        for flag in list(expanded):
            for implied in implies.get(flag, ()):
                if implied not in expanded:
                    expanded.add(implied)
                    changed = True
    return expanded


def _extract_shell_array(path: Path, name: str) -> list[str]:
    text = path.read_text(encoding="utf-8")
    match = re.search(rf"^{re.escape(name)}=\(\n(?P<body>.*?)\n\)", text, re.M | re.S)
    if match is None:
        raise ValueError(f"Could not find shell array {name!r} in {path}")
    return re.findall(r'"([^"]*)"', match.group("body"))


def _load_cargo_features(path: Path) -> dict[str, list[str]]:
    raw_features = _load_toml(path).get("features", {})
    if not isinstance(raw_features, dict):
        raise TypeError(f"{path} [features] must be a table")
    features: dict[str, list[str]] = {}
    for name, deps in raw_features.items():
        if not isinstance(name, str):
            raise TypeError(f"{path} has a non-string feature name")
        features[name] = _as_str_list(deps, f"{path}:{name}")
    return features


def _compare_sequence(
    errors: list[str], label: str, actual: Sequence[str], expected: Sequence[str]
) -> None:
    if list(actual) != list(expected):
        errors.append(
            f"{label} drifted:\n"
            f"  expected: {list(expected)}\n"
            f"  actual:   {list(actual)}"
        )


def _compare_set(
    errors: list[str], label: str, actual: Sequence[str], expected: Sequence[str]
) -> None:
    if _feature_set(actual) != _feature_set(expected):
        errors.append(
            f"{label} drifted:\n"
            f"  expected: {sorted(expected)}\n"
            f"  actual:   {sorted(actual)}"
        )


def _validate_combo_list(
    errors: list[str],
    label: str,
    combos: Sequence[str],
    known_flags: set[str],
    implies: Mapping[str, Sequence[str]],
    exclusive_groups: Mapping[str, Sequence[str]],
) -> None:
    for combo in combos:
        flags = set(_split_combo(combo))
        unknown = flags - known_flags
        if unknown:
            errors.append(
                f"{label} combo {combo!r} contains unknown flags: {sorted(unknown)}"
            )
        expanded = _expand_implied_flags(flags, implies)
        for flag, deps in implies.items():
            if flag in flags and not set(deps).issubset(expanded):
                errors.append(
                    f"{label} combo {combo!r} does not satisfy implications for {flag}"
                )
        for group_name, group_flags in exclusive_groups.items():
            selected = flags & set(group_flags)
            if len(selected) > 1:
                errors.append(
                    f"{label} combo {combo!r} selects mutually exclusive "
                    f"{group_name} flags: {sorted(selected)}"
                )


def _validate_app_crates(
    errors: list[str], repo_root: Path, manifest: Mapping[str, Any]
) -> None:
    app_crates = manifest["app_crates"]
    paths = _as_str_list(app_crates["paths"], "app_crates.paths")
    expected_features = _as_str_list_map(app_crates["features"], "app_crates.features")

    for rel_path in paths:
        actual = _load_cargo_features(repo_root / rel_path)
        undocumented = {
            feature
            for feature in actual
            if _is_policy_feature(feature) and feature not in expected_features
        }
        if undocumented:
            errors.append(
                f"{rel_path} has policy-looking features not listed in {MATRIX_PATH}: "
                f"{sorted(undocumented)}"
            )

        for feature, expected_deps in expected_features.items():
            if feature not in actual:
                errors.append(f"{rel_path} is missing feature {feature!r}")
                continue
            _compare_set(
                errors,
                f"{rel_path} feature {feature!r}",
                actual[feature],
                expected_deps,
            )


def _validate_cargo_crates(
    errors: list[str],
    repo_root: Path,
    manifest: Mapping[str, Any],
    known_flags: set[str],
) -> None:
    cargo = manifest["cargo"]
    if not isinstance(cargo, dict):
        raise TypeError("cargo must be a table")

    for rel_path, spec in cargo.items():
        if not isinstance(rel_path, str) or not isinstance(spec, dict):
            raise TypeError("cargo entries must be tables keyed by path")
        expected_features = _as_str_list(spec["features"], f"cargo.{rel_path}.features")
        expected_deps = _as_str_list_map(spec.get("deps", {}), f"cargo.{rel_path}.deps")
        actual = _load_cargo_features(repo_root / rel_path)

        missing = set(expected_features) - set(actual)
        if missing:
            errors.append(f"{rel_path} is missing policy features: {sorted(missing)}")

        undocumented = {
            feature
            for feature in actual
            if _is_policy_feature(feature)
            and feature not in known_flags
            and feature not in expected_features
        }
        if undocumented:
            errors.append(
                f"{rel_path} has policy-looking features not listed in {MATRIX_PATH}: "
                f"{sorted(undocumented)}"
            )

        unexpected_known = (set(actual) & known_flags) - set(expected_features)
        if unexpected_known:
            errors.append(
                f"{rel_path} has known policy features outside its manifest section: "
                f"{sorted(unexpected_known)}"
            )

        for feature in expected_features:
            if feature not in actual:
                continue
            _compare_set(
                errors,
                f"{rel_path} feature {feature!r}",
                actual[feature],
                expected_deps.get(feature, []),
            )


def _validate_python_policy(
    errors: list[str], repo_root: Path, manifest: Mapping[str, Any]
) -> None:
    sys.path.insert(0, str(repo_root))
    from exp_runner.runner import policy  # noqa: PLC0415

    expected = manifest["python_policy"]
    if not isinstance(expected, dict):
        raise TypeError("python_policy must be a table")

    prio = _as_str_map(expected["prio"], "python_policy.prio")
    est = _as_str_map(expected["est"], "python_policy.est")
    drop = _as_str_map(expected["drop"], "python_policy.drop")
    ac = _as_str_map(expected["ac"], "python_policy.ac")
    ignored = _as_str_list(expected["ignored"], "python_policy.ignored")
    precedence = _as_str_list(
        expected["prio_precedence"], "python_policy.prio_precedence"
    )

    if policy._PRIO_MAP != prio:
        errors.append(
            f"exp_runner/runner/policy.py _PRIO_MAP drifted from {MATRIX_PATH}"
        )
    if policy._EST_MAP != est:
        errors.append(
            f"exp_runner/runner/policy.py _EST_MAP drifted from {MATRIX_PATH}"
        )
    if policy._DROP_MAP != drop:
        errors.append(
            f"exp_runner/runner/policy.py _DROP_MAP drifted from {MATRIX_PATH}"
        )
    if policy._AC_MAP != ac:
        errors.append(f"exp_runner/runner/policy.py _AC_MAP drifted from {MATRIX_PATH}")
    _compare_sequence(
        errors,
        "exp_runner/runner/policy.py _PRIO_PRIORITY",
        policy._PRIO_PRIORITY,
        precedence,
    )
    _compare_set(
        errors,
        "exp_runner/runner/policy.py _IGNORED_FLAGS",
        policy._IGNORED_FLAGS,
        ignored,
    )

    parser_flags = set(prio) | set(est) | set(drop) | set(ac) | set(ignored)
    expected_parser_flags = set(_as_str_list(manifest["flags"]["known"], "flags.known"))
    if parser_flags != expected_parser_flags:
        errors.append(
            "exp_runner/runner/policy.py does not account for every known policy flag:\n"
            f"  missing: {sorted(expected_parser_flags - parser_flags)}\n"
            f"  extra:   {sorted(parser_flags - expected_parser_flags)}"
        )


def _validate_shell_matrices(
    errors: list[str],
    repo_root: Path,
    manifest: Mapping[str, Any],
    known_flags: set[str],
    implies: Mapping[str, Sequence[str]],
    exclusive_groups: Mapping[str, Sequence[str]],
) -> None:
    matrices = manifest["matrices"]
    check_combos = _as_str_list(matrices["check"], "matrices.check")
    test_combos = _as_str_list(matrices["test"], "matrices.test")

    _compare_sequence(
        errors,
        "scripts/check.sh flag_combos",
        _extract_shell_array(repo_root / "scripts/check.sh", "flag_combos"),
        check_combos,
    )
    _compare_sequence(
        errors,
        "scripts/test.sh feature_combos",
        _extract_shell_array(repo_root / "scripts/test.sh", "feature_combos"),
        test_combos,
    )
    _validate_combo_list(
        errors, "check matrix", check_combos, known_flags, implies, exclusive_groups
    )
    _validate_combo_list(
        errors, "test matrix", test_combos, known_flags, implies, exclusive_groups
    )


def validate_policy_matrix(repo_root: Path = REPO_ROOT) -> list[str]:
    manifest = _load_toml(repo_root / MATRIX_PATH)
    errors: list[str] = []

    known_flags = set(_as_str_list(manifest["flags"]["known"], "flags.known"))
    implies = _as_str_list_map(manifest.get("implies", {}), "implies")
    exclusive_groups = _as_str_list_map(
        manifest.get("mutually_exclusive", {}), "mutually_exclusive"
    )

    _validate_app_crates(errors, repo_root, manifest)
    _validate_cargo_crates(errors, repo_root, manifest, known_flags)
    _validate_python_policy(errors, repo_root, manifest)
    _validate_shell_matrices(
        errors, repo_root, manifest, known_flags, implies, exclusive_groups
    )

    return errors


def main() -> int:
    errors = validate_policy_matrix()
    if errors:
        print("Policy matrix validation failed:", file=sys.stderr)
        for error in errors:
            print(f"- {error}", file=sys.stderr)
        return 1
    print("Policy matrix validation passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
