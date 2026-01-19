from argparse import Namespace
import argparse
import json
import logging
import os
from pathlib import Path

import pandas as pd

logger = logging.getLogger(__name__)


def _repair_row_parts(parts: list[str], *, expected_fields: int) -> list[str]:
    """
    Best-effort repair for malformed request CSV rows.

    We expect synthetic request CSVs to have:
      api, request_id, slo, start_at, deadline, latency, error, <7 optional numeric latencies>

    Some rows are malformed in two common ways:
    - Missing trailing latency fields (e.g. early-return rows end with a trailing comma).
    - error field contains unescaped commas (e.g. gRPC error strings), which increases the
      observed field count while also omitting the trailing latency fields.

    This function repairs by:
    - Treating the first 6 fields as fixed.
    - Treating the last 7 fields as optional numeric latencies *only if* they are plain integers.
    - Joining any remaining middle fields back into the 'error' column.
    - Padding any missing fields with empty strings.
    """
    if expected_fields <= 0:
        return []

    # Generic fallback for unexpected schemas.
    if expected_fields < 7:
        if len(parts) >= expected_fields:
            return parts[:expected_fields]
        return parts + [""] * (expected_fields - len(parts))

    # Ensure we have at least the fixed prefix.
    fixed = (parts + [""] * 6)[:6]
    remaining = parts[6:]

    # Drop trailing empty tokens caused by a trailing comma.
    while remaining and remaining[-1] == "":
        remaining.pop()

    # Try to peel off trailing numeric latency fields (up to 7).
    n_optional_latencies = expected_fields - 7  # 7 = fixed(6) + error(1)
    latencies_reversed: list[str] = []
    while remaining and len(latencies_reversed) < n_optional_latencies:
        tok = remaining[-1].strip()
        if tok.isdigit():
            latencies_reversed.append(remaining.pop())
        else:
            break
    latencies_reversed.reverse()

    error = ",".join(remaining).strip()
    row = fixed + [error] + latencies_reversed
    if len(row) < expected_fields:
        row += [""] * (expected_fields - len(row))
    elif len(row) > expected_fields:
        row = row[:expected_fields]
    return row


def _read_request_csv(file_path: str) -> pd.DataFrame:
    """
    Read a request CSV, repairing malformed rows when needed.

    We do not rely on pandas' CSV parser here because malformed 'error' fields may contain
    unescaped commas, which breaks tokenization.
    """
    def _needs_repair(parts: list[str], *, expected_fields: int) -> bool:
        # Obvious mismatch.
        if len(parts) != expected_fields:
            return True

        # Subtle mismatch: unescaped commas in the error field can "balance out" missing
        # trailing latency fields, resulting in the *correct* number of tokens while still
        # shifting data into latency columns.
        #
        # For the expected synthetic schema:
        #   fixed(6) + error(1) + optional numeric latencies(N)
        # the optional latency columns should be either empty or plain integers.
        if expected_fields < 7:
            return False

        n_optional_latencies = expected_fields - 7
        if n_optional_latencies <= 0:
            return False

        trailing = parts[-n_optional_latencies:]
        for tok in trailing:
            t = tok.strip()
            if t == "":
                continue
            # Allow negative integers defensively.
            if t.lstrip("-").isdigit():
                continue
            return True
        return False

    with open(file_path, "r", encoding="utf-8", errors="replace") as f:
        header = f.readline()
        if not header:
            return pd.DataFrame()

        columns = header.rstrip("\n").split(",")
        expected = len(columns)
        rows: list[list[str]] = []
        repaired = 0

        for line_no, line in enumerate(f, start=2):
            line = line.rstrip("\n")
            if not line:
                continue

            parts = line.split(",")
            if _needs_repair(parts, expected_fields=expected):
                repaired += 1
                parts = _repair_row_parts(parts, expected_fields=expected)
            rows.append(parts)

    df = pd.DataFrame(rows, columns=columns)

    # Convert any non-string columns to numeric where possible.
    for col in columns:
        if col in ("api", "error"):
            continue
        df[col] = pd.to_numeric(df[col], errors="coerce")

    if repaired:
        logger.warning("Repaired %d malformed row(s) while reading %s", repaired, file_path)

    return df


def read_policies(config_dir: Path) -> list[str]:
    policies_path = Path(config_dir) / "policies"
    if not policies_path.exists():
        raise FileNotFoundError(f"Missing policies file at: {policies_path}")

    policies_text = policies_path.read_text(encoding="utf-8").strip()
    policies = policies_text.split()

    if not policies:
        raise ValueError("policies file is empty or contains no policies")

    return policies


def read_data(config_dir, data_dir):
    with open(os.path.join(config_dir, "gen_config.json")) as f:
        config = json.load(f)
    repeats = config["Repeats"]
    rps_values = config["Rps"]
    apis = config["Apis"]
    slos = config.get("Slos", [])
    
    # Create mapping from API name to SLO value (in microseconds)
    api_to_slo = {}
    if len(slos) == len(apis):
        for api, slo in zip(apis, slos):
            api_to_slo[api] = slo
    else:
        raise ValueError(f"Slos array length ({len(slos)}) must match Apis array length ({len(apis)})")
    
    policies = read_policies(Path(config_dir))
    results = [{} for _ in range(repeats)]
    for i in range(repeats):
        for api in apis + ["ALL"]:
            results[i][api] = {policy: {} for policy in policies}
        for policy in policies:
            policy_folder = os.path.join(data_dir, str(i), policy)
            # process each CSV file
            for rps in rps_values:
                combined = None
                # pyrefly: ignore  # bad-assignment
                for api in apis:
                    file_path = os.path.join(policy_folder, f"r{rps}_{api}.csv")
                    df = _read_request_csv(file_path)
                    # Replace SLO column with value from gen_config.json
                    if api in api_to_slo:
                        df["slo"] = api_to_slo[api]
                    results[i][api][policy][rps] = df
                    if combined is None:
                        combined = df.copy()
                    else:
                        common_cols = combined.columns.intersection(df.columns)
                        combined = pd.concat(
                            [combined[common_cols], df[common_cols]], ignore_index=True
                        )
                # For "ALL" API, keep the individual SLO values from each API type
                # (already set correctly above, so each row has the correct SLO for its API)
                # Don't overwrite with a single value - we want per-API SLO filtering
                results[i]["ALL"][policy][rps] = combined

    apis.append("ALL")

    return repeats, apis, policies, rps_values, results


def prepare_output_dir(args) -> None:
    os.makedirs(args.output_dir, exist_ok=True)

    with open(os.path.join(args.config_dir, "gen_config.json")) as f:
        config = json.load(f)
    repeats = config["Repeats"]

    for i in range(repeats):
        os.makedirs(os.path.join(args.output_dir, str(i)), exist_ok=True)


def parse_args() -> Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--config-dir", type=Path, required=True)
    parser.add_argument("--data-dir", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    args = parser.parse_args()

    return args


def get_policy_color(policy: str) -> str | None:
    """
    Get color for a policy.

    Color scheme:
    - FIFO uses grey hues
    - prio_global uses blue hues
    - prio_local uses pink hues
    - prio_oldest uses purple hues

    Args:
        policy: Policy name

    Returns:
        Color string, or None to use matplotlib default color cycle
    """
    policy_lower = policy.lower()
    if policy_lower.startswith("fifo"):
        if ",early" in policy_lower:
            return "darkgrey"
        return "grey"
    elif policy_lower.startswith("prio_global"):
        if ",early" in policy_lower:
            return "cornflowerblue"
        return "steelblue"
    elif policy_lower.startswith("prio_local"):
        if ",early" in policy_lower:
            return "lightpink"
        return "hotpink"
    elif policy_lower.startswith("prio_oldest"):
        if ",early" in policy_lower:
            return "mediumpurple"
        return "purple"
    return None  # Use matplotlib default color cycle


def get_policy_display_name(policy: str) -> str:
    """
    Return a human-friendly display name for a policy.

    Rules:
    - Drop the trailing ",early" suffix if present.
    - Add "(no-drop)" when the policy does not have the ",early" suffix.
    - Map known base policy names to display names.
    """
    base_policy = policy
    has_early = False
    if base_policy.endswith(",early"):
        base_policy = base_policy[: -len(",early")]
        has_early = True

    base_lower = base_policy.lower()
    display_name_map = {
        "fifo": "FIFO",
        "prio_global": "Masa (global ddl)",
        "prio_local": "Masa (local ddl)",
        "prio_oldest": "Tailclipper",
    }
    display = display_name_map.get(base_lower, base_policy)

    if not has_early:
        display = f"{display} (no-drop)"

    return display


def filter_excluded_errors(df):
    """
    Filter out requests with excluded error types.

    Excludes /ClientTimeout and /EarlyReturn errors as they are not meaningful
    for latency/goodput analysis (timeouts don't represent actual execution,
    and early returns are intentional early exits).

    Args:
        df: DataFrame with an 'error' column

    Returns:
        DataFrame with excluded errors filtered out
    """
    excluded_errors = df["error"].isin(["/ClientTimeout", "/EarlyReturn"])
    return df[~excluded_errors].copy()
