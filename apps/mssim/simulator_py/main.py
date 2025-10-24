from __future__ import annotations

import sys
from pathlib import Path
from typing import Optional, Sequence

from . import cli
from .orchestrator import launch_simulation_from_trace
from .simulator_config import SimulatorConfig, SimulatorConfigError
from .trace_config import TraceConfig, TraceConfigError
from .validator import ValidationError, validate_config


def run_from_alibaba_trace(
    trace_dir: Path,
    replay_path: Optional[Path],
    config_dir: Optional[Path],
    docker_compose_output_path: Path,
    deployment_output_path: Path,
) -> None:
    trace_config = TraceConfig.from_config_dir(trace_dir)
    sim_config = SimulatorConfig.from_config_dir(config_dir)

    validate_config(trace_config)

    launch_simulation_from_trace(
        trace_config,
        trace_dir,
        sim_config,
        docker_compose_output_path,
        deployment_output_path,
    )


def main(argv: Optional[Sequence[str]] = None) -> int:
    args = cli.parse_args(argv)

    try:
        run_from_alibaba_trace(
            args.alibaba_trace,
            args.replay_path,
            args.config_dir,
            args.docker_compose_output_path,
            args.deployment_output_path,
        )
    except (TraceConfigError, SimulatorConfigError, ValidationError, RuntimeError) as exc:
        print(f"Error: {exc}", file=sys.stderr)
        return 1

    return 0


if __name__ == "__main__":
    sys.exit(main())

