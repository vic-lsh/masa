from argparse import Namespace
import importlib
import json

import pandas as pd

from exp_runner.runner.plotting.util import (
    get_policy_display_name,
    load_plot_data,
    PlotData,
    read_data,
    read_policies,
)

plotting_all = importlib.import_module("exp_runner.runner.plotting.all")


def test_get_policy_display_name_known_policies():
    assert get_policy_display_name("fifo") == "FIFO (no-drop)"
    assert get_policy_display_name("fifo,early") == "FIFO"
    assert get_policy_display_name("prio_global") == "Masa (global ddl) (no-drop)"
    assert get_policy_display_name("prio_global,early") == "Masa (global ddl)"
    assert get_policy_display_name("prio_local") == "Masa (local ddl) (no-drop)"
    assert get_policy_display_name("prio_local,early") == "Masa (local ddl)"
    assert get_policy_display_name("prio_oldest") == "Tailclipper (no-drop)"
    assert get_policy_display_name("prio_oldest,early") == "Tailclipper"


def test_get_policy_display_name_unknown_policies():
    assert get_policy_display_name("custom") == "custom (no-drop)"
    assert get_policy_display_name("custom,early") == "custom"


def test_read_policies_from_config_dir(tmp_path):
    config_dir = tmp_path / "config"
    config_dir.mkdir()
    (config_dir / "policies").write_text("fifo prio_global\n", encoding="utf-8")

    policies = read_policies(config_dir)

    assert policies == ["fifo", "prio_global"]


def test_read_data_uses_policies_file(tmp_path):
    config_dir = tmp_path / "config"
    data_dir = tmp_path / "data"
    config_dir.mkdir()
    data_dir.mkdir()

    (config_dir / "gen_config.json").write_text(
        json.dumps({"Repeats": 1, "Rps": [10], "Apis": ["Login"], "Slos": [1000]}),
        encoding="utf-8",
    )
    (config_dir / "policies").write_text("fifo prio_global\n", encoding="utf-8")

    for policy in ["fifo", "prio_global", "extra_policy"]:
        policy_dir = data_dir / "0" / policy
        policy_dir.mkdir(parents=True, exist_ok=True)
        (policy_dir / "r10_Login.csv").write_text(
            "start_at,error\n0,\n", encoding="utf-8"
        )

    repeats, apis, policies, rps_values, results = read_data(config_dir, data_dir)

    assert repeats == 1
    assert apis[-1] == "ALL"
    assert policies == ["fifo", "prio_global"]
    assert rps_values == [10]
    assert "extra_policy" not in results[0]["Login"]


def test_load_plot_data_matches_read_data(tmp_path):
    config_dir = tmp_path / "config"
    data_dir = tmp_path / "data"
    config_dir.mkdir()
    data_dir.mkdir()

    (config_dir / "gen_config.json").write_text(
        json.dumps({"Repeats": 1, "Rps": [10], "Apis": ["Login"], "Slos": [1000]}),
        encoding="utf-8",
    )
    (config_dir / "policies").write_text("fifo\n", encoding="utf-8")

    policy_dir = data_dir / "0" / "fifo"
    policy_dir.mkdir(parents=True, exist_ok=True)
    (policy_dir / "r10_Login.csv").write_text(
        "api,start_at,latency,slo,error\nLogin,0,100,1000,\n",
        encoding="utf-8",
    )

    plot_data = load_plot_data(config_dir, data_dir)
    repeats, apis, policies, rps_values, results = read_data(config_dir, data_dir)

    assert plot_data.repeats == repeats
    assert plot_data.apis == apis
    assert plot_data.policies == policies
    assert plot_data.rps_values == rps_values
    assert len(plot_data.results) == len(results)
    assert plot_data.results[0]["Login"]["fifo"][10].equals(
        results[0]["Login"]["fifo"][10]
    )
    assert plot_data.results[0]["ALL"]["fifo"][10].equals(results[0]["ALL"]["fifo"][10])


def test_read_data_repairs_malformed_request_csv_rows(tmp_path):
    """
    Synthetic request CSVs can contain malformed rows where:
    - the error field contains unescaped commas (e.g. gRPC error strings), and/or
    - trailing latency fields are missing (e.g. early-return rows).

    Plot generation should still work by repairing those rows during read.
    """
    config_dir = tmp_path / "config"
    data_dir = tmp_path / "data"
    config_dir.mkdir()
    data_dir.mkdir()

    (config_dir / "gen_config.json").write_text(
        json.dumps(
            {
                "Repeats": 1,
                "Rps": [300],
                "Apis": ["a"],
                "Slos": [150000],
            }
        ),
        encoding="utf-8",
    )
    (config_dir / "policies").write_text("prio_local,early\n", encoding="utf-8")

    policy_dir = data_dir / "0" / "prio_local,early"
    policy_dir.mkdir(parents=True, exist_ok=True)

    header = "api,request_id,slo,start_at,deadline,latency,error,frontend_latency"
    good = "a,1,150000,0,150000,100,/None,90"
    early_return_missing_tail = "a,2,150000,1,150001,150542,/EarlyReturn,"
    grpc_error_with_commas_missing_tail = (
        "a,3,150000,2,150002,139592,RPC error: status: Internal, message: "
        '"RPC error: status: DeadlineExceeded, message: \\"/EarlyReturn\\", details: [], metadata: '
        'MetadataMap { headers: {\\"content-type\\": \\"application/grpc\\"} }", details: [], '
        'metadata: MetadataMap { headers: {"content-type": "application/grpc"} },'
    )

    (policy_dir / "r300_a.csv").write_text(
        "\n".join(
            [
                header,
                good,
                early_return_missing_tail,
                grpc_error_with_commas_missing_tail,
            ]
        )
        + "\n",
        encoding="utf-8",
    )

    repeats, apis, policies, rps_values, results = read_data(config_dir, data_dir)

    assert repeats == 1
    assert apis == ["a", "ALL"]
    assert policies == ["prio_local,early"]
    assert rps_values == [300]

    df = results[0]["a"]["prio_local,early"][300]
    # Original columns should be present
    for col in header.split(","):
        assert col in df.columns
    # New metadata columns should also be present
    assert "error_type" in df.columns
    assert "er_service" in df.columns

    assert len(df) == 3

    # Error field should be preserved even when it contains commas.
    err = df.loc[df["request_id"] == 3, "error"].iloc[0]
    assert "RPC error" in err
    assert "/EarlyReturn" in err

    # Missing trailing latency fields should become NaN after numeric conversion.
    assert df.loc[df["request_id"] == 2, "error"].iloc[0] == "/EarlyReturn"
    assert pd.isna(df.loc[df["request_id"] == 2, "frontend_latency"].iloc[0])


def test_generate_all_plots_loads_request_data_once(tmp_path, monkeypatch):
    args = Namespace(
        config_dir=tmp_path / "config",
        data_dir=tmp_path / "data",
        output_dir=tmp_path / "plots",
    )
    args.config_dir.mkdir()
    args.data_dir.mkdir()

    (args.config_dir / "gen_config.json").write_text(
        json.dumps({"Repeats": 1, "Rps": [10], "Apis": ["Login"], "Slos": [1000]}),
        encoding="utf-8",
    )
    (args.config_dir / "policies").write_text("fifo\n", encoding="utf-8")

    plot_data = PlotData(
        repeats=1,
        apis=["Login", "ALL"],
        policies=["fifo"],
        rps_values=[10],
        results=[],
    )
    calls = {"load": 0, "goodput": 0, "latency": 0, "queueing": 0, "cpu": 0}

    def fake_load_plot_data(config_dir, data_dir):
        calls["load"] += 1
        assert config_dir == args.config_dir
        assert data_dir == args.data_dir
        return plot_data

    def fake_plotter(name):
        def _run(passed_args, plot_data=None):
            calls[name] += 1
            assert passed_args is args
            assert plot_data is plot_data_ref

        return _run

    plot_data_ref = plot_data

    def fake_cpu_plot(data_dir, output_dir, policies=None):
        calls["cpu"] += 1
        assert data_dir == args.data_dir
        assert output_dir == args.output_dir
        assert policies == ["fifo"]

    monkeypatch.setattr(plotting_all, "load_plot_data", fake_load_plot_data)
    monkeypatch.setattr(plotting_all.goodput, "generate_plots", fake_plotter("goodput"))
    monkeypatch.setattr(plotting_all.latency, "generate_plots", fake_plotter("latency"))
    monkeypatch.setattr(
        plotting_all.queueing, "generate_plots", fake_plotter("queueing")
    )
    monkeypatch.setattr(plotting_all.cpu, "plot_cpu_utilization", fake_cpu_plot)

    plotting_all.generate_all_plots(args)

    assert calls == {
        "load": 1,
        "goodput": 1,
        "latency": 1,
        "queueing": 1,
        "cpu": 1,
    }
