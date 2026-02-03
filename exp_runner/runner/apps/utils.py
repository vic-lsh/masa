"""
Shared utilities for application plugins.
"""

import os
import re
import sys
import logging
import csv
from typing import Optional, TYPE_CHECKING

if TYPE_CHECKING:
    from exp.runner.config import ExperimentConfig

logger = logging.getLogger(__name__)

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
                for api in apis:
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
                    expected_per_api = expected_total / num_apis
                    
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