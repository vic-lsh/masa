"""
Generate all plots for an experiment.
"""

from . import goodput
from . import latency
from .util import parse_args


def generate_all_plots(args):
    """
    Generate all plots (goodput and latency) for an experiment.
    
    Args:
        args: Parsed arguments with config_dir, data_dir, and output_dir
    """
    goodput.generate_plots(args)
    latency.generate_plots(args)


if __name__ == "__main__":
    args = parse_args()
    generate_all_plots(args)
