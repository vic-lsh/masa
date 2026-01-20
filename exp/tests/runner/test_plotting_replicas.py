import json
from pathlib import Path

from exp.runner.plotting.replicas import _collect_replica_data, generate_replicas_plots


def _write_hotel_config(path: Path, *, reservation_replicas: int) -> None:
    path.write_text(
        json.dumps(
            {
                "frontend": {"replicas": 1},
                "reservation": {"replicas": reservation_replicas},
                "geo": {"replicas": 1},
            }
        )
    )


def test_generate_replicas_plot_creates_output(tmp_path: Path) -> None:
    in_dir = tmp_path / "exp" / "hotel" / "data" / "in"
    fifo_1000 = in_dir / "fifo_1000"
    prio_1500 = in_dir / "prio_1500"
    noise_dir = in_dir / "template"
    fifo_1000.mkdir(parents=True)
    prio_1500.mkdir(parents=True)
    noise_dir.mkdir(parents=True)

    _write_hotel_config(fifo_1000 / "hotel.json", reservation_replicas=2)
    _write_hotel_config(prio_1500 / "hotel.json", reservation_replicas=3)
    (fifo_1000 / "gen_config.json").write_text("{}")
    (prio_1500 / "gen_config.json").write_text("{}")

    output_dir = tmp_path / "plots"
    generate_replicas_plots(in_dir, output_dir)

    assert (output_dir / "replica_distribution.png").exists()
    assert (output_dir / "replica_policy_comparison.png").exists()


def test_collect_replica_data_excludes_frontend(tmp_path: Path) -> None:
    in_dir = tmp_path / "exp" / "hotel" / "data" / "in"
    fifo_1000 = in_dir / "fifo_1000"
    fifo_1000.mkdir(parents=True)

    _write_hotel_config(fifo_1000 / "hotel.json", reservation_replicas=2)

    data, service_order = _collect_replica_data(in_dir)

    assert "frontend" not in service_order
    assert "frontend" not in data["fifo"][1000]
