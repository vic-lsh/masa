import argparse
import json
import logging
import os
from dataclasses import dataclass
from argparse import Namespace
from pathlib import Path

import pandas as pd

logger = logging.getLogger(__name__)


@dataclass
class PlotData:
    repeats: int
    apis: list[str]
    policies: list[str]
    rps_values: list[int]
    rps_sequence: list[int]  # Original order from gen_config["Rps"]
    duration_sec: float
    warmup_sec: float
    results: list[dict]


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

    # Strip surrounding double quotes from the error column.
    # The Rust request logger quotes error strings that contain commas
    # (e.g. ",reason=LocalDeadlineExceeded"), which the custom CSV
    # parser preserves verbatim.
    if "error" in df.columns:
        df["error"] = df["error"].str.strip('"')

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
    df["er_reason"] = None

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
    # Regex captures: /EarlyReturn?src=<svc>::<method>[?last_rpc=<child>][&reason=<reason>]
    # Pattern explanation:
    # ^/EarlyReturn\?src=         Start with literal prefix
    # (?P<er_service>.+?)         Capture service (non-greedy)
    # ::                          Literal separator
    # (?P<er_method>[^?&,\s]+)    Capture method (until next delimiter or end)
    # (?:[?,]last_rpc=(?P<er_last_child>[^&,\s]*))?  Optional: last_rpc
    # (?:[&?,]reason=(?P<er_reason>[^\s]*))?          Optional: reason (& or ? or , separator)
    pattern = r"^/EarlyReturn\?src=(?P<er_service>.+?)::(?P<er_method>[^?&,\s]+)(?:[?,]last_rpc=(?P<er_last_child>[^&,\s]*))?(?:[&?,]reason=(?P<er_reason>[^\s]*))?"

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


def load_plot_data(config_dir: Path | str, data_dir: Path | str) -> PlotData:
    with open(os.path.join(config_dir, "gen_config.json")) as f:
        config = json.load(f)
    repeats = config["Repeats"]
    rps_values = config["Rps"]
    rps_sequence = list(rps_values)  # Preserve original order before any dedup
    duration_sec = float(config.get("DurationSecs", 60))
    warmup_sec = float(config.get("WarmupSecs", 0))
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

    return PlotData(
        repeats=repeats,
        apis=apis,
        policies=policies,
        rps_values=rps_values,
        rps_sequence=rps_sequence,
        duration_sec=duration_sec,
        warmup_sec=warmup_sec,
        results=results,
    )


def read_data(config_dir, data_dir):
    plot_data = load_plot_data(config_dir, data_dir)
    return (
        plot_data.repeats,
        plot_data.apis,
        plot_data.policies,
        plot_data.rps_values,
        plot_data.results,
    )


def prepare_output_dir(args) -> None:
    os.makedirs(args.output_dir, exist_ok=True)

    with open(os.path.join(args.config_dir, "gen_config.json")) as f:
        config = json.load(f)
    repeats = config["Repeats"]

    for i in range(repeats):
        os.makedirs(os.path.join(args.output_dir, str(i)), exist_ok=True)


def get_plot_worker_count(task_count: int, *, max_workers: int = 8) -> int:
    if task_count <= 0:
        return 1

    return max(1, min(task_count, max_workers))


def parse_args() -> Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--config-dir", type=Path, required=True)
    parser.add_argument("--data-dir", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    args = parser.parse_args()

    return args


def apply_plot_defaults() -> None:
    """Apply shared matplotlib rcParams defaults for all Masa plots."""
    import matplotlib.pyplot as plt

    plt.rcParams["legend.fontsize"] = 13
    plt.rcParams["legend.title_fontsize"] = 13


def get_policy_color(policy: str) -> str | None:
    """Get matplotlib color for a policy, or None for the default cycle."""
    from exp_runner.runner.policy import Policy

    return Policy.parse(policy).color


def get_policy_display_name(policy: str) -> str:
    """Return a human-friendly key=value display name for a policy."""
    from exp_runner.runner.policy import Policy

    return Policy.parse(policy).display_name


def get_policy_marker(policy: str) -> str:
    """Get matplotlib marker shape for a policy."""
    from exp_runner.runner.policy import Policy

    return Policy.parse(policy).marker


def get_policy_linestyle(policy: str) -> str:
    """Get matplotlib linestyle for a policy."""
    from exp_runner.runner.policy import Policy

    return Policy.parse(policy).linestyle


def get_policy_hatch(policy: str) -> str:
    """Get matplotlib bar hatch pattern for a policy."""
    from exp_runner.runner.policy import Policy

    return Policy.parse(policy).hatch


def get_policy_line_style(policy: str) -> dict:
    """Bundle of {color, marker, linestyle} suitable for ax.plot(**...).

    Provides redundant encoding (color + shape + line style) so plots stay
    readable in greyscale and for color-vision-deficient viewers.
    """
    from exp_runner.runner.policy import Policy

    p = Policy.parse(policy)
    return {"color": p.color, "marker": p.marker, "linestyle": p.linestyle}


def get_policy_bar_style(policy: str) -> dict:
    """Bundle of {color, hatch, edgecolor} suitable for ax.bar(**...).

    `edgecolor="black"` is required so the hatch pattern renders visibly.
    """
    from exp_runner.runner.policy import Policy

    p = Policy.parse(policy)
    return {"color": p.color, "hatch": p.hatch, "edgecolor": "black"}


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

    if "error" not in df.columns:
        return df

    # Fast path: use parsed error_type if available
    if "error_type" in df.columns:
        # Exclude EarlyReturn
        # Also check for ClientTimeout (which is usually just "Generic" type currently,
        # unless we add it to parser, but let's check string for Timeout for now or add it)
        # Actually, let's just use string check for ClientTimeout and column for EarlyReturn
        is_early_return = df["error_type"] == "EarlyReturn"
        is_timeout = (
            (df["error"] == "/ClientTimeout")
            if "error" in df.columns
            else pd.Series(False, index=df.index)
        )
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
