"""
Generate all plots for an experiment.
"""

import os
import shutil
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path

from . import goodput
from . import latency
from . import queueing
from . import mssim
from . import cpu
from .util import (
    get_plot_worker_count,
    load_plot_data,
    parse_args,
    prepare_output_dir,
)


def generate_all_plots(args):
    """
    Generate all plots for an experiment.

    Args:
        args: Parsed arguments with config_dir, data_dir, and output_dir

    Output directory structure:
        <output_dir>/
          goodput_ALL_aggregated.png   # Quick-glance headline result
          0/ 1/ 2/ ...                 # Per-repeat detailed plots (unchanged)
          summary/
            goodput/                   # Averaged goodput plots and CSVs
            early_return/              # Averaged early-return & SLO-miss breakdowns
            tail_latency/              # Averaged p80/p90/p99 plots and latency CSVs
            cpu/                       # CPU utilization plots and summary CSV
    """
    config_dir = Path(args.config_dir)
    if (config_dir / "mssim.json").exists():
        mssim.generate_plots(args)
        return

    prepare_output_dir(args)

    # Remove existing plot files before generating new ones
    output_dir = Path(args.output_dir)
    if output_dir.exists():
        for png_file in output_dir.rglob("*.png"):
            try:
                os.remove(png_file)
            except OSError as e:
                print(f"Warning: Could not remove {png_file}: {e}")

    plot_data = load_plot_data(config_dir, args.data_dir)

    # Generate goodput, latency, and CPU plots in parallel
    data_dir = Path(args.data_dir)
    cpu_dir = output_dir / "summary" / "cpu"
    cpu_dir.mkdir(parents=True, exist_ok=True)

    task_count = 4
    with ThreadPoolExecutor(
        max_workers=get_plot_worker_count(task_count, max_workers=4)
    ) as executor:
        futures = {
            executor.submit(
                goodput.generate_plots, args, plot_data=plot_data
            ): "goodput",
            executor.submit(
                latency.generate_plots, args, plot_data=plot_data
            ): "latency",
            executor.submit(
                queueing.generate_plots, args, plot_data=plot_data
            ): "queueing",
            executor.submit(
                cpu.plot_cpu_utilization,
                data_dir,
                cpu_dir,
                policies=plot_data.policies,
            ): "cpu",
        }

        for future in as_completed(futures):
            plot_type = futures[future]
            try:
                future.result()
            except Exception as e:
                raise RuntimeError(f"Failed to generate {plot_type} plots: {e}") from e

    # Copy headline goodput plot to root for quick access
    headline = output_dir / "summary" / "goodput" / "goodput_ALL_aggregated.png"
    if headline.exists():
        shutil.copy2(headline, output_dir / "goodput_ALL_aggregated.png")


if __name__ == "__main__":
    args = parse_args()
    generate_all_plots(args)
