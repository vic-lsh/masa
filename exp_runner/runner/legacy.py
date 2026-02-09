"""
Configuration converters between legacy and v2 experiment formats.
"""

import json
import logging
from pathlib import Path
from typing import Any, Optional

from .experiment_config_v2 import (
    ApiSpec,
    ExperimentConfigV2,
    ExecutionSpec,
    LoadGenSpec,
)
from .topology import TopologySpec

logger = logging.getLogger(__name__)


def load_legacy_gen_config(path: Path) -> dict[str, Any]:
    """Load legacy gen_config.json file."""
    with open(path) as f:
        return json.load(f)


def convert_legacy_to_experiment_config(
    gen_config: dict[str, Any],
    policies: list[str],
    experiment_name: str,
    app_name: str,
) -> ExperimentConfigV2:
    """
    Convert legacy gen_config.json + policies to ExperimentConfigV2.

    Args:
        gen_config: Loaded gen_config.json data
        policies: List of policies from policies file
        experiment_name: Experiment name
        app_name: Application name

    Returns:
        ExperimentConfigV2 with equivalent configuration

    Supports both historical key styles:
    - Repeats/RPSList/Warmup/Duration/APIs (list of dicts)
    - Repeats/Rps/WarmupSecs/DurationSecs/Apis+Slos+Timeouts_ms
    """
    # Parse execution parameters
    repeats = gen_config.get("Repeats", 1)
    warmup_secs = int(gen_config.get("WarmupSecs", gen_config.get("Warmup", 10)))
    duration_secs = int(gen_config.get("DurationSecs", gen_config.get("Duration", 60)))

    execution = ExecutionSpec(
        repeats=repeats,
        policies=policies,
        warmup_secs=warmup_secs,
        duration_secs=duration_secs,
    )

    # Parse load generation parameters
    rps = gen_config.get("Rps", gen_config.get("RPSList", []))

    # Find default timeout (use first API's timeout, or 1000 if not specified)
    default_timeout_ms = 1000
    apis_data = gen_config.get("APIs")
    apis = []
    if isinstance(apis_data, list) and apis_data:
        if isinstance(apis_data[0], dict):
            if "Timeout" in apis_data[0]:
                default_timeout_ms = int(apis_data[0]["Timeout"])
            for api_data in apis_data:
                timeout_ms = api_data.get("Timeout")
                if timeout_ms == default_timeout_ms:
                    timeout_ms = None
                apis.append(
                    ApiSpec(
                        name=api_data.get("Name", ""),
                        slo_us=int(api_data.get("SLO", 0)),
                        weight=float(api_data.get("ReqWeight", 1.0)),
                        timeout_ms=int(timeout_ms) if timeout_ms is not None else None,
                    )
                )
    else:
        names = gen_config.get("Apis", [])
        slos = gen_config.get("Slos", [])
        timeouts = gen_config.get("Timeouts_ms", [])
        if timeouts:
            default_timeout_ms = int(timeouts[0])

        for idx, api_name in enumerate(names):
            slo_us = int(slos[idx]) if idx < len(slos) else 0
            timeout_ms: Optional[int] = None
            if idx < len(timeouts):
                timeout = int(timeouts[idx])
                if timeout != default_timeout_ms:
                    timeout_ms = timeout
            apis.append(
                ApiSpec(
                    name=str(api_name),
                    slo_us=slo_us,
                    weight=1.0,
                    timeout_ms=timeout_ms,
                )
            )

    loadgen = LoadGenSpec(
        rps=rps,
        default_timeout_ms=default_timeout_ms,
        apis=apis,
    )

    return ExperimentConfigV2(
        kind="Experiment",
        name=experiment_name,
        app=app_name,
        execution=execution,
        loadgen=loadgen,
        metadata={
            "source": "legacy_gen_config",
        },
    )


def convert_experiment_config_to_gen_config(
    exp_config: ExperimentConfigV2,
    frontend_addr: str = "frontend:8080",
) -> dict[str, Any]:
    """
    Convert ExperimentConfigV2 to runner-compatible gen_config.json format.

    Useful for gradual migration - allows new config to drive old code paths.

    Args:
        exp_config: New experiment configuration
        frontend_addr: Frontend address (required by legacy format)

    Returns:
        Dictionary in gen_config.json format
    """
    apis_data = []
    for api in exp_config.loadgen.apis:
        api_dict = {
            "Name": api.name,
            "ReqWeight": api.weight,
            "SLO": api.slo_us,
            "Timeout": api.timeout_ms or exp_config.loadgen.default_timeout_ms,
        }
        apis_data.append(api_dict)

    return {
        "Repeats": exp_config.execution.repeats,
        "Rps": exp_config.loadgen.rps,
        "Addr": frontend_addr,
        "WarmupSecs": exp_config.execution.warmup_secs,
        "DurationSecs": exp_config.execution.duration_secs,
        "Apis": [api["Name"] for api in apis_data],
        "Slos": [api["SLO"] for api in apis_data],
        "Timeouts_ms": [api["Timeout"] for api in apis_data],
        "Gap": "const",
    }


def infer_topology_from_gen_config(
    gen_config: dict[str, Any],
    app_name: str,
) -> Optional[TopologySpec]:
    """
    Attempt to infer topology from legacy gen_config.json.

    This is limited - legacy format doesn't include topology info.
    Returns None for most cases, letting resolver use default topology.

    Args:
        gen_config: Loaded gen_config.json data
        app_name: Application name

    Returns:
        TopologySpec if topology can be inferred, None otherwise
    """
    # Legacy format doesn't include topology info
    # Return None to use default topology for the app
    return None
