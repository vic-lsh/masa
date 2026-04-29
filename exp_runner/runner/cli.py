"""
Command-line interface for the experiment runner.
"""

import argparse
import logging
import os
import shlex
import shutil
import subprocess
import sys
from argparse import Namespace
from pathlib import Path
from dataclasses import dataclass

from .apps import get_app_plugin
from .apps.base import AppBuilder
from .config import ExperimentConfig
from .experiment import Experiment
from .optimizer import OptimizerConfig, RajomonOptimizer
from .pred_optimizer import PredOptimizerConfig, PredOptimizer
from .plotting import generate_all_plots
from .plotting.all import PLOT_MODULES
from .plotting.replicas import generate_replicas_plots


def _split_csv(value: str | None) -> list[str]:
    if not value:
        return []
    return [item.strip() for item in value.split(",") if item.strip()]


def _add_plot_selector_args(parser: argparse.ArgumentParser) -> None:
    """Shared --only / --skip / --summary-only / --no-plot-cache flags."""
    known = ", ".join(PLOT_MODULES)
    parser.add_argument(
        "--only",
        default=None,
        help=f"Comma-separated subset of plot modules to run ({known}). "
        "Overrides --skip when both are set.",
    )
    parser.add_argument(
        "--skip",
        default=None,
        help=f"Comma-separated plot modules to exclude ({known}).",
    )
    parser.add_argument(
        "--summary-only",
        action="store_true",
        help="Skip per-iteration plots; only render the summary/ directory.",
    )
    parser.add_argument(
        "--no-plot-cache",
        action="store_true",
        help="Bypass <data_dir>/.plotcache.pkl and re-parse all CSVs.",
    )


def _plot_options(args: argparse.Namespace) -> dict:
    """Extract the four plot-selector flags into a kwarg dict for
    `generate_all_plots`. Safe to call on a Namespace that doesn't carry
    them — defaults match `generate_all_plots`'s defaults."""
    return {
        "only": _split_csv(getattr(args, "only", None)),
        "skip": _split_csv(getattr(args, "skip", None)),
        "summary_only": bool(getattr(args, "summary_only", False)),
        "use_cache": not bool(getattr(args, "no_plot_cache", False)),
    }

# Setup logging
logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s - %(name)s - %(levelname)s - %(message)s",
    datefmt="%Y-%m-%d %H:%M:%S",
)

logger = logging.getLogger(__name__)


@dataclass
class BuildContext:
    """Context object returned by _get_build_context."""

    config: "ExperimentConfig"
    policies_to_build: list[str]
    gen_config_path: Path
    builder: AppBuilder


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


def _get_build_context(args: argparse.Namespace, repo_root: Path) -> BuildContext:
    """Helper function to load config, validate files, and create a builder for build commands."""

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

    # Determine which policies to build
    policies_to_build = [args.policy] if args.policy else config.policies

    # Get app config path if it exists
    docker_config = app_plugin.get_docker_config()
    if docker_config.app_config_filename:
        app_config_path = config.in_dir / docker_config.app_config_filename
        if not app_config_path.exists():
            logger.error(f"App config not found at: {app_config_path}")
            sys.exit(1)

    # Get gen_config.json path
    gen_config_path = config.in_dir / "gen_config.json"
    if not gen_config_path.exists():
        logger.error(f"gen_config.json not found at: {gen_config_path}")
        sys.exit(1)

    # Create builder
    builder = app_plugin.create_builder()

    return BuildContext(config, policies_to_build, gen_config_path, builder)


def cmd_run_experiment(args: argparse.Namespace) -> None:
    """
    Run a single experiment.

    Args:
        args: Parsed command-line arguments
    """
    repo_root = find_repo_root()

    # --kind implies --k8s
    if getattr(args, "kind", False):
        args.k8s = True

    # Get application plugin
    try:
        app_plugin = get_app_plugin(args.app)
    except ValueError as e:
        logger.error(str(e))
        sys.exit(1)

    # If running with --kind, set the KIND_CLUSTER_NAME environment variable
    # (only if not already set by the calling shell script, e.g. CI sets kind-{job_id})
    if getattr(args, "kind", False) and "KIND_CLUSTER_NAME" not in os.environ:
        os.environ["KIND_CLUSTER_NAME"] = "kind"

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
        rm_data=args.rm_data,
        dry_run=args.dry_run,
        smoke_test=args.smoke_test,
        use_k8s=args.k8s,
        plot_options=_plot_options(args),
    )

    try:
        experiment.run()
        logger.info("✅ Experiment completed successfully!")
    except Exception as e:
        logger.error(f"⚠️ Experiment failed: {e}", exc_info=True)
        # Print log tails if we have a current output directory and are in CI
        if experiment.current_output_dir and os.environ.get("CI") == "true":
            experiment._print_log_tails(experiment.current_output_dir)
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
        logger.info(f"{'=' * 70}")
        logger.info(f"Running experiment: {exp_name}")
        logger.info(f"{'=' * 70}")

        # Create args for single experiment
        exp_args = argparse.Namespace(
            app=args.app,
            experiment=exp_name,
            plot=args.plot,
            no_cache=args.no_cache,
            rm_data=args.rm_data,
            dry_run=args.dry_run,
            smoke_test=args.smoke_test,
            k8s=args.k8s,
            kind=args.kind,
            only=getattr(args, "only", None),
            skip=getattr(args, "skip", None),
            summary_only=getattr(args, "summary_only", False),
            no_plot_cache=getattr(args, "no_plot_cache", False),
        )

        try:
            cmd_run_experiment(exp_args)
        except SystemExit as e:
            if e.code != 0:
                logger.error(f"Experiment {exp_name} failed, stopping queue")
                sys.exit(1)

    logger.info("All queued experiments completed successfully!")


def cmd_build(args: argparse.Namespace) -> None:
    """
    Build Docker images for an experiment without running it.

    Args:
        args: Parsed command-line arguments
    """
    repo_root = find_repo_root()

    context = _get_build_context(args, repo_root)

    logger.info(
        f"Building Docker images for experiment: {context.config.experiment_name}"
    )
    logger.info(f"Application: {context.config.app_name}")
    logger.info(f"Policies to build: {', '.join(context.policies_to_build)}")

    # Build images for each policy
    for policy in context.policies_to_build:
        logger.info(f"{'=' * 60}")
        logger.info(f"Building images for policy: {policy}")
        logger.info(f"{'=' * 60}")

        try:
            context.builder.build(
                repo_root=repo_root,
                app_dir=context.config.app_dir,
                features=policy,
                rust_log="info",
                no_cache=args.no_cache,
                gen_config_path=context.gen_config_path,
                dry_run=False,
            )
            logger.info(f"Successfully built images for policy: {policy}")
        except Exception as e:
            logger.error(
                f"Failed to build images for policy {policy}: {e}", exc_info=True
            )
            sys.exit(1)

    logger.info("All Docker images built successfully!")


def cmd_build_dryrun(args: argparse.Namespace) -> None:
    """
    Output the build commands that would be run without executing them.

    Args:
        args: Parsed command-line arguments
    """
    repo_root = find_repo_root()

    context = _get_build_context(args, repo_root)

    logger.info(
        f"Dry-run: Would build Docker images for experiment: {context.config.experiment_name}"
    )
    logger.info(f"Application: {context.config.app_name}")
    logger.info(f"Policies to build: {', '.join(context.policies_to_build)}")

    # Collect all commands
    all_commands: list[tuple[str, list[str]]] = []  # (policy, command)

    # Build images for each policy
    for policy in context.policies_to_build:
        logger.info(f"{'=' * 60}")
        logger.info(f"Dry-run: Would build images for policy: {policy}")
        logger.info(f"{'=' * 60}")

        try:
            commands = context.builder.build(
                repo_root=repo_root,
                app_dir=context.config.app_dir,
                features=policy,
                rust_log="info",
                no_cache=args.no_cache,
                gen_config_path=context.gen_config_path,
                dry_run=True,
            )
            if commands:
                for cmd in commands:
                    all_commands.append((policy, cmd))
        except Exception as e:
            logger.error(
                f"Failed to generate commands for policy {policy}: {e}", exc_info=True
            )
            sys.exit(1)

    # Print all commands
    print("\n" + "=" * 80)
    print("BUILD COMMANDS (DRY-RUN)")
    print("=" * 80 + "\n")
    print(f"# Working directory: {repo_root}")
    print("# All commands should be run from the repository root\n")

    for policy, cmd in all_commands:
        print(f"# Policy: {policy}")
        print(" ".join(shlex.quote(arg) for arg in cmd))
        print()

    print(f"# Total commands: {len(all_commands)}")
    logger.info("Dry-run completed successfully!")


def cmd_plot(args: argparse.Namespace) -> None:
    """
    Generate plots for an existing experiment.

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

    logger.info(f"Generating plots for experiment: {args.experiment}")
    print(f"Plot output directory: {config.plot_dir}")

    # Clear plot directory before generating new plots
    if config.plot_dir.exists():
        shutil.rmtree(config.plot_dir)
    config.plot_dir.mkdir(parents=True, exist_ok=True)
    logger.debug(f"Cleared plot directory: {config.plot_dir}")

    # Create args-like object for plotting functions
    plot_args = Namespace(
        config_dir=config.in_dir,
        data_dir=config.out_dir,
        output_dir=config.plot_dir,
    )

    try:
        generate_all_plots(plot_args, **_plot_options(args))
        logger.info("Plots generated successfully!")
    except Exception as e:
        logger.error(f"Failed to generate plots: {e}")
        sys.exit(1)


def cmd_plot_replicas(args: argparse.Namespace) -> None:
    """
    Generate replica plots for hotel experiments.

    Args:
        args: Parsed command-line arguments
    """
    repo_root = find_repo_root()
    app_name = args.app

    if app_name != "hotel":
        logger.error("Replica plots are only available for the hotel app")
        sys.exit(1)

    in_dir = repo_root / "exp" / app_name / "in"
    output_dir = repo_root / "exp" / app_name / "plots" / "replicas"

    if not in_dir.exists():
        logger.error(f"Input directory not found: {in_dir}")
        sys.exit(1)

    if output_dir.exists():
        shutil.rmtree(output_dir)
    output_dir.mkdir(parents=True, exist_ok=True)

    try:
        generate_replicas_plots(in_dir, output_dir)
        logger.info("Replica plots generated successfully!")
        print(f"Replica plot output directory: {output_dir}")
    except Exception as e:
        logger.error(f"Failed to generate replica plots: {e}")
        sys.exit(1)


def cmd_optimize(args: argparse.Namespace) -> None:
    """
    Run Bayesian optimization for Rajomon parameters.

    Args:
        args: Parsed command-line arguments
    """
    repo_root = find_repo_root()

    output_path = Path(args.output) if args.output else None

    config = OptimizerConfig(
        app=args.app,
        experiment_base=args.experiment_base,
        policy=args.policy,
        saturation_rps=args.saturation_rps,
        n_iterations=args.iterations,
        penalty_weight=args.penalty_weight,
        warm_start_path=Path(args.warm_start) if args.warm_start else None,
        output_path=output_path,
    )

    optimizer = RajomonOptimizer(config, repo_root)

    try:
        best_params = optimizer.run()
        if best_params:
            logger.info("Optimization completed successfully!")
            logger.info(f"Best params: {best_params}")
        else:
            logger.error("Optimization completed with no successful trials")
            sys.exit(1)
    except Exception as e:
        logger.error(f"Optimization failed: {e}", exc_info=True)
        sys.exit(1)


def cmd_optimize_pred(args: argparse.Namespace) -> None:
    """
    Run Bayesian optimization for predictive admission control parameters.
    """
    repo_root = find_repo_root()

    output_path = Path(args.output) if args.output else None

    config = PredOptimizerConfig(
        app=args.app,
        experiment_base=args.experiment_base,
        policy=args.policy,
        saturation_rps=args.saturation_rps,
        n_iterations=args.iterations,
        objective_mode=args.objective_mode,
        stability_weight=args.stability_weight,
        window_secs=args.window_secs,
        floor_percentile=args.floor_percentile,
        warm_start_path=Path(args.warm_start) if args.warm_start else None,
        warmup_secs=args.warmup_secs,
        duration_secs=args.duration_secs,
        output_path=output_path,
    )

    optimizer = PredOptimizer(config, repo_root)

    try:
        best_params = optimizer.run()
        if best_params:
            logger.info("Optimization completed successfully!")
        else:
            logger.error("Optimization completed with no successful trials")
            sys.exit(1)
    except Exception as e:
        logger.error(f"Optimization failed: {e}", exc_info=True)
        sys.exit(1)


def create_parser() -> argparse.ArgumentParser:
    """Main entry point for the CLI."""
    parser = argparse.ArgumentParser(
        description="MASA Experiment Runner - Run performance experiments with different scheduling policies",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="""
Examples:
  # Run a single experiment
  uv run -m exp_runner run hotel exp1 --plot

  # Build Docker images for an experiment
  uv run -m exp_runner build hotel exp1

  # Build images for a specific policy (new or old flag names work)
  uv run -m exp_runner build synthetic exp1 --policy sched_slo
  uv run -m exp_runner build synthetic exp1 --policy prio_global  # old name still works

  # Show build commands without executing them
  uv run -m exp_runner build-dryrun hotel exp1

  # Show build commands for a specific policy
  uv run -m exp_runner build-dryrun synthetic exp1 --policy sched_slo

  # Queue multiple experiments
  uv run -m exp_runner run-multiple synthetic "exp1 exp2 exp3" --plot

  # Generate plots only
  uv run -m exp_runner plot hotel exp1
  uv run -m exp_runner plot mssim e2e_test

  # Generate replica plots for hotel experiments
  uv run -m exp_runner plot-replicas hotel

  # Run with verbose logging
  uv run -m exp_runner run hotel exp1 --verbose
        """,
    )

    parser.add_argument(
        "--verbose", "-v", action="store_true", help="Enable verbose (DEBUG) logging"
    )

    subparsers = parser.add_subparsers(dest="command", help="Command to execute")
    subparsers.required = True

    # run command
    run_parser = subparsers.add_parser(
        "run",
        help="Run a single experiment",
        description="Run a performance experiment with the specified application and configuration",
    )
    run_parser.add_argument(
        "app",
        choices=["hotel", "mssim", "socialnet", "synthetic"],
        help="Application to run (hotel, mssim, or synthetic)",
    )
    run_parser.add_argument(
        "experiment",
        help="Name of the experiment (directory name in exp/<app>/data/in/)",
    )
    run_parser.add_argument(
        "--plot", action="store_true", help="Generate plots after experiment completion"
    )
    _add_plot_selector_args(run_parser)
    run_parser.add_argument(
        "--no-cache", action="store_true", help="Disable Docker cache during build"
    )
    run_parser.add_argument(
        "--rm-data",
        action="store_true",
        help="Remove existing data from experiment output directory before running",
    )
    run_parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Print what would be executed without running containers",
    )
    run_parser.add_argument(
        "--smoke-test",
        action="store_true",
        help="Verify experiment results (goodput/files) after completion",
    )
    run_parser.add_argument(
        "--k8s",
        action="store_true",
        help="Run on Kubernetes instead of Docker Compose",
    )
    run_parser.add_argument(
        "--kind",
        action="store_true",
        help="Run on Kind (implies --k8s) and auto-load images",
    )
    run_parser.set_defaults(func=cmd_run_experiment)

    # run-multiple command
    queue_parser = subparsers.add_parser(
        "run-multiple",
        help="Run multiple experiments sequentially",
        description="Queue and run multiple experiments one after another",
    )
    queue_parser.add_argument(
        "app",
        choices=["hotel", "mssim", "socialnet", "synthetic"],
        help="Application to run (hotel, mssim, or synthetic)",
    )
    queue_parser.add_argument(
        "experiments", help="Space-separated list of experiment names (quoted)"
    )
    queue_parser.add_argument(
        "--plot", action="store_true", help="Generate plots after each experiment"
    )
    _add_plot_selector_args(queue_parser)
    queue_parser.add_argument(
        "--no-cache", action="store_true", help="Disable Docker cache during builds"
    )
    queue_parser.add_argument(
        "--rm-data",
        action="store_true",
        help="Remove existing data from experiment output directory before running",
    )
    queue_parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Print what would be executed without running containers",
    )
    queue_parser.add_argument(
        "--smoke-test",
        action="store_true",
        help="Verify experiment results (goodput/files) after completion",
    )
    queue_parser.add_argument(
        "--k8s",
        action="store_true",
        help="Run on Kubernetes instead of Docker Compose",
    )
    queue_parser.add_argument(
        "--kind",
        action="store_true",
        help="Run on Kind (implies --k8s) and auto-load images",
    )
    queue_parser.set_defaults(func=cmd_queue_experiments)

    # build command
    build_parser = subparsers.add_parser(
        "build",
        help="Build Docker images for an experiment without running it",
        description="Build Docker images for all policies (or a specific policy) in an experiment",
    )
    build_parser.add_argument(
        "app",
        choices=["hotel", "mssim", "socialnet", "synthetic"],
        help="Application to build (hotel, mssim, or synthetic)",
    )
    build_parser.add_argument(
        "experiment",
        help="Name of the experiment (directory name in exp/<app>/data/in/)",
    )
    build_parser.add_argument(
        "--policy",
        help="Build images for a specific policy only (default: build all policies)",
    )
    build_parser.add_argument(
        "--no-cache", action="store_true", help="Disable Docker cache during build"
    )
    build_parser.set_defaults(func=cmd_build)

    # build-dryrun command
    build_dryrun_parser = subparsers.add_parser(
        "build-dryrun",
        help="Output build commands without executing them",
        description="Show the Docker build commands that would be executed without actually running them",
    )
    build_dryrun_parser.add_argument(
        "app",
        choices=["hotel", "mssim", "socialnet", "synthetic"],
        help="Application to build (hotel, mssim, or synthetic)",
    )
    build_dryrun_parser.add_argument(
        "experiment",
        help="Name of the experiment (directory name in exp/<app>/data/in/)",
    )
    build_dryrun_parser.add_argument(
        "--policy",
        help="Build images for a specific policy only (default: build all policies)",
    )
    build_dryrun_parser.add_argument(
        "--no-cache",
        action="store_true",
        help="Disable Docker cache during build (affects command output)",
    )
    build_dryrun_parser.set_defaults(func=cmd_build_dryrun)

    # plot command
    plot_parser = subparsers.add_parser(
        "plot",
        help="Generate plots for an existing experiment",
        description="Generate plots from existing experiment output data",
    )
    plot_parser.add_argument(
        "app",
        choices=["hotel", "mssim", "socialnet", "synthetic"],
        help="Application name (hotel, mssim, or synthetic)",
    )
    plot_parser.add_argument("experiment", help="Name of the experiment to plot")
    _add_plot_selector_args(plot_parser)
    plot_parser.set_defaults(func=cmd_plot)

    # plot-replicas command
    plot_replicas_parser = subparsers.add_parser(
        "plot-replicas",
        help="Generate replica plots from hotel experiment inputs",
        description="Scan exp/<app>/data/in for <policy>_<rps> directories and plot replicas",
    )
    plot_replicas_parser.add_argument(
        "app", choices=["hotel"], help="Application name (hotel only)"
    )
    plot_replicas_parser.set_defaults(func=cmd_plot_replicas)

    # optimize command
    optimize_parser = subparsers.add_parser(
        "optimize",
        help="Optimize Rajomon admission control parameters via Bayesian optimization",
        description="Run Optuna-based Bayesian optimization to find good Rajomon parameters",
    )
    optimize_parser.add_argument(
        "app",
        choices=["hotel", "mssim", "socialnet", "synthetic"],
        help="Application to optimize",
    )
    optimize_parser.add_argument(
        "experiment_base",
        help="Existing experiment to copy app topology from (e.g., rajomon_hotel)",
    )
    optimize_parser.add_argument(
        "--policy",
        required=True,
        help="Feature flags (e.g., sched_slo,ac_rajomon,abort_slo)",
    )
    optimize_parser.add_argument(
        "--saturation-rps",
        type=int,
        required=True,
        help="Max sustainable RPS (used to generate sweep: 0.8x-2.0x)",
    )
    optimize_parser.add_argument(
        "--iterations",
        type=int,
        default=30,
        help="Number of optimization iterations (default: 30)",
    )
    optimize_parser.add_argument(
        "--penalty-weight",
        type=float,
        default=10.0,
        help="Penalty multiplier for p99 SLO violations (default: 10.0)",
    )
    optimize_parser.add_argument(
        "--warm-start",
        help="Path to previous best_params.json for warm-starting",
    )
    optimize_parser.add_argument(
        "--output",
        help="Path to write best params (default: exp/<app>/out/_opt_rajomon/best_params.json)",
    )
    optimize_parser.set_defaults(func=cmd_optimize)

    # optimize-pred command
    optimize_pred_parser = subparsers.add_parser(
        "optimize-pred",
        help="Optimize predictive admission control (ac_pred/abort_slack) parameters",
        description=(
            "Run Optuna-based Bayesian optimization for PredParams "
            "(tau_er, estimator_k, aimd_alpha, aimd_beta, aimd_er_threshold). "
            "Objective maximizes goodput while penalizing intra-run oscillation."
        ),
    )
    optimize_pred_parser.add_argument(
        "app",
        choices=["hotel", "mssim", "socialnet", "synthetic"],
        help="Application to optimize",
    )
    optimize_pred_parser.add_argument(
        "experiment_base",
        help="Existing experiment to copy app topology from (e.g., ember_10)",
    )
    optimize_pred_parser.add_argument(
        "--policy",
        default="sched_pred,ac_pred,abort_slack,est_mean_var",
        help=(
            "Feature flags to optimize "
            "(default: sched_pred,ac_pred,abort_slack,est_mean_var)"
        ),
    )
    optimize_pred_parser.add_argument(
        "--saturation-rps",
        type=int,
        required=True,
        help="Max sustainable RPS (used to generate sweep: 0.8x–2.0x)",
    )
    optimize_pred_parser.add_argument(
        "--iterations",
        type=int,
        default=30,
        help="Number of optimization iterations (default: 30)",
    )
    optimize_pred_parser.add_argument(
        "--objective-mode",
        choices=["sharpe", "floor", "penalized"],
        default="sharpe",
        help=(
            "Objective function mode (default: sharpe). "
            "sharpe: mean_gp/(1+w*CV); "
            "floor: percentile(windows, q); "
            "penalized: mean_gp - w*std_gp"
        ),
    )
    optimize_pred_parser.add_argument(
        "--stability-weight",
        type=float,
        default=0.3,
        help=(
            "Stability penalty weight for sharpe/penalized modes (default: 0.3). "
            "0 = pure goodput maximization."
        ),
    )
    optimize_pred_parser.add_argument(
        "--window-secs",
        type=float,
        default=5.0,
        help="Width of each goodput measurement window in seconds (default: 5.0)",
    )
    optimize_pred_parser.add_argument(
        "--floor-percentile",
        type=float,
        default=10.0,
        help="Percentile for floor objective mode (default: 10th)",
    )
    optimize_pred_parser.add_argument(
        "--warmup-secs",
        type=int,
        default=15,
        help="Warmup duration per trial in seconds (default: 15)",
    )
    optimize_pred_parser.add_argument(
        "--duration-secs",
        type=int,
        default=45,
        help="Measurement duration per trial in seconds (default: 45)",
    )
    optimize_pred_parser.add_argument(
        "--warm-start",
        help="Path to previous best_params.json for warm-starting",
    )
    optimize_pred_parser.add_argument(
        "--output",
        help=(
            "Path to write best params JSON "
            "(default: exp/<app>/out/_opt_pred/best_params.json)"
        ),
    )
    optimize_pred_parser.set_defaults(func=cmd_optimize_pred)

    return parser


def main() -> None:
    """Main entry point for the CLI."""
    parser = create_parser()
    # Parse arguments
    args = parser.parse_args()

    # Setup logging level
    if args.verbose:
        logging.getLogger().setLevel(logging.DEBUG)
        logger.debug("Verbose logging enabled")

    # Execute command
    args.func(args)


if __name__ == "__main__":
    main()
