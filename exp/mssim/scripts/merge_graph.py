#!/usr/bin/env python3

from __future__ import annotations

import argparse
import csv
import json
from collections import defaultdict
from pathlib import Path
from typing import Dict, Iterable, Tuple


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Merge multiple call graph directories.")
    parser.add_argument(
        "-g",
        "--graph-dirs",
        nargs="+",
        type=Path,
        required=True,
        help="Input call graph directories (each containing edges.csv, etc.).",
    )
    parser.add_argument(
        "-o",
        "--output-dir",
        type=Path,
        required=True,
        help="Directory where the merged graph should be written.",
    )
    parser.add_argument(
        "--output-service",
        default="combined",
        help="Name to use in the 'service' column of the merged edges.csv.",
    )
    return parser.parse_args()





def load_edges(graph_dir: Path, user_alias: str) -> Iterable[Tuple[str, str, float]]:
    def _parse_weight(raw_value: str | None) -> float:
    if raw_value is None or raw_value == "":
        return 1.0
    try:
        return float(raw_value)
    except ValueError as exc:
        raise ValueError(f"Invalid weight value: {raw_value!r}") from exc
    
    edges_path = graph_dir / "edges.csv"
    if not edges_path.exists():
        raise FileNotFoundError(f"Missing edges.csv in {graph_dir}")

    with edges_path.open(newline="") as handle:
        reader = csv.DictReader(handle)
        expected = {"service", "caller", "callee", "weight"}
        if reader.fieldnames is None or not expected.issubset(set(reader.fieldnames)):
            raise ValueError(f"edges.csv in {graph_dir} is missing expected columns")
        for row in reader:
            caller = row["caller"].strip()
            callee = row["callee"].strip()
            if not caller or not callee:
                continue
            if caller == "USER":
                caller = user_alias
            if callee == "USER":
                callee = user_alias
            weight = _parse_weight(row.get("weight"))
            yield caller, callee, weight


def merge_edges(graph_dirs: list[Path]) -> Tuple[Dict[Tuple[str, str], float], Dict[Path, str]]:
    merged: Dict[Tuple[str, str], float] = defaultdict(float)
    alias_map: Dict[Path, str] = {}
    for idx, graph_dir in enumerate(graph_dirs, start=1):
        user_alias = f"USER{idx}"
        alias_map[graph_dir] = user_alias
        for caller, callee, weight in load_edges(graph_dir, user_alias):
            merged[(caller, callee)] += weight
    return merged, alias_map


def collect_json(graph_dirs: list[Path], filename: str) -> Dict[str, dict] | None:
    collected: Dict[str, dict] = {}
    any_data = False
    for graph_dir in graph_dirs:
        graph_name = graph_dir.name
        path = graph_dir / filename
        if path.exists():
            collected[graph_name] = json.loads(path.read_text())
            any_data = True
        else:
            collected[graph_name] = {}
    return collected if any_data else None


def write_edges(
    output_dir: Path,
    edges: Dict[Tuple[str, str], float],
    service_name: str,
) -> None:
    edges_path = output_dir / "edges.csv"
    with edges_path.open("w", newline="") as handle:
        writer = csv.writer(handle)
        writer.writerow(["service", "caller", "callee", "weight"])
        for (caller, callee), weight in sorted(edges.items()):
            writer.writerow([service_name, caller, callee, weight])


def write_json(output_dir: Path, filename: str, data: Dict[str, dict]) -> None:
    path = output_dir / filename
    path.write_text(json.dumps(data, indent=2, sort_keys=True))


def main() -> None:
    args = parse_args()
    graph_dirs = [path.expanduser().resolve() for path in args.graph_dirs]
    for path in graph_dirs:
        if not path.exists():
            raise SystemExit(f"Input graph directory does not exist: {path}")
        if not path.is_dir():
            raise SystemExit(f"Input graph path is not a directory: {path}")

    output_dir = args.output_dir.expanduser().resolve()
    output_dir.mkdir(parents=True, exist_ok=True)

    # Merge edges
    merged_edges, alias_map = merge_edges(graph_dirs)
    write_edges(output_dir, merged_edges, args.output_service)

    # Merge interface
    interface_data = collect_json(graph_dirs, "interface_distribution.json")
    if interface_data is not None:
        write_json(output_dir, "interface_distribution.json", interface_data)

    # Merge latencies
    latency_data = collect_json(graph_dirs, "latency_percentiles.json")
    if latency_data is not None:
        write_json(output_dir, "latency_percentiles.json", latency_data)


if __name__ == "__main__":
    main()
