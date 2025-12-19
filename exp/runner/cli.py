"""
Command-line interface for the experiment runner.
"""

import argparse
import logging
import subprocess
import sys
from pathlib import Path

from .apps import get_app_plugin
from .config import ExperimentConfig
from .experiment import Experiment

# Setup logging
logging.basicConfig(
    level=logging.INFO,
    format='%(asctime)s - %(name)s - %(levelname)s - %(message)s',
    datefmt='%Y-%m-%d %H:%M:%S'
)

logger = logging.getLogger(__name__)


def find_repo_root() -> Path:
    """Find the repository root directory."""
    try:
        result = subprocess.run(
            ["git", "rev-parse", "--show-toplevel"],
            capture_output=True,
            text=True,
            check=True,
        )
        return Path(result.stdout.strip())
    except subprocess.CalledProcessError:
        logger.error("Not in a git repository")
        sys.exit(1)


def cmd_run_experiment(args: argparse.Namespace) -> None:
    """
    Run a single experiment.
    
    Args:
        args: Parsed command-line arguments
    """
    repo_root = find_repo_root()
    
    # Get application plugin
    try:
        app_plugin = get_app_plugin(args.app)
    except ValueError as e:
        logger.error(str(e))
        sys.exit(1)
    
    # Load experiment configuration
    try:
        config = ExperimentConfig.load(
            experiment_name=args.experiment,
            app_name=args.app,
            repo_root=repo_root,
            app_plugin=app_plugin,
        )
    except (FileNotFoundError, ValueError) as e:
        logger.error(f"Failed to load experiment configuration: {e}")
        sys.exit(1)
    
    # Create and run experiment
    experiment = Experiment(
        app=app_plugin,
        config=config,
        repo_root=repo_root,
        plot=args.plot,
        no_cache=args.no_cache,
    )
    
    try:
        experiment.run()
        logger.info("Experiment completed successfully!")
    except Exception as e:
        logger.error(f"Experiment failed: {e}", exc_info=True)
        sys.exit(1)


def cmd_queue_experiments(args: argparse.Namespace) -> None:
    """
    Run multiple experiments sequentially.
    
    Args:
        args: Parsed command-line arguments
    """
    experiments = args.experiments.split()
    
    logger.info(f"Queuing {len(experiments)} experiments: {', '.join(experiments)}")
    
    for exp_name in experiments:
        logger.info(f"{'='*70}")
        logger.info(f"Running experiment: {exp_name}")
        logger.info(f"{'='*70}")
        
        # Create args for single experiment
        exp_args = argparse.Namespace(
            app=args.app,
            experiment=exp_name,
            plot=args.plot,
            no_cache=args.no_cache,
        )
        
        try:
            cmd_run_experiment(exp_args)
        except SystemExit as e:
            if e.code != 0:
                logger.error(f"Experiment {exp_name} failed, stopping queue")
                sys.exit(1)
    
    logger.info("All queued experiments completed successfully!")


def cmd_plot(args: argparse.Namespace) -> None:
    """
    Generate plots for an existing experiment.
    
    Args:
        args: Parsed command-line arguments
    """
    repo_root = find_repo_root()
    
    exp_dir = repo_root / "exp" / args.app
    
    if not exp_dir.exists():
        logger.error(f"Experiment directory not found: {exp_dir}")
        sys.exit(1)
    
    plotting_script = repo_root / "exp" / "common" / "scripts" / "plotting" / "plot-experiment.sh"
    
    if not plotting_script.exists():
        logger.error(f"Plotting script not found: {plotting_script}")
        sys.exit(1)
    
    logger.info(f"Generating plots for experiment: {args.experiment}")
    
    try:
        subprocess.run(
            [str(plotting_script), args.experiment],
            cwd=exp_dir,
            check=True,
        )
        logger.info("Plots generated successfully!")
    except subprocess.CalledProcessError as e:
        logger.error(f"Failed to generate plots: {e}")
        sys.exit(1)


def main() -> None:
    """Main entry point for the CLI."""
    parser = argparse.ArgumentParser(
        description="MASA Experiment Runner - Run performance experiments with different scheduling policies",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="""
Examples:
  # Run a single experiment
  python -m exp.runner run-experiment hotel exp1 --plot
  
  # Queue multiple experiments
  python -m exp.runner queue-experiments synthetic "exp1 exp2 exp3" --plot
  
  # Generate plots only
  python -m exp.runner plot hotel exp1
  
  # Run with verbose logging
  python -m exp.runner run-experiment hotel exp1 --verbose
        """
    )
    
    parser.add_argument(
        '--verbose', '-v',
        action='store_true',
        help='Enable verbose (DEBUG) logging'
    )
    
    subparsers = parser.add_subparsers(dest='command', help='Command to execute')
    subparsers.required = True
    
    # run-experiment command
    run_parser = subparsers.add_parser(
        'run-experiment',
        help='Run a single experiment',
        description='Run a performance experiment with the specified application and configuration'
    )
    run_parser.add_argument(
        'app',
        choices=['hotel', 'synthetic'],
        help='Application to run (hotel or synthetic)'
    )
    run_parser.add_argument(
        'experiment',
        help='Name of the experiment (directory name in exp/<app>/data/in/)'
    )
    run_parser.add_argument(
        '--plot',
        action='store_true',
        help='Generate plots after experiment completion'
    )
    run_parser.add_argument(
        '--no-cache',
        action='store_true',
        help='Disable Docker cache during build'
    )
    run_parser.set_defaults(func=cmd_run_experiment)
    
    # queue-experiments command
    queue_parser = subparsers.add_parser(
        'queue-experiments',
        help='Run multiple experiments sequentially',
        description='Queue and run multiple experiments one after another'
    )
    queue_parser.add_argument(
        'app',
        choices=['hotel', 'synthetic'],
        help='Application to run (hotel or synthetic)'
    )
    queue_parser.add_argument(
        'experiments',
        help='Space-separated list of experiment names (quoted)'
    )
    queue_parser.add_argument(
        '--plot',
        action='store_true',
        help='Generate plots after each experiment'
    )
    queue_parser.add_argument(
        '--no-cache',
        action='store_true',
        help='Disable Docker cache during builds'
    )
    queue_parser.set_defaults(func=cmd_queue_experiments)
    
    # plot command
    plot_parser = subparsers.add_parser(
        'plot',
        help='Generate plots for an existing experiment',
        description='Generate plots from existing experiment output data'
    )
    plot_parser.add_argument(
        'app',
        choices=['hotel', 'synthetic'],
        help='Application name (hotel or synthetic)'
    )
    plot_parser.add_argument(
        'experiment',
        help='Name of the experiment to plot'
    )
    plot_parser.set_defaults(func=cmd_plot)
    
    # Parse arguments
    args = parser.parse_args()
    
    # Setup logging level
    if args.verbose:
        logging.getLogger().setLevel(logging.DEBUG)
        logger.debug("Verbose logging enabled")
    
    # Execute command
    args.func(args)


if __name__ == '__main__':
    main()
