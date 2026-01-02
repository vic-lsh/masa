#!/usr/bin/env python3

"""
DEPRECATED: MSSIM experiments now run via `exp.runner`.

This repository consolidated experiment-running infrastructure. Use:

  python -m exp.runner run mssim <experiment_name>

MSSIM experiments are configured under:

  exp/mssim/data/in/<experiment_name>/
    - gen_config.json
    - policies
    - mssim.json
"""

from __future__ import annotations

import argparse
import sys


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(add_help=True)
    parser.add_argument(
        "--config",
        help="(deprecated) Path to the old MSSIM JSON plan. Not supported anymore.",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="(deprecated) Use `python -m exp.runner run mssim <exp> --dry-run` instead.",
    )
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    _ = parse_args(argv)
    msg = "\n".join(
        [
            "error: exp/mssim/scripts/experiment.py has been deprecated.",
            "",
            "Run MSSIM via the unified runner:",
            "  python -m exp.runner run mssim <experiment_name>",
            "",
            "Example:",
            "  python -m exp.runner run mssim e2e_test",
            "  python -m exp.runner run mssim e2e_test --dry-run",
            "",
            "Configure experiments under:",
            "  exp/mssim/data/in/<experiment_name>/",
            "",
        ]
    )
    print(msg, file=sys.stderr)
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
