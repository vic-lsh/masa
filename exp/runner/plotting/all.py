"""
Generate all plots for an experiment.
"""

import os
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path

from . import goodput
from . import latency
from . import mssim
from .util import parse_args


def generate_all_plots(args):
    """
    Generate all plots for an experiment.
    
    Args:
        args: Parsed arguments with config_dir, data_dir, and output_dir
    """
    config_dir = Path(args.config_dir)
    if (config_dir / "mssim.json").exists():
        mssim.generate_plots(args)
        return

    # Remove existing plot files before generating new ones
    output_dir = Path(args.output_dir)
    if output_dir.exists():
        for png_file in output_dir.rglob("*.png"):
            try:
                os.remove(png_file)
            except OSError as e:
                print(f"Warning: Could not remove {png_file}: {e}")
    
    # Generate goodput and latency plots in parallel
    with ThreadPoolExecutor(max_workers=2) as executor:
        futures = {
            executor.submit(goodput.generate_plots, args): "goodput",
            executor.submit(latency.generate_plots, args): "latency",
        }
        
        for future in as_completed(futures):
            plot_type = futures[future]
            try:
                future.result()
            except Exception as e:
                raise RuntimeError(f"Failed to generate {plot_type} plots: {e}") from e


if __name__ == "__main__":
    args = parse_args()
    generate_all_plots(args)
