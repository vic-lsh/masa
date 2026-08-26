from __future__ import annotations

import csv
import json
from pathlib import Path

import pytest

from eval.estimator_ordering.analyze import analyze, parse_audit_logs, write_outputs


HEADERS = (
    "api",
    "request_id",
    "slo",
    "start_at",
    "deadline",
    "latency",
    "error",
    "hop_trace_json",
)


def _hops(long: bool, queue_us: int, base: int) -> str:
    hops = [
        {
            "service_id": "shared",
            "method_name": "run",
            "started_at": base + queue_us,
            "finished_at": base + queue_us + 3_000,
            "configured_work_us": 3_000,
            "queueing_latency_us": queue_us,
            "children": [],
        },
        {
            "service_id": "short-tail",
            "method_name": "wait",
            "started_at": base + 4_000,
            "finished_at": base + 6_000,
            "configured_work_us": 2_000,
            "queueing_latency_us": 0,
            "children": [],
        },
    ]
    if long:
        hops.append(
            {
                "service_id": "long-extra",
                "method_name": "wait",
                "started_at": base + 6_000,
                "finished_at": base + 29_000,
                "configured_work_us": 23_000,
                "queueing_latency_us": 0,
                "children": [],
            }
        )
    return json.dumps(hops)


def _write_fixture(tmp_path: Path) -> Path:
    root = tmp_path / "estimator_ordering_hidden"
    audited = root / "0" / "eval_estimator_audit,est_mean_var"
    baseline = root / "0" / "sched_slo"
    for policy_dir in (audited, baseline):
        policy_dir.mkdir(parents=True)
        (policy_dir / "gen_config.json").write_text(
            json.dumps({"DurationSecs": 10}), encoding="utf-8"
        )
        with (policy_dir / "r220_a.csv").open(
            "w", newline="", encoding="utf-8"
        ) as result_file:
            writer = csv.DictWriter(result_file, fieldnames=HEADERS)
            writer.writeheader()
            for request_id, long, latency, queue in (
                (1, False, 20_000, 400),
                (2, True, 55_000, 100),
                (3, False, 22_000, 500),
                (4, True, 45_000, 200),
            ):
                writer.writerow(
                    {
                        "api": "a",
                        "request_id": request_id,
                        "slo": 50_000,
                        "start_at": 1_000,
                        "deadline": 51_000,
                        "latency": latency,
                        "error": "/None",
                        "hop_trace_json": _hops(long, queue, request_id * 10_000),
                    }
                )

    logs = audited / "logs"
    logs.mkdir()
    with (logs / "frontend.log").open("w", encoding="utf-8") as log_file:
        log_file.write("not an audit record\n")
        log_file.write(f"INFO {AUDIT_PREFIX_FOR_TEST}{{bad json}}\n")
        # One effective key deliberately hides the short/long branch. Learned
        # values tie within the burst and shadow feasibility signals requests
        # 2 and 4. Only request 2 ultimately misses its SLO.
        for request_id, reference, learned, time_left, child_deadline in (
            (1, 2_000, 10_000, 40_000, 50_000),
            (2, 25_000, 10_000, 5_000, 20_000),
            (3, 2_000, 10_000, 40_000, 50_000),
            (4, 25_000, 10_000, 5_000, 40_000),
        ):
            event = {
                "request_id": request_id,
                "gateway_entry_us": 1_000,
                "child_method": "shared::run",
                "parent_deadline_us": 51_000,
                "effective_priority_key": "hidden-key",
                "learned_full_us": learned,
                "reference_remaining_us": reference,
                "time_left_us": time_left,
                "child_deadline_us": child_deadline,
            }
            log_file.write(f"INFO {AUDIT_PREFIX_FOR_TEST}{json.dumps(event)}\n")
    return root


AUDIT_PREFIX_FOR_TEST = "EST_AUDIT_JSON:"


def test_hidden_path_metrics_and_shadow_feasibility(tmp_path: Path) -> None:
    root = _write_fixture(tmp_path)

    result = analyze([root])
    runs = {run["policy"]: run for run in result["runs"]}
    audited = runs["eval_estimator_audit,est_mean_var"]

    assert audited["requests"] == 4
    assert audited["goodput_rps"] == pytest.approx(3 / 0.055)
    assert audited["classes"]["short"]["slo_attainment"] == 1.0
    assert audited["classes"]["long"]["slo_attainment"] == 0.5
    assert audited["shared_queue"]["p50_us"] == 300.0
    assert audited["audit"]["request_coverage"] == 1.0
    assert audited["audit"]["nae_p50"] == pytest.approx(0.23)
    assert audited["audit"]["priority_nae_p50"] == pytest.approx(0.23)
    assert audited["ordering"]["learned_pair_tie_fraction"] == 1.0
    assert audited["ordering"]["pairwise_reference_order_agreement"] is None
    assert audited["ordering"]["same_key_ambiguity"]["gt_10pct_slo"] == 2 / 3
    assert audited["shadow_feasibility"]["learned"]["precision"] == 0.5
    assert audited["shadow_feasibility"]["learned"]["recall"] == 1.0
    assert audited["shadow_feasibility"]["learned"]["false_positive_rate"] == 1 / 3

    baseline = runs["sched_slo"]
    assert baseline["audit"]["available"] is False
    assert baseline["audit"]["nae_p50"] is None
    assert baseline["ordering"]["realized_shared_start_reference_agreement"] is not None


def test_malformed_audit_lines_are_ignored_and_outputs_are_written(
    tmp_path: Path,
) -> None:
    root = _write_fixture(tmp_path)
    policy_dir = root / "0" / "eval_estimator_audit,est_mean_var"

    assert len(parse_audit_logs(policy_dir)) == 4

    result = analyze([tmp_path])
    output_dir = tmp_path / "summary"
    write_outputs(result, output_dir)
    assert json.loads((output_dir / "summary.json").read_text())["runs"]
    assert "goodput_rps_mean" in (output_dir / "summary.csv").read_text()
    assert "| Experiment | Policy |" in (output_dir / "summary.md").read_text()


def test_visible_keys_have_correct_pair_and_top_choice_order(tmp_path: Path) -> None:
    root = _write_fixture(tmp_path)
    log_path = (
        root / "0" / "eval_estimator_audit,est_mean_var" / "logs" / "frontend.log"
    )
    rewritten = []
    for line in log_path.read_text(encoding="utf-8").splitlines():
        marker = line.find(AUDIT_PREFIX_FOR_TEST)
        if marker < 0:
            rewritten.append(line)
            continue
        try:
            event = json.loads(line[marker + len(AUDIT_PREFIX_FOR_TEST) :])
        except json.JSONDecodeError:
            rewritten.append(line)
            continue
        reference = event["reference_remaining_us"]
        event["effective_priority_key"] = (
            "long-key" if reference == 25_000 else "short-key"
        )
        # Point accuracy consumes the uncapped raw field, while ordering must
        # consume the scaled/decayed priority actually handed to the runtime.
        event["learned_full_raw_us"] = 50_000 - reference
        event["effective_priority_us"] = reference
        rewritten.append(f"INFO {AUDIT_PREFIX_FOR_TEST}{json.dumps(event)}")
    log_path.write_text("\n".join(rewritten) + "\n", encoding="utf-8")

    result = analyze([root])
    audited = next(
        run
        for run in result["runs"]
        if run["policy"] == "eval_estimator_audit,est_mean_var"
    )
    ordering = audited["ordering"]
    assert ordering["pairwise_reference_order_agreement"] == 1.0
    assert ordering["top_choice_reference_agreement"] == 1.0
    assert ordering["cross_key_flip_fraction"] == 0.0
    assert ordering["same_key_ambiguity"]["gt_1pct_slo"] == 0.0
    assert audited["audit"]["nae_p50"] == pytest.approx(0.46)
    assert audited["audit"]["priority_nae_p50"] == 0.0
