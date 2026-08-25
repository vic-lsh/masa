#!/usr/bin/env python3
"""Measure long-before-short ordering within matched-deadline bursts."""

from __future__ import annotations

import argparse
import csv
import json
import re
import sys
from collections import defaultdict
from pathlib import Path


POLICIES = (
    "sched_slo",
    "sched_pred,est_mean_var",
    "eval_oracle_continuation",
)
RESULT_FILE = re.compile(r"r(?P<rps>\d+)_(?P<api>[ab])\.csv")


def shared_hop(row: dict[str, str]) -> dict[str, object]:
    hops = json.loads(row["hop_trace_json"])
    return next(hop for hop in hops if hop["service_id"] == "shared")


def measure(policy_dir: Path, rps: int) -> dict[str, float | int]:
    with (policy_dir / "gen_config.json").open() as config_file:
        burst_size = int(json.load(config_file).get("MatchedDeadlineBurstSize", 1))

    groups: dict[int, list[tuple[str, int, int]]] = defaultdict(list)
    for api in ("a", "b"):
        with (policy_dir / f"r{rps}_{api}.csv").open(newline="") as result_file:
            for row in csv.DictReader(result_file):
                hop = shared_hop(row)
                groups[int(row["deadline"])].append(
                    (api, int(hop["started_at"]), int(hop["finished_at"]))
                )

    start_wins = 0
    finish_wins = 0
    pairs = 0
    for group in groups.values():
        short = [request for request in group if request[0] == "a"]
        long = [request for request in group if request[0] == "b"]
        for short_request in short:
            for long_request in long:
                start_wins += long_request[1] < short_request[1]
                finish_wins += long_request[2] < short_request[2]
                pairs += 1

    if pairs == 0:
        raise ValueError(f"no mixed a/b bursts in {policy_dir} at {rps} RPS")

    mixed_bursts = sum(
        {request[0] for request in group} == {"a", "b"}
        for group in groups.values()
    )
    full_bursts = sum(len(group) == burst_size for group in groups.values())
    return {
        "pair_count": pairs,
        "long_before_short_start_fraction": start_wins / pairs,
        "long_before_short_finish_fraction": finish_wins / pairs,
        "mixed_burst_fraction": mixed_bursts / len(groups),
        "full_burst_fraction": full_bursts / len(groups),
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "output",
        type=Path,
        nargs="?",
        default=Path("exp/synthbench/out/oracle_continuation"),
    )
    args = parser.parse_args()

    writer = csv.DictWriter(
        sys.stdout,
        fieldnames=(
            "iteration",
            "rps",
            "policy",
            "pair_count",
            "long_before_short_start_fraction",
            "long_before_short_finish_fraction",
            "mixed_burst_fraction",
            "full_burst_fraction",
        ),
    )
    writer.writeheader()
    for iteration_dir in sorted(
        (path for path in args.output.iterdir() if path.name.isdigit()),
        key=lambda path: int(path.name),
    ):
        for policy in POLICIES:
            policy_dir = iteration_dir / policy
            rps_values = sorted(
                {
                    int(match.group("rps"))
                    for path in policy_dir.glob("r*_*.csv")
                    if (match := RESULT_FILE.fullmatch(path.name))
                }
            )
            for rps in rps_values:
                writer.writerow(
                    {
                        "iteration": int(iteration_dir.name),
                        "rps": rps,
                        "policy": policy,
                        **measure(policy_dir, rps),
                    }
                )


if __name__ == "__main__":
    main()
