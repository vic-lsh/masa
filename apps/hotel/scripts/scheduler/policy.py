#!/usr/bin/env python3
"""Plot goodput comparison between FIFO and prio-global schedulers."""

from __future__ import annotations

import argparse
import csv
import math
import re
import subprocess
import sys
from pathlib import Path
from typing import Dict, Iterable, List, Optional

import matplotlib.pyplot as plt

try:
    import pandas as pd  # type: ignore
except ImportError:  # pragma: no cover - pandas is optional at runtime
    pd = None

try:
    from opt import calculate_goodput_from_log
except ModuleNotFoundError:
    def calculate_goodput_from_log(input_file: str) -> float:
        total_requests = 0
        successful_requests = 0
        try:
            with open(input_file, "r", encoding="utf-8") as fh:
                header = fh.readline().strip().split(",")
                if "error" not in header:
                    raise ValueError(f"'error' column not found in {input_file}")
                error_col_idx = header.index("error")

                for line in fh:
                    if not line.strip():
                        continue
                    parts = line.strip().split(",")
                    if len(parts) <= error_col_idx:
                        continue
                    total_requests += 1
                    if parts[error_col_idx] == "/None":
                        successful_requests += 1
        except FileNotFoundError as exc:
            raise FileNotFoundError(f"Input file not found at {input_file}") from exc

        return successful_requests / total_requests if total_requests else 0.0


RPS_PATTERN = re.compile(r"r(\d+)", re.IGNORECASE)
DEFAULT_DATA_DIR = Path(__file__).resolve().parents[3] / "data" / "out" / "queue-experiment01" / "0"
DEFAULT_OUTPUT_NAME = "goodput_fifo_vs_prio_global.png"
TRACE_PARSER_SCRIPT = Path(__file__).resolve().parents[1] / "frontend_trace_parser.py"
TRACE_OUTPUT_DIR = Path(__file__).resolve().parents[4] / "trace-analysis" / "golden" / "hotel"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Plot goodput for FIFO vs prio-global schedulers.")
    parser.add_argument(
        "--data-dir",
        type=Path,
        default=DEFAULT_DATA_DIR,
        help="Directory containing scheduler outputs (defaults to queue-experiment01/0).",
    )
    parser.add_argument(
        "--output",
        type=Path,
        help="Optional path for the generated chart (defaults inside data-dir).",
    )
    parser.add_argument(
        "--show",
        action="store_true",
        help="Display the chart interactively after saving it.",
    )
    return parser.parse_args()


def extract_rps(csv_path: Path) -> int:
    match = RPS_PATTERN.search(csv_path.stem)
    if not match:
        raise ValueError(f"Could not infer RPS from filename: {csv_path}")
    return int(match.group(1))


def iter_rps_csvs(scheduler_dir: Path) -> Iterable[Path]:
    return sorted(scheduler_dir.glob("r*_*.[cC][sS][vV]"))


def calculate_goodput_with_csv(csv_path: Path) -> float:
    with csv_path.open(newline="") as fh:
        reader = csv.DictReader(fh)
        if not reader.fieldnames or "error" not in reader.fieldnames:
            raise ValueError(f"CSV file {csv_path} is missing an 'error' column")
        successful = 0
        total = 0
        for row in reader:
            total += 1
            if (row.get("error") or "").strip() == "/None":
                successful += 1
    return successful / total if total else math.nan


def compute_fifo_goodput(
    fifo_dir: Path, csv_paths: Optional[Iterable[Path]] = None
) -> Dict[int, float]:
    results: Dict[int, float] = {}
    paths = csv_paths if csv_paths is not None else iter_rps_csvs(fifo_dir)
    for csv_path in paths:
        rps = extract_rps(csv_path)
        results[rps] = calculate_goodput_from_log(str(csv_path))
    return results


def compute_prio_goodput(
    prio_dir: Path, csv_paths: Optional[Iterable[Path]] = None
) -> Dict[int, float]:
    results: Dict[int, float] = {}
    paths = csv_paths if csv_paths is not None else iter_rps_csvs(prio_dir)
    for csv_path in paths:
        rps = extract_rps(csv_path)
        if pd is not None:
            df = pd.read_csv(csv_path)
            if "error" not in df.columns:
                raise ValueError(f"CSV file {csv_path} is missing an 'error' column")
            goodput = (df["error"].astype(str).str.strip() == "/None").mean()
        else:
            goodput = calculate_goodput_with_csv(csv_path)
        results[rps] = float(goodput)
    return results


def run_frontend_trace_parser(csv_paths: Iterable[Path], output_dir: Path) -> List[Path]:
    csv_list = list(csv_paths)
    if not csv_list:
        return []
    if not TRACE_PARSER_SCRIPT.exists():
        raise FileNotFoundError(
            f"frontend_trace_parser.py not found at {TRACE_PARSER_SCRIPT}"
        )

    output_dir.mkdir(parents=True, exist_ok=True)

    generated: List[Path] = []
    for csv_path in csv_list:
        prefix = f"fifo_{csv_path.stem}"
        target_path = output_dir / f"{prefix}_frontend_modified.json"
        cmd = [
            sys.executable,
            str(TRACE_PARSER_SCRIPT),
            "--input",
            str(csv_path),
            "--output-dir",
            str(output_dir),
            "--prefix",
            prefix,
        ]
        result = subprocess.run(cmd, capture_output=True, text=True)
        if result.returncode != 0:
            raise RuntimeError(
                f"frontend_trace_parser failed for {csv_path}: {result.stderr or result.stdout}"
            )
        generated.append(target_path)
    return generated


def plot_goodput(
    rps_values: List[int],
    fifo_goodput: Dict[int, float],
    prio_goodput: Dict[int, float],
    output_path: Path,
) -> Path:
    fig, ax = plt.subplots(figsize=(6, 4))
    fifo_points = [fifo_goodput.get(rps, math.nan) for rps in rps_values]
    prio_points = [prio_goodput.get(rps, math.nan) for rps in rps_values]

    ax.plot(rps_values, fifo_points, marker="o", label="FIFO")
    ax.plot(rps_values, prio_points, marker="o", label="Prio-Global")

    ax.set_xlabel("Requests Per Second")
    ax.set_ylabel("Goodput")
    ax.set_title("Goodput Comparison")
    ax.set_ylim(0, 1.05)
    ax.grid(True, linestyle="--", linewidth=0.5, alpha=0.7)
    ax.legend()
    fig.tight_layout()

    output_path.parent.mkdir(parents=True, exist_ok=True)
    fig.savefig(output_path, dpi=200)
    plt.close(fig)
    return output_path


def print_goodput_table(rps_values: List[int], fifo: Dict[int, float], prio: Dict[int, float]) -> None:
    header = f"{'RPS':>6} | {'FIFO':>6} | {'Prio':>6}"
    print(header)
    print("-" * len(header))
    for rps in rps_values:
        fifo_val = fifo.get(rps)
        prio_val = prio.get(rps)
        fifo_str = f"{fifo_val:.3f}" if fifo_val is not None else "--"
        prio_str = f"{prio_val:.3f}" if prio_val is not None else "--"
        print(f"{rps:>6} | {fifo_str:>6} | {prio_str:>6}")


def main() -> None:
    args = parse_args()
    data_dir = args.data_dir
    fifo_dir = data_dir / "fifo"
    prio_dir = data_dir / "prio_global"

    if not fifo_dir.exists() or not prio_dir.exists():
        raise FileNotFoundError(f"Could not find expected fifo/prio_global directories under {data_dir}")

    fifo_csv_paths = list(iter_rps_csvs(fifo_dir))
    prio_csv_paths = list(iter_rps_csvs(prio_dir))

    fifo_goodput = compute_fifo_goodput(fifo_dir, fifo_csv_paths)
    prio_goodput = compute_prio_goodput(prio_dir, prio_csv_paths)

    if not fifo_goodput and not prio_goodput:
        raise RuntimeError(f"No CSV data found under {data_dir}")

    rps_values = sorted(set(fifo_goodput) | set(prio_goodput))
    output_path = args.output or (data_dir / DEFAULT_OUTPUT_NAME)
    saved_path = plot_goodput(rps_values, fifo_goodput, prio_goodput, output_path)

    generated_traces = run_frontend_trace_parser(fifo_csv_paths, TRACE_OUTPUT_DIR)

    print_goodput_table(rps_values, fifo_goodput, prio_goodput)
    print(f"Chart saved to: {saved_path}")
    if generated_traces:
        print(f"Frontend traces saved under: {TRACE_OUTPUT_DIR}")

    if args.show:
        plt.imshow(plt.imread(saved_path))
        plt.axis("off")
        plt.show()


if __name__ == "__main__":
    main()
