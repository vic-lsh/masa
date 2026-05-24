"""
Generate all plots for an experiment.
"""

import logging
import os
import shutil
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path

from . import goodput
from . import latency
from . import queueing
from . import tracebench
from . import cpu
from .util import (
    get_plot_worker_count,
    load_plot_data,
    parse_args,
    prepare_output_dir,
)

logger = logging.getLogger(__name__)

# Names a user can pass to --only / --skip. Order matters for default
# scheduling (heaviest last so the lighter modules don't wait on it).
PLOT_MODULES: tuple[str, ...] = ("goodput", "latency", "queueing", "cpu")


def _resolve_modules(only: list[str] | None, skip: list[str] | None) -> set[str]:
    """Resolve --only / --skip into the set of module names to run.

    `only` wins over `skip` (selector vs. filter): if `only` is given, the
    enabled set is exactly that list intersected with the known modules.
    Otherwise the enabled set is all modules minus `skip`.
    """
    known = set(PLOT_MODULES)
    if only:
        requested = {m.strip() for m in only if m.strip()}
        unknown = requested - known
        if unknown:
            raise ValueError(
                f"unknown plot module(s) {sorted(unknown)}; known: {sorted(known)}"
            )
        return requested & known

    enabled = set(known)
    if skip:
        for m in skip:
            m = m.strip()
            if not m:
                continue
            if m not in known:
                raise ValueError(f"unknown plot module {m!r}; known: {sorted(known)}")
            enabled.discard(m)
    return enabled


def generate_all_plots(
    args,
    *,
    only: list[str] | None = None,
    skip: list[str] | None = None,
    summary_only: bool = False,
    use_cache: bool = True,
):
    """
    Generate all plots for an experiment.

    Args:
        args: Parsed arguments with config_dir, data_dir, and output_dir.
        only: If non-empty, run only these plot modules (subset of
            PLOT_MODULES). Overrides `skip`.
        skip: Plot modules to exclude (subset of PLOT_MODULES).
        summary_only: When true, skip per-iteration plots (the
            <output_dir>/0, 1, ... directories). Only `summary/` is rendered.
        use_cache: When true, reuse `<data_dir>/.plotcache.pkl` if its
            fingerprint still matches the inputs.

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
    # Resolve selectors first so an unknown module / empty set fails fast,
    # before any directory side effects.
    enabled = _resolve_modules(only, skip)
    if not enabled:
        logger.warning("no plot modules enabled — nothing to do")
        return

    config_dir = Path(args.config_dir)
    if (config_dir / "tracebench.json").exists():
        # tracebench has its own pipeline but honors the same selector kwargs.
        tracebench.generate_plots(
            args,
            only=only,
            skip=skip,
            summary_only=summary_only,
            use_cache=use_cache,
        )
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

    plot_data = load_plot_data(config_dir, args.data_dir, use_cache=use_cache)

    data_dir = Path(args.data_dir)
    cpu_dir = output_dir / "summary" / "cpu"
    if "cpu" in enabled:
        cpu_dir.mkdir(parents=True, exist_ok=True)

    runners = {
        "goodput": lambda: goodput.generate_plots(
            args, plot_data=plot_data, summary_only=summary_only
        ),
        "latency": lambda: latency.generate_plots(
            args, plot_data=plot_data, summary_only=summary_only
        ),
        "queueing": lambda: queueing.generate_plots(
            args, plot_data=plot_data, summary_only=summary_only
        ),
        "cpu": lambda: cpu.plot_cpu_utilization(
            data_dir, cpu_dir, policies=plot_data.policies
        ),
    }
    selected = [(name, runners[name]) for name in PLOT_MODULES if name in enabled]
    logger.info(
        f"plotting modules: {[n for n, _ in selected]}"
        + (" (summary only)" if summary_only else "")
    )

    with ThreadPoolExecutor(
        max_workers=get_plot_worker_count(len(selected), max_workers=4)
    ) as executor:
        futures = {executor.submit(fn): name for name, fn in selected}

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
