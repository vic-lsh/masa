from argparse import Namespace
from pathlib import Path

import pytest
from exp_runner.runner import cli


def test_run_group_discovers_nested_configs_in_stable_order(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    group = tmp_path / "exp" / "hotel" / "in" / "hotel_capacity_savings"
    for relative in ("p99/masa/3k", "p90/fifo/3k"):
        config_dir = group / relative
        config_dir.mkdir(parents=True)
        (config_dir / "gen_config.json").write_text("{}")
        (config_dir / "policies").write_text("sched_fifo\n")

    captured = None

    def capture_queue(args: Namespace) -> None:
        nonlocal captured
        captured = args

    monkeypatch.setattr(cli, "find_repo_root", lambda: tmp_path)
    monkeypatch.setattr(cli, "cmd_queue_experiments", capture_queue)

    args = Namespace(
        app="hotel",
        group="hotel_capacity_savings",
        plot=False,
        no_cache=False,
        rm_data=False,
        dry_run=True,
        smoke_test=False,
        k8s=False,
        kind=False,
    )
    cli.cmd_run_group(args)

    assert captured is not None
    assert captured.experiments == (
        "hotel_capacity_savings/p90/fifo/3k hotel_capacity_savings/p99/masa/3k"
    )
