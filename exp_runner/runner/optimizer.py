"""
Rajomon parameter optimizer using Optuna-based Bayesian optimization.

Finds good Rajomon admission control parameters for each app by running
short experiments and optimizing the objective:
    goodput - penalty_weight * max(0, p99_latency - SLO)
"""

import copy
import csv
import json
import logging
import shutil
import subprocess
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Optional

import optuna

from .apps import get_app_plugin
from .config import ExperimentConfig
from .experiment import Experiment
from .plotting.goodput import compute_goodput
from .plotting.util import _read_request_csv, filter_excluded_errors

logger = logging.getLogger(__name__)

# Default Rajomon parameter values (from libs/masa-policy/src/policy_params.rs).
# Only parameters actually used by rajomon.rs are included.
DEFAULT_RAJOMON_PARAMS: dict = {
    "latency_threshold_us": 5_000,
    "price_update_rate_ms": 10,
    "price_step_up": 8,
    "init_price": 0,
    "price_freq": 3,
    "tokens_left_init": 100,
    "token_update_rate_ms": 10,
    "token_update_step": 100,
}

# Keys that the optimizer tunes (the rest are fixed).
OPTIMIZED_KEYS = {
    "latency_threshold_us",
    "price_step_up",
    "token_update_step",
    "init_price",
    "token_update_rate_ms",
    "tokens_left_init",
}

# RPS multipliers for the optimization sweep
RPS_MULTIPLIERS = [0.8, 1.0, 1.2, 1.5, 2.0]


def generate_rps_sweep(saturation_rps: int) -> list[int]:
    """Generate RPS values for the optimization sweep.

    Returns [0.8x, 1.0x, 1.2x, 1.5x, 2.0x] of saturation_rps,
    each rounded to the nearest 100.
    """
    return [int(round(saturation_rps * m / 100) * 100) for m in RPS_MULTIPLIERS]


def sample_params(trial: optuna.Trial) -> dict:
    """Sample Rajomon parameters from an Optuna trial.

    Optimized (6): latency_threshold_us, price_step_up, token_update_step,
    init_price, token_update_rate_ms, tokens_left_init.
    Fixed: price_update_rate_ms, price_freq.
    """
    latency_threshold_us = trial.suggest_int(
        "latency_threshold_us", 2000, 40000, log=True
    )
    price_step_up = trial.suggest_int("price_step_up", 1, 20)
    token_update_step = trial.suggest_int("token_update_step", 10, 500000, log=True)
    init_price = trial.suggest_int("init_price", 0, 1000)
    token_update_rate_ms = trial.suggest_int("token_update_rate_ms", 1, 20, log=True)
    tokens_left_init = trial.suggest_int("tokens_left_init", 10, 500000, log=True)

    return {
        "latency_threshold_us": latency_threshold_us,
        "price_update_rate_ms": 10,  # fixed
        "price_step_up": price_step_up,
        "init_price": init_price,
        "price_freq": 3,  # fixed
        "tokens_left_init": tokens_left_init,
        "token_update_rate_ms": token_update_rate_ms,
        "token_update_step": token_update_step,
    }


def compute_objective(
    out_dir: Path,
    policy: str,
    gen_config: dict,
    penalty_weight: float = 10.0,
) -> float:
    """Compute the optimization objective from experiment results.

    Objective: sum(goodput across APIs and RPS) - penalty * worst_p99_violation

    Args:
        out_dir: Experiment output directory (contains 0/<policy>/r<RPS>_<API>.csv)
        policy: Policy string (feature flags)
        gen_config: gen_config dict with Apis, Slos, Rps keys
        penalty_weight: Multiplier for worst p99 SLO violation (in seconds)

    Returns:
        Objective value (higher is better). Large negative on failure.
    """
    apis = gen_config["Apis"]
    slos = gen_config["Slos"]
    rps_values = gen_config["Rps"]

    api_to_slo = dict(zip(apis, slos))

    total_goodput = 0.0
    worst_p99_violation_us = 0.0  # microseconds
    any_data = False

    policy_dir = out_dir / "0" / policy

    for rps in rps_values:
        for api in apis:
            csv_path = policy_dir / f"r{rps}_{api}.csv"
            if not csv_path.exists():
                logger.warning(f"Missing CSV: {csv_path}")
                continue

            df = _read_request_csv(str(csv_path))
            if df.empty:
                continue

            # Set SLO from gen_config
            slo_us = api_to_slo[api]
            df["slo"] = slo_us

            # Compute goodput for this (RPS, API) pair
            gp = compute_goodput(df)
            total_goodput += gp
            any_data = True

            # Compute p99 latency violation
            df_filtered = filter_excluded_errors(df)
            if not df_filtered.empty and "latency" in df_filtered.columns:
                p99 = df_filtered["latency"].quantile(0.99)
                violation = max(0.0, p99 - slo_us)
                worst_p99_violation_us = max(worst_p99_violation_us, violation)

    if not any_data:
        logger.warning(f"No data found in {policy_dir}")
        return -1e9

    # Convert violation from microseconds to seconds for penalty
    worst_p99_violation_sec = worst_p99_violation_us / 1e6
    objective = total_goodput - penalty_weight * worst_p99_violation_sec
    return objective


@dataclass
class OptimizerConfig:
    """Configuration for the Rajomon parameter optimizer."""

    app: str
    experiment_base: str  # Existing experiment to copy topology from
    policy: str  # e.g., "sched_slo,ac_rajomon,abort_slo"
    saturation_rps: int
    n_iterations: int = 30
    penalty_weight: float = 10.0
    warm_start_path: Optional[Path] = None  # Previous best_params.json
    warmup_secs: int = 15
    duration_secs: int = 30
    output_path: Optional[Path] = None  # Where to write best params


class RajomonOptimizer:
    """Bayesian optimizer for Rajomon admission control parameters.

    Uses Optuna's TPE sampler with SQLite persistence for resumability.
    Each iteration runs a short experiment with the sampled parameters.
    """

    def __init__(self, config: OptimizerConfig, repo_root: Path):
        self.config = config
        self.repo_root = repo_root

        # Output paths
        self.opt_out_dir = repo_root / "exp" / config.app / "out" / "_opt_rajomon"
        self.opt_out_dir.mkdir(parents=True, exist_ok=True)

        self.output_path = config.output_path or (self.opt_out_dir / "best_params.json")
        self.log_csv_path = self.opt_out_dir / "optimization_log.csv"
        self.db_path = self.opt_out_dir / "optuna.db"

        # RPS sweep (fixed across iterations)
        self.rps_sweep = generate_rps_sweep(config.saturation_rps)

        # Load the base experiment config to get APIs, SLOs, app config
        self.app_plugin = get_app_plugin(config.app)
        self.base_config = ExperimentConfig.load(
            experiment_name=config.experiment_base,
            app_name=config.app,
            repo_root=repo_root,
            app_plugin=self.app_plugin,
        )

    def _build_gen_config(self) -> dict:
        """Build the gen_config for optimization experiments."""
        gen_config = copy.deepcopy(self.base_config.gen_config)
        gen_config["Rps"] = self.rps_sweep
        gen_config["Repeats"] = 1
        gen_config["WarmupSecs"] = self.config.warmup_secs
        gen_config["DurationSecs"] = self.config.duration_secs
        return gen_config

    def _create_experiment_dir(self, iteration: int, params: dict) -> Path:
        """Create a temporary experiment directory for one iteration."""
        exp_name = f"_opt_rajomon_{iteration}"
        in_dir = self.repo_root / "exp" / self.config.app / "in" / exp_name

        # Clean up any existing directory
        if in_dir.exists():
            import shutil

            shutil.rmtree(in_dir)
        in_dir.mkdir(parents=True, exist_ok=True)

        # Write gen_config.json
        gen_config = self._build_gen_config()
        with (in_dir / "gen_config.json").open("w") as f:
            json.dump(gen_config, f, indent=2)

        # Write policies file (single policy)
        with (in_dir / "policies").open("w") as f:
            f.write(f"{self.config.policy}\n")

        # Write policy_param.json (flat format — single policy, no nesting)
        policy_params = {"rajomon": params}
        with (in_dir / "policy_param.json").open("w") as f:
            json.dump(policy_params, f, indent=2)

        # Copy app-specific config from base experiment
        docker_config = self.app_plugin.get_docker_config()
        if docker_config.app_config_filename:
            src = self.base_config.in_dir / docker_config.app_config_filename
            if src.exists():
                import shutil

                shutil.copy2(src, in_dir / docker_config.app_config_filename)

        return in_dir

    def _cleanup_stale_docker(self) -> None:
        """Remove stale Docker containers and networks from previous optimizer runs."""
        app_prefix = self.app_plugin.get_app_name()
        pattern = f"{app_prefix}-opt-rajomon"
        try:
            # Find and remove stale containers
            result = subprocess.run(
                ["docker", "ps", "-aq", "--filter", f"name={pattern}"],
                capture_output=True,
                text=True,
                check=False,
            )
            container_ids = result.stdout.strip().split()
            if container_ids and container_ids[0]:
                logger.info(f"Removing {len(container_ids)} stale optimizer containers")
                subprocess.run(
                    ["docker", "rm", "-f", *container_ids],
                    capture_output=True,
                    check=False,
                )

            # Remove stale networks
            result = subprocess.run(
                [
                    "docker",
                    "network",
                    "ls",
                    "--filter",
                    f"name={pattern}",
                    "-q",
                ],
                capture_output=True,
                text=True,
                check=False,
            )
            network_ids = result.stdout.strip().split()
            if network_ids and network_ids[0]:
                logger.info(f"Removing {len(network_ids)} stale optimizer networks")
                for nid in network_ids:
                    subprocess.run(
                        ["docker", "network", "rm", nid],
                        capture_output=True,
                        check=False,
                    )
        except Exception as e:
            logger.warning(f"Docker cleanup failed (non-fatal): {e}")

    def _cleanup_trial_dirs(self, exp_name: str, out_dir: Path) -> None:
        """Remove trial input/output directories to save disk space."""
        try:
            # Remove output directory
            if out_dir.exists():
                shutil.rmtree(out_dir)
            # Remove input directory
            in_dir = self.repo_root / "exp" / self.config.app / "in" / exp_name
            if in_dir.exists():
                shutil.rmtree(in_dir)
        except Exception as e:
            logger.warning(f"Trial directory cleanup failed (non-fatal): {e}")

    def _run_experiment(self, iteration: int, params: dict) -> float:
        """Run one experiment iteration and return the objective value."""
        self._cleanup_stale_docker()

        exp_name = f"_opt_rajomon_{iteration}"
        self._create_experiment_dir(iteration, params)

        # Load the experiment config
        config = ExperimentConfig.load(
            experiment_name=exp_name,
            app_name=self.config.app,
            repo_root=self.repo_root,
            app_plugin=self.app_plugin,
        )

        # Run the experiment (plots enabled so per-trial artifacts are inspectable)
        experiment = Experiment(
            app=self.app_plugin,
            config=config,
            repo_root=self.repo_root,
            plot=True,
            no_cache=False,
            rm_data=True,
            dry_run=False,
            smoke_test=False,
        )

        try:
            experiment.run()
        except Exception as e:
            logger.error(f"Experiment failed for iteration {iteration}: {e}")
            return -1e9

        # Compute objective
        gen_config = self._build_gen_config()
        objective = compute_objective(
            config.out_dir,
            self.config.policy,
            gen_config,
            self.config.penalty_weight,
        )

        return objective

    def _log_trial(
        self, trial_number: int, params: dict, objective: float, is_best: bool
    ) -> None:
        """Append a trial to the optimization log CSV."""
        write_header = not self.log_csv_path.exists()

        fieldnames = ["trial", "objective", "is_best"] + sorted(params.keys())
        row = {"trial": trial_number, "objective": objective, "is_best": is_best}
        row.update(params)

        with self.log_csv_path.open("a", newline="") as f:
            writer = csv.DictWriter(f, fieldnames=fieldnames)
            if write_header:
                writer.writeheader()
            writer.writerow(row)

    def run(self) -> dict:
        """Run the full optimization loop.

        Returns:
            Best parameters dict (flat RajomonParams format).
        """
        storage = f"sqlite:///{self.db_path}"

        study = optuna.create_study(
            study_name="rajomon_optimization",
            storage=storage,
            load_if_exists=True,
            direction="maximize",
            # n_startup_trials=2 so TPE switches to history-aware sampling
            # after the seeded default + warm-start. Default is 10, which meant
            # the first 10 draws were deterministic from the seed — on resume,
            # the sampler's RNG reset caused those early random draws to repeat
            # exactly (trials 2/3 getting re-sampled as trials 7/8, etc.).
            sampler=optuna.samplers.TPESampler(seed=42, n_startup_trials=2),
        )

        # Only seed the queue on a fresh study. On resume, re-enqueueing would
        # waste trials re-running already-evaluated default/warm-start points
        # (Optuna persists the queue in SQLite across invocations).
        if len(study.trials) == 0:
            study.enqueue_trial(
                {k: v for k, v in DEFAULT_RAJOMON_PARAMS.items() if k in OPTIMIZED_KEYS}
            )

            if self.config.warm_start_path:
                try:
                    with open(self.config.warm_start_path) as f:
                        warm_params = json.load(f)
                    if "rajomon" in warm_params:
                        warm_params = warm_params["rajomon"]
                    enqueue_params = {
                        k: v for k, v in warm_params.items() if k in OPTIMIZED_KEYS
                    }
                    if enqueue_params:
                        study.enqueue_trial(enqueue_params)
                        logger.info(
                            f"Enqueued warm-start params from {self.config.warm_start_path}"
                        )
                except Exception as e:
                    logger.warning(f"Failed to load warm-start params: {e}")

        # Print startup summary
        logger.info(f"{'=' * 60}")
        logger.info("Rajomon Parameter Optimizer")
        logger.info(f"{'=' * 60}")
        logger.info(f"  App:            {self.config.app}")
        logger.info(f"  Base experiment: {self.config.experiment_base}")
        logger.info(f"  Policy:         {self.config.policy}")
        logger.info(f"  Saturation RPS: {self.config.saturation_rps}")
        logger.info(f"  RPS sweep:      {self.rps_sweep}")
        logger.info(f"  Iterations:     {self.config.n_iterations}")
        logger.info(f"  Penalty weight: {self.config.penalty_weight}")
        logger.info(
            f"  Per-trial time: ~{self.config.warmup_secs + self.config.duration_secs}s "
            f"({self.config.warmup_secs}s warmup + {self.config.duration_secs}s run)"
        )
        logger.info(f"  Output:         {self.output_path}")
        logger.info(f"  Optuna DB:      {self.db_path}")
        if self.config.warm_start_path:
            logger.info(f"  Warm start:     {self.config.warm_start_path}")
        logger.info(f"{'=' * 60}")

        # Build Docker images once (policy doesn't change between iterations)
        logger.info("Building Docker images for optimization (one-time)...")
        build_start = time.monotonic()
        self._build_images_once()
        build_elapsed = time.monotonic() - build_start
        logger.info(f"Docker images built in {build_elapsed:.1f}s")

        best_objective = -float("inf")
        best_params: dict = {}

        # Only count finished trials toward the budget. study.trials includes
        # WAITING (queued-but-not-popped) entries, so using len() would charge
        # the iteration budget for trials that haven't run yet.
        completed = len(
            study.get_trials(
                deepcopy=False,
                states=(
                    optuna.trial.TrialState.COMPLETE,
                    optuna.trial.TrialState.FAIL,
                    optuna.trial.TrialState.PRUNED,
                ),
            )
        )
        remaining = max(0, self.config.n_iterations - completed)

        if completed > 0:
            logger.info(
                f"Resuming from {completed} completed trials, {remaining} remaining"
            )
            try:
                bt = study.best_trial
                best_objective = bt.value
                best_params = sample_params_from_values(bt.params)
                logger.info(f"Previous best objective: {best_objective:.4f}")
            except ValueError:
                logger.info("No successful trials yet in previous runs")

        optimization_start = time.monotonic()

        for i in range(remaining):
            trial_num = completed + i
            trial_start = time.monotonic()

            logger.info(f"{'=' * 60}")
            logger.info(f"[{trial_num + 1}/{self.config.n_iterations}] Starting trial")

            trial = study.ask()
            params = sample_params(trial)

            # Log key params on one line for quick scanning
            logger.info(
                f"  Params: threshold={params['latency_threshold_us']}us, "
                f"step_up={params['price_step_up']}, "
                f"tok_step={params['token_update_step']}, "
                f"init_price={params['init_price']}, "
                f"tok_rate={params['token_update_rate_ms']}ms, "
                f"tok_init={params['tokens_left_init']}"
            )

            objective = self._run_experiment(trial_num, params)

            study.tell(trial, objective)

            trial_elapsed = time.monotonic() - trial_start
            is_best = objective > best_objective
            if is_best:
                best_objective = objective
                best_params = params
                logger.info(
                    f"  Result: {objective:.4f} ** NEW BEST ** ({trial_elapsed:.1f}s)"
                )
            else:
                logger.info(
                    f"  Result: {objective:.4f} (best so far: {best_objective:.4f}) ({trial_elapsed:.1f}s)"
                )

            self._log_trial(trial_num, params, objective, is_best)

            # Estimate time remaining
            elapsed_total = time.monotonic() - optimization_start
            avg_per_trial = elapsed_total / (i + 1)
            trials_left = remaining - (i + 1)
            eta_mins = (avg_per_trial * trials_left) / 60
            if trials_left > 0:
                logger.info(f"  ETA: ~{eta_mins:.1f} min ({trials_left} trials left)")

        total_elapsed = time.monotonic() - optimization_start
        logger.info(f"{'=' * 60}")
        logger.info(f"Optimization complete in {total_elapsed / 60:.1f} minutes")

        # Write best params
        if best_params:
            self.output_path.parent.mkdir(parents=True, exist_ok=True)
            with self.output_path.open("w") as f:
                json.dump(best_params, f, indent=2)
            logger.info(f"Best objective: {best_objective:.4f}")
            logger.info(f"Best params written to {self.output_path}")
            logger.info(f"Trial log written to {self.log_csv_path}")
            logger.info(f"Best params: {json.dumps(best_params, indent=2)}")
        else:
            logger.warning("No successful trials completed")

        return best_params

    def _build_images_once(self) -> None:
        """Build Docker images for the policy (only needed once)."""
        builder = self.app_plugin.create_builder()
        gen_config_path = self.base_config.in_dir / "gen_config.json"

        builder.build(
            repo_root=self.repo_root,
            app_dir=self.base_config.app_dir,
            features=self.config.policy,
            rust_log="info",
            no_cache=False,
            gen_config_path=gen_config_path,
            dry_run=False,
        )


def sample_params_from_values(values: dict) -> dict:
    """Reconstruct full params dict from Optuna trial values (optimized keys only)."""
    return {
        "latency_threshold_us": values.get(
            "latency_threshold_us", DEFAULT_RAJOMON_PARAMS["latency_threshold_us"]
        ),
        "price_update_rate_ms": 10,
        "price_step_up": values.get(
            "price_step_up", DEFAULT_RAJOMON_PARAMS["price_step_up"]
        ),
        "init_price": values.get("init_price", DEFAULT_RAJOMON_PARAMS["init_price"]),
        "price_freq": 3,
        "tokens_left_init": values.get(
            "tokens_left_init", DEFAULT_RAJOMON_PARAMS["tokens_left_init"]
        ),
        "token_update_rate_ms": values.get(
            "token_update_rate_ms", DEFAULT_RAJOMON_PARAMS["token_update_rate_ms"]
        ),
        "token_update_step": values.get(
            "token_update_step", DEFAULT_RAJOMON_PARAMS["token_update_step"]
        ),
    }
