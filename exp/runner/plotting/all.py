"""
Generate all plots for an experiment.
"""

import os
from pathlib import Path

from . import goodput
from . import latency
from .util import parse_args


def generate_all_plots(args):
    """
    Generate all plots (goodput and latency) for an experiment.
    
    Args:
        args: Parsed arguments with config_dir, data_dir, and output_dir
    """
    # Remove existing plot files before generating new ones
    output_dir = Path(args.output_dir)
    if output_dir.exists():
        for png_file in output_dir.rglob("*.png"):
            try:
                os.remove(png_file)
            except OSError as e:
                print(f"Warning: Could not remove {png_file}: {e}")
    
    goodput.generate_plots(args)
    latency.generate_plots(args)


if __name__ == "__main__":
    args = parse_args()
    generate_all_plots(args)
