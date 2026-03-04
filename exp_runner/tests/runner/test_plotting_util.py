import json

import pandas as pd

from exp_runner.runner.plotting.util import (
    get_policy_display_name,
    read_data,
    read_policies,
)


def test_get_policy_display_name_known_policies():
    assert get_policy_display_name("fifo") == "FIFO (no-drop)"
    assert get_policy_display_name("fifo,early") == "FIFO"
    assert get_policy_display_name("prio_global") == "Masa (global ddl) (no-drop)"
    assert get_policy_display_name("prio_global,early") == "Masa (global ddl)"
    assert get_policy_display_name("prio_local") == "Masa (local ddl) (no-drop)"
    assert get_policy_display_name("prio_local,early") == "Masa (local ddl)"
    assert (
        get_policy_display_name("prio_local,early,prio_local_transform")
        == "Masa (local ddl, transformed)"
    )
    assert (
        get_policy_display_name("prio_local,prio_local_transform")
        == "Masa (local ddl, transformed) (no-drop)"
    )
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
