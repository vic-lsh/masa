import argparse
import json
import logging
import os
from argparse import Namespace
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
        logger.warning(
            "Repaired %d malformed row(s) while reading %s", repaired, file_path
        )

    # Centralized error parsing
    df = _parse_error_columns(df)

    return df


def _parse_error_columns(df: pd.DataFrame) -> pd.DataFrame:
    """
    Centralized parsing logic.
    Extracts metadata from error strings into dedicated columns.
    """
    # 1. Initialize default columns
    # Use object dtype initially to allow strings and NaNs
    df["error_type"] = "Generic"
    df["er_service"] = None
    df["er_method"] = None
    df["er_last_child"] = None

    if "error" not in df.columns:
        return df

    # 2. Identify Early Returns
    # Handle NaN/None in error column safely
    # We cast to string just in case, though usually it should be string or NaN
    mask_er = df["error"].astype(str).str.startswith("/EarlyReturn")
    if not mask_er.any():
        return df

    df.loc[mask_er, "error_type"] = "EarlyReturn"

    # 3. Vectorized extraction using Regex
    # Regex captures: /EarlyReturn?src=<svc>::<method>?last_rpc=<last_child>
    # Pattern explanation:
    # ^/EarlyReturn\?src=         Start with literal prefix
    # (?P<er_service>.+?)         Capture service (non-greedy)
    # ::                          Literal separator
    # (?P<er_method>[^?]+)        Capture method (until next ? or end)
    # (?:\?last_rpc=(?P<er_last_child>.*))?  Optional group: ?last_rpc= followed by anything
    pattern = r"^/EarlyReturn\?src=(?P<er_service>.+?)::(?P<er_method>[^?]+)(?:\?last_rpc=(?P<er_last_child>.*))?$"

    extracted_data = df.loc[mask_er, "error"].str.extract(pattern)

    # 4. Merge back into main dataframe
    if not extracted_data.empty:
        df.update(extracted_data)

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
        raise ValueError(
            f"Slos array length ({len(slos)}) must match Apis array length ({len(apis)})"
        )

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
    features = [feature.strip().lower() for feature in policy.split(",") if feature.strip()]
    has_early = "early" in features
    has_local_transform = "prio_local_transform" in features

    if policy_lower.startswith("fifo"):
        if has_early:
            return "darkgrey"
        return "grey"
    elif policy_lower.startswith("prio_global"):
        if has_early:
            return "cornflowerblue"
        return "steelblue"
    elif policy_lower.startswith("prio_local"):
        if has_local_transform:
            if has_early:
                return "lightsalmon"
            return "orangered"
        if has_early:
            return "lightpink"
        return "hotpink"
    elif policy_lower.startswith("prio_oldest"):
        if has_early:
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
    features = [feature.strip() for feature in policy.split(",") if feature.strip()]
    has_early = "early" in features
    has_local_transform = "prio_local_transform" in features

    base_policy = policy
    for known_base in ("fifo", "prio_global", "prio_local", "prio_oldest"):
        if known_base in features:
            base_policy = known_base
            break
    else:
        non_early_features = [feature for feature in features if feature != "early"]
        if non_early_features:
            base_policy = ",".join(non_early_features)

    base_lower = base_policy.lower()
    display_name_map = {
        "fifo": "FIFO",
        "prio_global": "Masa (global ddl)",
        "prio_local": "Masa (local ddl)",
        "prio_oldest": "Tailclipper",
    }
    display = display_name_map.get(base_lower, base_policy)

    if base_lower == "prio_local" and has_local_transform:
        display = "Masa (local ddl, transformed)"

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
    if df.empty:
        return df

    # Fast path: use parsed error_type if available
    if "error_type" in df.columns:
        # Exclude EarlyReturn
        # Also check for ClientTimeout (which is usually just "Generic" type currently,
        # unless we add it to parser, but let's check string for Timeout for now or add it)
        # Actually, let's just use string check for ClientTimeout and column for EarlyReturn
        is_early_return = df["error_type"] == "EarlyReturn"
        is_timeout = df["error"] == "/ClientTimeout"
        return df[~(is_early_return | is_timeout)].copy()

    # Use apply instead of str.startswith to avoid potential numpy.rec issues
    # in some pandas/numpy version combinations (specifically pandas < 2.2 with numpy 2.0+).
    def is_excluded(err):
        # Treat non-string errors (NaN, None, numbers) as NOT excluded
        if not isinstance(err, str):
            return False
        return err == "/ClientTimeout" or err.startswith("/EarlyReturn")

    excluded_errors = df["error"].apply(is_excluded)
    return df[~excluded_errors].copy()
