"""
Shared utilities for application plugins.
"""

import copy
import os
import re
import sys
import logging
import csv
from typing import Optional, TYPE_CHECKING

if TYPE_CHECKING:
    from exp.runner.config import ExperimentConfig

logger = logging.getLogger(__name__)

# ── Scheduling-policy → parameter key mapping ──────────────────────────
#
# Resolves per-scheduling-policy parameter variants from a single
# policy_param.json.  A section like "rajomon" may contain nested
# scheduling-policy keys:
#
#   { "rajomon": {
#       "sched_fifo":  { "price_cap": 10, ... },
#       "sched_slo":   { "init_price": 3, ... }
#   }}
#
# The runner detects the active scheduling policy from the feature string,
# picks the matching sub-dict, and flattens it so the container sees:
#
#   { "rajomon": { "price_cap": 10, ... } }
#
# If "rajomon" contains flat params (no nested policy keys), it passes
# through unchanged.

SCHED_POLICY_KEYS = {"sched_pred", "sched_slo", "sched_fifo", "sched_tailclipper"}


def resolve_policy_params(params: dict, policy: str) -> dict:
    """Resolve per-scheduling-policy parameter variants.

    For each known section (``rajomon``, ``pred``), if the value is a dict
    whose keys are scheduling policy names, select the sub-dict matching the
    active scheduling policy.  Otherwise pass through unchanged.
    """
    features = set(policy.split(","))

    # Determine which scheduling policy is active.
    active_sched = None
    for sched in SCHED_POLICY_KEYS:
        if sched in features:
            active_sched = sched
            break

    resolved = copy.deepcopy(params)

    for section in ("rajomon", "pred"):
        value = resolved.get(section)
        if not isinstance(value, dict):
            continue
        # Check if value contains scheduling policy keys (nested variant).
        if value.keys() & SCHED_POLICY_KEYS:
            if active_sched and active_sched in value:
                resolved[section] = value[active_sched]
                logger.info(
                    f"Using {section}.{active_sched} params for policy {policy}"
                )
            else:
                # No match — remove section so Rust uses built-in defaults.
                del resolved[section]
                logger.info(f"No {section} params for {active_sched}, using defaults")

    return resolved


def normalize_features_to_tag(features: Optional[str]) -> str:
    """
    Normalize cargo feature flags into a deterministic, valid docker tag.

    Cargo features are comma-separated (e.g., "feat-b,feat-a").
    Docker tags must be lowercase alphanumeric with periods, dashes, or underscores.

    Args:
        features: Comma-separated cargo features or None

    Returns:
        A normalized docker tag string (e.g., "feat-a-feat-b" or "latest")
    """
    if not features or features.strip() == "":
        return "latest"

    # Split by comma, strip whitespace, and sort for determinism
    feature_list = [f.strip() for f in features.split(",")]
    feature_list = [f for f in feature_list if f]  # Remove empty strings

    if not feature_list:
        return "latest"

    # Sort for determinism (case-insensitive for consistency)
    feature_list.sort(key=str.lower)

    # Join with dashes, ensuring valid docker tag characters
    # Replace any invalid characters with dashes
    tag = "-".join(feature_list)

    # Docker tags: lowercase alphanumeric, periods, dashes, underscores only
    # Also ensure it doesn't start with a period or dash
    tag = re.sub(r"[^a-zA-Z0-9._-]", "-", tag)
    tag = tag.lower()
    tag = re.sub(r"^[.-]+", "", tag)  # Remove leading periods or dashes
    tag = re.sub(r"-+", "-", tag)  # Collapse multiple dashes

    return tag if tag else "latest"


def get_docker_progress_flag() -> str:
    """
    Get the appropriate docker build progress flag based on environment.

    In CI environments (detected via CI environment variable), use 'plain'
    progress mode since TTY is not available. Otherwise, use 'tty' for
    better interactive output.

    Returns:
        Progress flag string: '--progress=plain' in CI, '--progress=tty' otherwise
    """
    # Check for CI environment variable (set by most CI systems including GitLab CI)
    if os.environ.get("CI", "").lower() in ("true", "1", "yes"):
        return "--progress=plain"
    # If no TTY is attached (e.g., non-interactive runner), use plain output.
    if not sys.stdout.isatty():
        return "--progress=plain"
    return "--progress=tty"


def _api_weight_fractions(gen_config: dict, num_apis: int) -> Optional[list[float]]:
    weights = gen_config.get("ApiWeights")
    if weights is None:
        return [1.0 / num_apis for _ in range(num_apis)]

    if len(weights) != num_apis:
        logger.error("ApiWeights length must match Apis length")
        return None

    try:
        weights = [float(weight) for weight in weights]
    except (TypeError, ValueError):
        logger.error("ApiWeights entries must be numeric")
        return None

    if any(weight < 0 for weight in weights):
        logger.error("ApiWeights entries must be non-negative")
        return None

    total_weight = sum(weights)
    if total_weight <= 0:
        logger.error("ApiWeights must contain at least one positive value")
        return None

    return [weight / total_weight for weight in weights]


def verify_standard_workload(config: "ExperimentConfig") -> bool:
    """
    Shared verification logic for standard workloads (hotel, socialnet, synthetic).

    Checks:
    - Existence of done marker
    - Existence of policy directories
    - Existence of loadgen logs
    - Existence of CSV trace files per API/RPS
    - Goodput within 20% margin of target RPS

    Args:
        config: Experiment configuration object

    Returns:
        True if all checks pass, False otherwise
    """
    logger.info(f"Verifying {config.app_name} experiment: {config.experiment_name}")

    gen_config = config.gen_config
    try:
        rps_list = gen_config["Rps"]
        apis = gen_config["Apis"]
        repeats = gen_config.get("Repeats", 1)
        duration = gen_config["DurationSecs"]
    except KeyError as e:
        logger.error(f"Missing key in gen_config.json: {e}")
        return False

    num_apis = len(apis)
    if num_apis == 0:
        logger.error("No APIs defined in gen_config.json")
        return False
    api_weight_fractions = _api_weight_fractions(gen_config, num_apis)
    if api_weight_fractions is None:
        return False

    policies = config.policies

    # Check done marker
    done_file = config.out_dir / "done"
    if not done_file.exists():
        logger.error(f"Experiment not marked as complete: {done_file} missing")
        return False

    all_passed = True

    for i in range(repeats):
        for policy in policies:
            policy_dir = config.out_dir / str(i) / policy
            if not policy_dir.exists():
                logger.error(f"Policy output directory missing: {policy_dir}")
                all_passed = False
                continue

            # Check loadgen log
            if not (policy_dir / "loadgen.log").exists():
                logger.error(f"Load generator log missing in {policy_dir}")
                all_passed = False

            for rps in rps_list:
                for api, api_fraction in zip(apis, api_weight_fractions):
                    expected_file = policy_dir / f"r{rps}_{api}.csv"
                    if not expected_file.exists():
                        logger.error(f"Expected output file missing: {expected_file}")
                        all_passed = False
                        continue

                    # Calculate goodput
                    goodput = 0
                    try:
                        with open(expected_file, "r") as f:
                            reader = csv.reader(f)
                            # Assuming column 7 (index 6) is the error column
                            for row in reader:
                                if len(row) > 6 and row[6].strip() == "/None":
                                    goodput += 1
                    except Exception as e:
                        logger.error(f"Failed to read CSV {expected_file}: {e}")
                        all_passed = False
                        continue

                    expected_total = rps * duration
                    expected_per_api = expected_total * api_fraction

                    lower = expected_per_api * 0.8
                    upper = expected_per_api * 1.2

                    if not (lower <= goodput <= upper):
                        observed_rps = goodput / duration
                        expected_rps_per_api = expected_per_api / duration
                        logger.error(
                            f"Goodput mismatch in {expected_file.name} (Policy: {policy}, Iteration: {i})\n"
                            f"  Expected: ~{expected_per_api:.0f} (+/- 20%)\n"
                            f"  Got: {goodput}\n"
                            f"  Observed RPS: {observed_rps:.2f} (Target: {expected_rps_per_api:.2f})"
                        )
                        all_passed = False

    return all_passed
