#!/usr/bin/env python3

"""Find callees shared across call graph reports."""

from __future__ import annotations

import argparse
import csv
from collections import defaultdict
from pathlib import Path
from typing import DefaultDict, Dict, Iterable, Set, Tuple

GRAPH_REPORTS_DIR = Path(__file__).parent / "graph_reports"


def iter_callees(graph_dir: Path) -> Iterable[Tuple[str, str]]:
    """Yield (service, callee) pairs for every MS_* callee in the graph reports."""
    edge_paths = sorted(graph_dir.glob("S_*/edges.csv"))
    for edge_path in edge_paths:
        service_name = edge_path.parent.name
        with edge_path.open(newline="") as csv_file:
            reader = csv.DictReader(csv_file)
            callee_field_missing = "callee" not in reader.fieldnames if reader.fieldnames else True
            if callee_field_missing:
                continue
            for row in reader:
                callee = row.get("callee", "")
                if callee.startswith("MS_"):
                    yield service_name, callee


def find_shared_callees(graph_dir: Path) -> Dict[str, Set[str]]:
    """Return mapping of callee -> services where the callee appears in more than one report."""
    callee_to_services: DefaultDict[str, Set[str]] = defaultdict(set)
    for service_name, callee in iter_callees(graph_dir):
        callee_to_services[callee].add(service_name)
    return {callee: services for callee, services in callee_to_services.items() if len(services) > 1}


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Find callee names (MS_*) shared across call graph reports."
    )
    parser.add_argument(
        "--graph-dir",
        type=Path,
        default=GRAPH_REPORTS_DIR,
        help="Directory containing graph report folders (default: trace-analysis/graph_reports).",
    )
    args = parser.parse_args()

    shared = find_shared_callees(args.graph_dir)
    if not shared:
        print("No shared callees found.")
        return

    for callee in sorted(shared):
        services = ", ".join(sorted(shared[callee]))
        print(f"{callee}: {services}")


if __name__ == "__main__":
    main()

