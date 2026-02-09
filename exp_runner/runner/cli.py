"""
Command-line interface for the experiment runner.
"""

import argparse
import json
import logging
import os
import shlex
import shutil
import subprocess
import sys
import tempfile
from argparse import Namespace
from pathlib import Path

import yaml

from .apps import get_app_plugin
from .config import ExperimentConfig
from .experiment import Experiment
from .experiment_config_v2 import ExperimentConfigV2, ExecutionSpec, LoadGenSpec
from .legacy import convert_legacy_to_experiment_config
from .plotting import generate_all_plots
from .plotting.replicas import generate_replicas_plots

# Setup logging
logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s - %(name)s - %(levelname)s - %(message)s",
    datefmt="%Y-%m-%d %H:%M:%S",
)

logger = logging.getLogger(__name__)


def _warn_compat_mode() -> None:
    logger.warning(
        "Using legacy exp config format (--compat). "
        "The old gen_config.json/policies format will be removed in a future release."
    )


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
    compat = bool(getattr(args, "compat", False))

    if compat:
        _warn_compat_mode()

    # --kind implies --k8s
    if getattr(args, "kind", False):
        args.k8s = True

    # Get application plugin
    try:
        app_plugin = get_app_plugin(args.app)
    except ValueError as e:
        logger.error(str(e))
        sys.exit(1)

    # If running with --kind, wrap run_workload to inject use_kind=True
    if getattr(args, "kind", False):
        original_run_workload = app_plugin.run_workload

        def run_workload_with_kind(*a, **kw):
            kw["use_kind"] = True
            return original_run_workload(*a, **kw)

        app_plugin.run_workload = run_workload_with_kind

    # Load experiment configuration
    try:
        config = ExperimentConfig.load(
            experiment_name=args.experiment,
            app_name=args.app,
            repo_root=repo_root,
            app_plugin=app_plugin,
            compat=compat,
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
        use_new_generator=True,
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
            compat=getattr(args, "compat", False),
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
    compat = bool(getattr(args, "compat", False))
    if compat:
        _warn_compat_mode()

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
            compat=compat,
        )
    except (FileNotFoundError, ValueError) as e:
        logger.error(f"Failed to load experiment configuration: {e}")
        sys.exit(1)

    # Determine which policies to build
    policies_to_build = [args.policy] if args.policy else config.policies

    logger.info(f"Building Docker images for experiment: {config.experiment_name}")
    logger.info(f"Application: {config.app_name}")
    logger.info(f"Policies to build: {', '.join(policies_to_build)}")

    # Get app config path if it exists
    app_config_path = None
    docker_config = app_plugin.get_docker_config()
    if docker_config.app_config_filename:
        app_config_path = config.in_dir / docker_config.app_config_filename
        if not app_config_path.exists():
            logger.error(f"App config not found at: {app_config_path}")
            sys.exit(1)

    # Get gen_config.json path (materialized from v2 when needed)
    gen_config_path = config.get_gen_config_path()

    # Build images for each policy
    builder = app_plugin.create_builder()
    for policy in policies_to_build:
        logger.info(f"{'=' * 60}")
        logger.info(f"Building images for policy: {policy}")
        logger.info(f"{'=' * 60}")

        try:
            builder.build(
                repo_root=repo_root,
                app_dir=config.app_dir,
                features=policy,
                rust_log="info",
                no_cache=args.no_cache,
                gen_config_path=gen_config_path,
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
    compat = bool(getattr(args, "compat", False))
    if compat:
        _warn_compat_mode()

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
            compat=compat,
        )
    except (FileNotFoundError, ValueError) as e:
        logger.error(f"Failed to load experiment configuration: {e}")
        sys.exit(1)

    # Determine which policies to build
    policies_to_build = [args.policy] if args.policy else config.policies

    logger.info(
        f"Dry-run: Would build Docker images for experiment: {config.experiment_name}"
    )
    logger.info(f"Application: {config.app_name}")
    logger.info(f"Policies to build: {', '.join(policies_to_build)}")

    # Get app config path if it exists
    app_config_path = None
    docker_config = app_plugin.get_docker_config()
    if docker_config.app_config_filename:
        app_config_path = config.in_dir / docker_config.app_config_filename
        if not app_config_path.exists():
            logger.error(f"App config not found at: {app_config_path}")
            sys.exit(1)

    # Get gen_config.json path (materialized from v2 when needed)
    gen_config_path = config.get_gen_config_path()

    # Collect all commands
    all_commands: list[tuple[str, list[str]]] = []  # (policy, command)

    # Build images for each policy
    builder = app_plugin.create_builder()
    for policy in policies_to_build:
        logger.info(f"{'=' * 60}")
        logger.info(f"Dry-run: Would build images for policy: {policy}")
        logger.info(f"{'=' * 60}")

        try:
            commands = builder.build(
                repo_root=repo_root,
                app_dir=config.app_dir,
                features=policy,
                rust_log="info",
                no_cache=args.no_cache,
                gen_config_path=gen_config_path,
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
    compat = bool(getattr(args, "compat", False))
    if compat:
        _warn_compat_mode()

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
            compat=compat,
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

    try:
        if config.config_format == "legacy":
            plot_args = Namespace(
                config_dir=config.in_dir,
                data_dir=config.out_dir,
                output_dir=config.plot_dir,
            )
            generate_all_plots(plot_args)
        else:
            with tempfile.TemporaryDirectory(prefix="masa-plot-config-") as tmp:
                compat_config_dir = Path(tmp)
                config.write_plot_compat_inputs(compat_config_dir)
                plot_args = Namespace(
                    config_dir=compat_config_dir,
                    data_dir=config.out_dir,
                    output_dir=config.plot_dir,
                )
                generate_all_plots(plot_args)
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


def cmd_migrate_config(args: argparse.Namespace) -> None:
    """
    Migrate an experiment config directory from legacy to v2 format.
    """
    repo_root = find_repo_root()
    app_name = args.app

    try:
        app_plugin = get_app_plugin(app_name)
    except ValueError as e:
        logger.error(str(e))
        sys.exit(1)

    src_dir = repo_root / "exp" / app_name / "in" / args.source_experiment
    dst_dir = repo_root / "exp" / app_name / "in" / args.target_experiment

    if not src_dir.exists():
        logger.error(f"Source experiment directory not found: {src_dir}")
        sys.exit(1)
    if dst_dir.exists() and not args.force:
        logger.error(
            f"Destination already exists: {dst_dir} (use --force to overwrite)"
        )
        sys.exit(1)
    if dst_dir.exists() and args.force:
        shutil.rmtree(dst_dir)

    gen_config_path = src_dir / "gen_config.json"
    policies_path = src_dir / "policies"
    if not gen_config_path.exists() or not policies_path.exists():
        logger.error(
            f"Source must contain legacy files: {gen_config_path} and {policies_path}"
        )
        sys.exit(1)

    with open(gen_config_path, encoding="utf-8") as f:
        gen_config = json.load(f)
    policies = [
        line.strip()
        for line in policies_path.read_text(encoding="utf-8").splitlines()
        if line.strip()
    ]
    if not policies:
        logger.error(f"Source policies file is empty: {policies_path}")
        sys.exit(1)

    if app_name == "mssim":
        exp_v2 = ExperimentConfigV2(
            name=args.target_experiment,
            app=app_name,
            execution=ExecutionSpec(
                repeats=int(gen_config.get("Repeats", 1)),
                policies=policies,
                warmup_secs=int(gen_config.get("WarmupSecs", 0) or 0),
                duration_secs=int(gen_config.get("DurationSecs", 0) or 0),
            ),
            loadgen=LoadGenSpec(rps=[float(v) for v in (gen_config.get("Rps") or [])]),
            metadata={"source": "migrated_legacy_gen_config"},
        )
    else:
        exp_v2 = convert_legacy_to_experiment_config(
            gen_config=gen_config,
            policies=policies,
            experiment_name=args.target_experiment,
            app_name=app_name,
        )
        exp_v2.metadata["source"] = "migrated_legacy_gen_config"

    dst_dir.mkdir(parents=True, exist_ok=True)
    experiment_yaml = dst_dir / "experiment.yaml"
    experiment_yaml.write_text(
        yaml.safe_dump(exp_v2.to_dict(), sort_keys=False),
        encoding="utf-8",
    )

    for item in src_dir.iterdir():
        if item.name in {"gen_config.json", "policies"}:
            continue
        dest = dst_dir / item.name
        if item.is_dir():
            shutil.copytree(item, dest)
        else:
            shutil.copy2(item, dest)

    docker_config = app_plugin.get_docker_config()
    if docker_config.app_config_filename:
        src_app_cfg = src_dir / docker_config.app_config_filename
        if src_app_cfg.exists():
            shutil.copy2(src_app_cfg, dst_dir / docker_config.app_config_filename)

    logger.info(f"Migrated legacy config: {src_dir} -> {dst_dir}")
    print(f"Wrote: {experiment_yaml}")


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

  # Build images for a specific policy
  uv run -m exp_runner build synthetic exp1 --policy prio_global

  # Show build commands without executing them
  uv run -m exp_runner build-dryrun hotel exp1

  # Show build commands for a specific policy
  uv run -m exp_runner build-dryrun synthetic exp1 --policy prio_global

  # Queue multiple experiments
  uv run -m exp_runner run-multiple synthetic "exp1 exp2 exp3" --plot

  # Run with legacy config format (deprecated)
  uv run -m exp_runner run hotel exp1 --compat

  # Migrate legacy config to new format
  uv run -m exp_runner migrate-config hotel old_exp new_exp

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
        help="Name of the experiment (directory name in exp/<app>/in/)",
    )
    run_parser.add_argument(
        "--plot", action="store_true", help="Generate plots after experiment completion"
    )
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
    run_parser.add_argument(
        "--compat",
        action="store_true",
        help="Use legacy gen_config.json/policies format (deprecated)",
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
    queue_parser.add_argument(
        "--compat",
        action="store_true",
        help="Use legacy gen_config.json/policies format (deprecated)",
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
        help="Name of the experiment (directory name in exp/<app>/in/)",
    )
    build_parser.add_argument(
        "--policy",
        help="Build images for a specific policy only (default: build all policies)",
    )
    build_parser.add_argument(
        "--no-cache", action="store_true", help="Disable Docker cache during build"
    )
    build_parser.add_argument(
        "--compat",
        action="store_true",
        help="Use legacy gen_config.json/policies format (deprecated)",
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
        help="Name of the experiment (directory name in exp/<app>/in/)",
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
    build_dryrun_parser.add_argument(
        "--compat",
        action="store_true",
        help="Use legacy gen_config.json/policies format (deprecated)",
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
    plot_parser.add_argument(
        "--compat",
        action="store_true",
        help="Use legacy gen_config.json/policies format (deprecated)",
    )
    plot_parser.set_defaults(func=cmd_plot)

    # plot-replicas command
    plot_replicas_parser = subparsers.add_parser(
        "plot-replicas",
        help="Generate replica plots from hotel experiment inputs",
        description="Scan exp/<app>/in for <policy>_<rps> directories and plot replicas",
    )
    plot_replicas_parser.add_argument(
        "app", choices=["hotel"], help="Application name (hotel only)"
    )
    plot_replicas_parser.set_defaults(func=cmd_plot_replicas)

    migrate_parser = subparsers.add_parser(
        "migrate-config",
        help="Migrate legacy experiment input to v2 experiment.yaml format",
        description=(
            "Read legacy exp/<app>/in/<src>/gen_config.json + policies and write "
            "v2 exp/<app>/in/<dst>/experiment.yaml"
        ),
    )
    migrate_parser.add_argument(
        "app",
        choices=["hotel", "mssim", "socialnet", "synthetic"],
        help="Application name",
    )
    migrate_parser.add_argument(
        "source_experiment",
        help="Source legacy experiment directory name under exp/<app>/in/",
    )
    migrate_parser.add_argument(
        "target_experiment",
        help="Destination experiment directory name under exp/<app>/in/",
    )
    migrate_parser.add_argument(
        "--force",
        action="store_true",
        help="Overwrite destination if it already exists",
    )
    migrate_parser.set_defaults(func=cmd_migrate_config)

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
