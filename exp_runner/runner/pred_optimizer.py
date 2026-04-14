"""
Predictive admission control parameter optimizer using Optuna-based Bayesian optimization.

Tunes PredParams for the sched_pred,ac_pred,abort_slack,est_mean_var policy combination.

Objective: maximize goodput while penalizing intra-run goodput oscillation.

Three objective modes are supported (select via --objective-mode):

  sharpe (default)
    score_rps = mean_gp / (1 + stability_weight * CV)
    where CV = std_gp / (mean_gp + ε).

    This is a Sharpe-ratio: numerator is throughput, denominator rises with
    relative oscillation. stability_weight=0 reduces to pure goodput.
    Smooth for Bayesian optimization — good default.

  floor
    score_rps = percentile(windowed_gp, q)   (default q=10)

    Directly optimizes near-worst-case goodput. No tunable weight —
    the objective naturally captures stability because a high floor
    requires low variance. Less smooth than "sharpe" so may need more
    iterations for TPE to converge.

  penalized
    score_rps = mean_gp - stability_weight * std_gp

    Both terms are in req/s so the weight has a concrete interpretation:
    1 unit of stability_weight trades 1 req/s of std for 1 req/s of mean.
    Easiest to reason about but sensitive to the absolute scale of
    goodput (needs stability_weight calibrated to actual req/s range).

For all modes, if a window has fewer than 2 samples (very short duration or
very low RPS), the score falls back to aggregate goodput with no stability
penalty, so low-load points do not distort the objective.
"""

import copy
import csv
import json
import logging
import shutil
import subprocess
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Optional

import numpy as np
import optuna

from .apps import get_app_plugin
from .config import ExperimentConfig
from .experiment import Experiment
from .plotting.goodput import compute_goodput
from .plotting.util import _read_request_csv, filter_excluded_errors

logger = logging.getLogger(__name__)

# Default PredParams (code defaults — good starting point for warm-start)
DEFAULT_PRED_PARAMS: dict = {
    "tau_er": 2.0,
    "estimator_k": 0.0,
    "aimd_alpha": 0.05,
    "aimd_beta": 0.875,
    "aimd_er_threshold": 0.10,
}

# ember_10 winner — empirically validated against hotel benchmark
EMBER_WINNER_PARAMS: dict = {
    "tau_er": 2.0,
    "estimator_k": 0.0,
    "aimd_alpha": 0.05,
    "aimd_beta": 0.875,
    "aimd_er_threshold": 0.40,
}


# ── Parameter sampling ────────────────────────────────────────────────────────


def sample_params(trial: optuna.Trial) -> dict:
    """Sample PredParams from an Optuna trial.

    Search space:
      tau_er            log-uniform [0.5, 20.0]  — idle decay time constant
      estimator_k       uniform     [0.0, 3.0]   — abort_slack variance multiplier
      aimd_alpha        log-uniform [0.005, 0.5] — additive recovery per healthy window
      aimd_beta         uniform     [0.50, 0.98] — multiplicative decrease factor
      aimd_er_threshold uniform     [0.10, 0.80] — overload threshold (ER fraction)
    """
    return {
        "tau_er": trial.suggest_float("tau_er", 0.5, 20.0, log=True),
        "estimator_k": trial.suggest_float("estimator_k", 0.0, 3.0),
        "aimd_alpha": trial.suggest_float("aimd_alpha", 0.005, 0.5, log=True),
        "aimd_beta": trial.suggest_float("aimd_beta", 0.50, 0.98),
        "aimd_er_threshold": trial.suggest_float("aimd_er_threshold", 0.10, 0.80),
    }


def sample_params_from_values(values: dict) -> dict:
    """Reconstruct full params dict from Optuna trial parameter values."""
    return {
        "tau_er": values.get("tau_er", DEFAULT_PRED_PARAMS["tau_er"]),
        "estimator_k": values.get("estimator_k", DEFAULT_PRED_PARAMS["estimator_k"]),
        "aimd_alpha": values.get("aimd_alpha", DEFAULT_PRED_PARAMS["aimd_alpha"]),
        "aimd_beta": values.get("aimd_beta", DEFAULT_PRED_PARAMS["aimd_beta"]),
        "aimd_er_threshold": values.get(
            "aimd_er_threshold", DEFAULT_PRED_PARAMS["aimd_er_threshold"]
        ),
    }


# ── Windowed goodput ──────────────────────────────────────────────────────────


def compute_windowed_goodput(
    df: "pd.DataFrame", window_secs: float = 5.0
) -> list[float]:
    """Split a request CSV into time windows and return per-window goodput (req/s).

    Uses start_at (microseconds) to assign requests to windows.  Only requests
    that meet their SLO and are not excluded errors count toward goodput.

    Returns an empty list if the run is shorter than one window.
    """
    import pandas as pd

    df_filtered = filter_excluded_errors(df)
    if df_filtered.empty:
        return []

    df_filtered = df_filtered.copy()
    df_filtered["met_slo"] = df_filtered["latency"] <= df_filtered["slo"]

    # Relative time in microseconds (start_at is already in µs)
    t0 = df["start_at"].min()
    df_filtered["rel_us"] = df_filtered["start_at"] - t0

    window_us = window_secs * 1_000_000
    t_max = float(df_filtered["rel_us"].max())

    if t_max < window_us:
        return []  # too short for even one complete window

    windows: list[float] = []
    t = 0.0
    while t + window_us <= t_max:
        mask = (df_filtered["rel_us"] >= t) & (df_filtered["rel_us"] < t + window_us)
        window_gp = float(df_filtered.loc[mask, "met_slo"].sum()) / window_secs
        windows.append(window_gp)
        t += window_us

    return windows


# ── Objective function ────────────────────────────────────────────────────────


def _score_rps(
    windows: list[float],
    fallback_gp: float,
    objective_mode: str,
    stability_weight: float,
    floor_percentile: float,
) -> float:
    """Compute a single (RPS, API) score from a windowed goodput series."""
    if len(windows) < 2:
        # Not enough windows for a stability estimate — use aggregate goodput.
        return fallback_gp

    arr = np.asarray(windows, dtype=float)
    mean_gp = float(arr.mean())
    std_gp = float(arr.std())

    if objective_mode == "sharpe":
        eps = 1e-6
        cv = std_gp / (mean_gp + eps)
        return mean_gp / (1.0 + stability_weight * cv)

    elif objective_mode == "floor":
        return float(np.percentile(arr, floor_percentile))

    elif objective_mode == "penalized":
        return mean_gp - stability_weight * std_gp

    else:
        raise ValueError(f"Unknown objective_mode: {objective_mode!r}")


def compute_pred_objective(
    out_dir: Path,
    policy: str,
    gen_config: dict,
    objective_mode: str = "sharpe",
    stability_weight: float = 0.3,
    window_secs: float = 5.0,
    floor_percentile: float = 10.0,
) -> float:
    """Compute the optimization objective from one experiment's output.

    For each (RPS, API) pair:
      1. Load the raw request CSV.
      2. Compute windowed goodput (stability-aware) and aggregate goodput (fallback).
      3. Score via the selected objective mode.

    Returns the sum of scores across all (RPS, API) pairs (higher is better).
    Returns -1e9 on failure.

    Args:
        out_dir:           Experiment output directory (contains 0/<policy>/r<RPS>_<API>.csv).
        policy:            Policy feature-flag string.
        gen_config:        gen_config dict with Apis, Slos, Rps keys.
        objective_mode:    "sharpe" | "floor" | "penalized" (see module docstring).
        stability_weight:  Stability penalty weight (used by "sharpe" and "penalized").
        window_secs:       Width of each goodput measurement window in seconds.
        floor_percentile:  Percentile used by "floor" mode (default 10th).
    """
    apis = gen_config["Apis"]
    slos = gen_config["Slos"]
    rps_values = gen_config["Rps"]
    api_to_slo = dict(zip(apis, slos))

    total_score = 0.0
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

            df["slo"] = api_to_slo[api]
            any_data = True

            # Aggregate goodput — used as fallback when windows are too few
            fallback_gp = compute_goodput(df)

            # Windowed goodput time series
            windows = compute_windowed_goodput(df, window_secs=window_secs)

            score = _score_rps(
                windows=windows,
                fallback_gp=fallback_gp,
                objective_mode=objective_mode,
                stability_weight=stability_weight,
                floor_percentile=floor_percentile,
            )
            total_score += score

    if not any_data:
        logger.warning(f"No data found in {policy_dir}")
        return -1e9

    return total_score


# ── Optimizer ─────────────────────────────────────────────────────────────────


@dataclass
class PredOptimizerConfig:
    """Configuration for the predictive-admission parameter optimizer."""

    app: str
    experiment_base: str  # Existing experiment to copy topology from
    policy: str  # e.g., "sched_pred,ac_pred,abort_slack,est_mean_var"
    saturation_rps: int
    n_iterations: int = 30
    objective_mode: str = "sharpe"  # "sharpe" | "floor" | "penalized"
    stability_weight: float = 0.3
    window_secs: float = 5.0
    floor_percentile: float = 10.0
    warm_start_path: Optional[Path] = None  # Previous best_params.json
    warmup_secs: int = 15
    duration_secs: int = 45  # Longer than Rajomon — need stability signal
    output_path: Optional[Path] = None


# RPS multipliers for the optimization sweep
RPS_MULTIPLIERS = [0.8, 1.0, 1.2, 1.5, 2.0]


def generate_rps_sweep(saturation_rps: int) -> list[int]:
    """Generate RPS values: [0.8x, 1.0x, 1.2x, 1.5x, 2.0x] of saturation_rps."""
    return [int(round(saturation_rps * m / 100) * 100) for m in RPS_MULTIPLIERS]


class PredOptimizer:
    """Bayesian optimizer for PredParams (ac_pred / abort_slack / est_mean_var).

    Uses Optuna's TPE sampler with SQLite persistence for resumability.
    Each iteration runs one short experiment and evaluates the stability-aware
    objective function.

    The optimizer warm-starts with three anchor points:
      1. Code defaults (tau_er=2, alpha=0.05, beta=0.875, threshold=0.10)
      2. Ember-winner (tau_er=2, alpha=0.05, beta=0.875, threshold=0.40)
      3. Any user-supplied warm_start_path (best result from a prior run)
    """

    def __init__(self, config: PredOptimizerConfig, repo_root: Path):
        self.config = config
        self.repo_root = repo_root

        self.opt_out_dir = repo_root / "exp" / config.app / "out" / "_opt_pred"
        self.opt_out_dir.mkdir(parents=True, exist_ok=True)

        self.output_path = config.output_path or (self.opt_out_dir / "best_params.json")
        self.log_csv_path = self.opt_out_dir / "optimization_log.csv"
        self.db_path = self.opt_out_dir / "optuna.db"

        self.rps_sweep = generate_rps_sweep(config.saturation_rps)

        self.app_plugin = get_app_plugin(config.app)
        self.base_config = ExperimentConfig.load(
            experiment_name=config.experiment_base,
            app_name=config.app,
            repo_root=repo_root,
            app_plugin=self.app_plugin,
        )

    def _build_gen_config(self) -> dict:
        gen_config = copy.deepcopy(self.base_config.gen_config)
        gen_config["Rps"] = self.rps_sweep
        gen_config["Repeats"] = 1
        gen_config["WarmupSecs"] = self.config.warmup_secs
        gen_config["DurationSecs"] = self.config.duration_secs
        return gen_config

    def _create_experiment_dir(self, iteration: int, params: dict) -> Path:
        exp_name = f"_opt_pred_{iteration}"
        in_dir = self.repo_root / "exp" / self.config.app / "in" / exp_name
        if in_dir.exists():
            shutil.rmtree(in_dir)
        in_dir.mkdir(parents=True, exist_ok=True)

        gen_config = self._build_gen_config()
        with (in_dir / "gen_config.json").open("w") as f:
            json.dump(gen_config, f, indent=2)

        with (in_dir / "policies").open("w") as f:
            f.write(f"{self.config.policy}\n")

        # PredParams live under the "pred" key in policy_param.json
        policy_params = {"pred": params}
        with (in_dir / "policy_param.json").open("w") as f:
            json.dump(policy_params, f, indent=2)

        # Copy app-specific config from base experiment
        docker_config = self.app_plugin.get_docker_config()
        if docker_config.app_config_filename:
            src = self.base_config.in_dir / docker_config.app_config_filename
            if src.exists():
                shutil.copy2(src, in_dir / docker_config.app_config_filename)

        return in_dir

    def _cleanup_stale_docker(self) -> None:
        app_prefix = self.app_plugin.get_app_name()
        pattern = f"{app_prefix}-opt-pred"
        try:
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

            result = subprocess.run(
                ["docker", "network", "ls", "--filter", f"name={pattern}", "-q"],
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
        try:
            if out_dir.exists():
                shutil.rmtree(out_dir)
            in_dir = self.repo_root / "exp" / self.config.app / "in" / exp_name
            if in_dir.exists():
                shutil.rmtree(in_dir)
        except Exception as e:
            logger.warning(f"Trial directory cleanup failed (non-fatal): {e}")

    def _run_experiment(self, iteration: int, params: dict) -> float:
        self._cleanup_stale_docker()

        exp_name = f"_opt_pred_{iteration}"
        self._create_experiment_dir(iteration, params)

        config = ExperimentConfig.load(
            experiment_name=exp_name,
            app_name=self.config.app,
            repo_root=self.repo_root,
            app_plugin=self.app_plugin,
        )

        experiment = Experiment(
            app=self.app_plugin,
            config=config,
            repo_root=self.repo_root,
            plot=False,
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

        gen_config = self._build_gen_config()
        objective = compute_pred_objective(
            out_dir=config.out_dir,
            policy=self.config.policy,
            gen_config=gen_config,
            objective_mode=self.config.objective_mode,
            stability_weight=self.config.stability_weight,
            window_secs=self.config.window_secs,
            floor_percentile=self.config.floor_percentile,
        )

        self._cleanup_trial_dirs(exp_name, config.out_dir)
        return objective

    def _log_trial(
        self, trial_number: int, params: dict, objective: float, is_best: bool
    ) -> None:
        write_header = not self.log_csv_path.exists()
        fieldnames = ["trial", "objective", "is_best"] + sorted(params.keys())
        row = {"trial": trial_number, "objective": objective, "is_best": is_best}
        row.update(params)
        with self.log_csv_path.open("a", newline="") as f:
            writer = csv.DictWriter(f, fieldnames=fieldnames)
            if write_header:
                writer.writeheader()
            writer.writerow(row)

    def _build_images_once(self) -> None:
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

    def run(self) -> dict:
        """Run the full optimization loop.  Returns the best PredParams dict."""
        storage = f"sqlite:///{self.db_path}"

        study = optuna.create_study(
            study_name="pred_optimization",
            storage=storage,
            load_if_exists=True,
            direction="maximize",
            sampler=optuna.samplers.TPESampler(seed=42),
        )

        # Anchor 1: code defaults
        study.enqueue_trial(DEFAULT_PRED_PARAMS)
        # Anchor 2: ember_10 winner
        study.enqueue_trial(EMBER_WINNER_PARAMS)

        # Anchor 3: user warm-start
        if self.config.warm_start_path:
            try:
                with open(self.config.warm_start_path) as f:
                    warm = json.load(f)
                if "pred" in warm:
                    warm = warm["pred"]
                warm_filtered = {
                    k: v for k, v in warm.items() if k in DEFAULT_PRED_PARAMS
                }
                if warm_filtered:
                    study.enqueue_trial(warm_filtered)
                    logger.info(
                        f"Enqueued warm-start from {self.config.warm_start_path}"
                    )
            except Exception as e:
                logger.warning(f"Failed to load warm-start params: {e}")

        # Print startup summary
        logger.info(f"{'=' * 60}")
        logger.info("Predictive Admission Control Parameter Optimizer")
        logger.info(f"{'=' * 60}")
        logger.info(f"  App:             {self.config.app}")
        logger.info(f"  Base experiment: {self.config.experiment_base}")
        logger.info(f"  Policy:          {self.config.policy}")
        logger.info(f"  Saturation RPS:  {self.config.saturation_rps}")
        logger.info(f"  RPS sweep:       {self.rps_sweep}")
        logger.info(f"  Iterations:      {self.config.n_iterations}")
        logger.info(f"  Objective mode:  {self.config.objective_mode}")
        logger.info(f"  Stability weight:{self.config.stability_weight}")
        logger.info(f"  Window size:     {self.config.window_secs}s")
        logger.info(
            f"  Per-trial time:  ~{self.config.warmup_secs + self.config.duration_secs}s "
            f"({self.config.warmup_secs}s warmup + {self.config.duration_secs}s run)"
        )
        logger.info(f"  Output:          {self.output_path}")
        logger.info(f"  Optuna DB:       {self.db_path}")
        logger.info(f"{'=' * 60}")

        logger.info("Building Docker images (one-time)...")
        build_start = time.monotonic()
        self._build_images_once()
        logger.info(f"Docker images built in {time.monotonic() - build_start:.1f}s")

        best_objective = -float("inf")
        best_params: dict = {}

        completed = len(study.trials)
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

            logger.info(
                f"  Params: threshold={params['aimd_er_threshold']:.3f} "
                f"alpha={params['aimd_alpha']:.4f} "
                f"beta={params['aimd_beta']:.3f} "
                f"tau_er={params['tau_er']:.2f} "
                f"estimator_k={params['estimator_k']:.2f}"
            )

            objective = self._run_experiment(trial_num, params)
            study.tell(trial, objective)

            trial_elapsed = time.monotonic() - trial_start
            is_best = objective > best_objective
            if is_best:
                best_objective = objective
                best_params = params
                logger.info(
                    f"  Score: {objective:.4f} ** NEW BEST ** ({trial_elapsed:.1f}s)"
                )
            else:
                logger.info(
                    f"  Score: {objective:.4f} (best: {best_objective:.4f}) ({trial_elapsed:.1f}s)"
                )

            self._log_trial(trial_num, params, objective, is_best)

            elapsed_total = time.monotonic() - optimization_start
            avg_per_trial = elapsed_total / (i + 1)
            trials_left = remaining - (i + 1)
            if trials_left > 0:
                eta_mins = (avg_per_trial * trials_left) / 60
                logger.info(f"  ETA: ~{eta_mins:.1f} min ({trials_left} trials left)")

        total_elapsed = time.monotonic() - optimization_start
        logger.info(f"{'=' * 60}")
        logger.info(f"Optimization complete in {total_elapsed / 60:.1f} minutes")

        if best_params:
            self.output_path.parent.mkdir(parents=True, exist_ok=True)
            full_output = {"pred": best_params}
            with self.output_path.open("w") as f:
                json.dump(full_output, f, indent=2)
            logger.info(f"Best objective: {best_objective:.4f}")
            logger.info(f"Best params written to {self.output_path}")
            logger.info(f"Trial log:      {self.log_csv_path}")
            logger.info(f"Best params: {json.dumps(best_params, indent=2)}")
        else:
            logger.warning("No successful trials completed")

        return best_params
