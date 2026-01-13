from __future__ import annotations

import argparse
from pathlib import Path
from typing import Optional, Sequence


def parse_args(argv: Optional[Sequence[str]] = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        prog="microservice-simulator-parser",
        description="Generate simulator artifacts from an Alibaba trace directory.",
    )
    parser.add_argument(
        "-a",
        "--callgraph-dir",
        dest="callgraph_dirs",
        type=Path,
        action="append",
        required=True,
        help="Path to a directory containing call graph inputs (edges.csv, etc.). Can be specified multiple times for multiple call graphs.",
    )
    parser.add_argument(
        "--replay-path",
        dest="replay_path",
        type=Path,
        default=None,
        help="Override the replay trace file used by the load generator.",
    )
    parser.add_argument(
        "--replicas-path",
        dest="replicas_path",
        type=Path,
        default=None,
        help="Optional path to replicas.json configuration file.",
    )
    parser.add_argument(
        "--docker-compose-output-path",
        dest="docker_compose_output_path",
        type=Path,
        default=Path("./docker-compose.yml"),
        help="Where to write the generated docker-compose.yml file.",
    )
    parser.add_argument(
        "--deployment-output-path",
        dest="deployment_output_path",
        type=Path,
        default=Path("./service_configs/deployment.json"),
        help="Where to write the generated deployment JSON file.",
    )

    args = parser.parse_args(argv)
    _validate_args(args)
    return args


def _validate_args(args: argparse.Namespace) -> None:
    for callgraph_dir in args.callgraph_dirs:
        if not callgraph_dir.exists():
            raise SystemExit(f"Callgraph directory does not exist: {callgraph_dir}")
        if not callgraph_dir.is_dir():
            raise SystemExit(f"Callgraph path is not a directory: {callgraph_dir}")
    if args.replicas_path is not None and not args.replicas_path.exists():
        raise SystemExit(f"Replicas file does not exist: {args.replicas_path}")

