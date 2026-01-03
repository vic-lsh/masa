from __future__ import annotations

import sys
from pathlib import Path
from typing import Optional, Sequence

from .cli import parse_args
from .orchestrator import launch_simulation_from_trace
from .simulator_config import SimulatorConfig, SimulatorConfigError
from .trace_config import TraceConfig, TraceConfigError
from .validator import ValidationError, validate_config


def run_from_alibaba_trace(
    callgraph_dirs: list[Path],
    replay_path: Optional[Path],
    config_dir: Optional[Path],
    docker_compose_output_path: Path,
    deployment_output_path: Path,
) -> None:
    # Load trace configs for all callgraph directories
    trace_configs = [TraceConfig.from_config_dir(d) for d in callgraph_dirs]
    
    # Validate all configs
    for trace_config in trace_configs:
        validate_config(trace_config)
    
    sim_config = SimulatorConfig.from_config_dir(config_dir)

    launch_simulation_from_trace(
        trace_configs,
        callgraph_dirs,
        sim_config,
        docker_compose_output_path,
        deployment_output_path,
    )


def main(argv: Optional[Sequence[str]] = None) -> int:
    args = parse_args(argv)

    try:
        run_from_alibaba_trace(
            args.callgraph_dirs,
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

