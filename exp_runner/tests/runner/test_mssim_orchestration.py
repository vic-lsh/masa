import json
import subprocess
from unittest.mock import MagicMock, patch

import pytest

from exp_runner.runner.apps.mssim import MssimApp
from exp_runner.runner.config import ExperimentConfig
from exp_runner.runner.executor import MockCommandExecutor, RecordedCommand
from exp_runner.runner.docker_manager import DockerManager
from exp_runner.runner.experiment_driver import ExpDriver
from exp_runner.runner.cpu_monitor import CPUMonitor


@pytest.fixture
def mock_executor():
    return MockCommandExecutor()


@pytest.fixture
def mock_deployment():
    # Mock DockerManager since it's used in type hints and instantiation
    dm = MagicMock(spec=DockerManager)
    dm.get_container_names.return_value = []
    dm.stream_logs.return_value = []
    dm.check_project_health.return_value = []
    return dm


def test_mssim_run_workload_orchestration(tmp_path, mock_executor, mock_deployment):
    """
    Test that MssimApp.run_workload generates the correct sequence of commands:
    1. Build images
    2. Generate docker-compose via simulator.main
    3. Run docker compose up
    4. Wait for loadgen
    5. Cleanup
    """
    app = MssimApp(executor=mock_executor)

    # Setup test directory structure
    repo_root = tmp_path / "repo"
    repo_root.mkdir()

    app_dir = repo_root / "apps" / "mssim"
    app_dir.mkdir(parents=True)

    # Dockerfile needed for build check
    (app_dir / "generic-service" / "Dockerfile").parent.mkdir(
        parents=True, exist_ok=True
    )
    (app_dir / "generic-service" / "Dockerfile").touch()

    in_dir = repo_root / "exp" / "mssim" / "in" / "test_exp"
    in_dir.mkdir(parents=True)

    # Config files
    (in_dir / "gen_config.json").write_text(
        json.dumps({"Rps": [100], "DurationSecs": 10, "WarmupSecs": 5})
    )

    (in_dir / "mssim.json").write_text(
        json.dumps(
            {
                "callgraph_dirs": [str(tmp_path / "graphs")],
                "slo_ms": 50,
                "stats_interval_sec": 1,
            }
        )
    )
    (tmp_path / "graphs").mkdir()

    # Create dummy ExperimentConfig
    # We mock ExperimentConfig since loading it requires more complex file structure
    config = MagicMock(spec=ExperimentConfig)
    config.experiment_name = "test_exp"
    config.app_name = "mssim"
    config.repo_root = repo_root
    config.app_dir = app_dir
    config.in_dir = in_dir
    config.out_dir = tmp_path / "out"
    config.gen_config = {"Rps": [100], "DurationSecs": 10}
    config.app_config = {"callgraph_dirs": [str(tmp_path / "graphs")], "slo_ms": 50}
    config.policies = ["prio_global"]

    output_dir = tmp_path / "out"
    output_dir.mkdir()

    # Mock CPUMonitor to avoid threading/file issues
    mock_deployment.get_container_names.return_value = [
        "service-1",
        "service-2",
        "loadgen",
    ]

    # Intercept run to handle docker inspect
    original_run = mock_executor.run

    def side_effect(args, **kwargs):
        cmd_list = [str(a) for a in args]
        # Check if it's the inspect command
        if "docker" in cmd_list and "inspect" in cmd_list and "loadgen" in cmd_list:
            # We still want to record history!
            mock_executor.history.append(
                RecordedCommand(cmd_list, kwargs.get("cwd"), kwargs.get("env"))
            )
            return subprocess.CompletedProcess(
                args=cmd_list, returncode=0, stdout="exited", stderr=""
            )
        return original_run(args, **kwargs)

    mock_executor.run = side_effect

    # Mock CPU monitor to avoid actual threads
    mock_cpu_monitor_cls = MagicMock(spec=CPUMonitor)
    mock_cpu_monitor_instance = mock_cpu_monitor_cls.return_value
    mock_cpu_monitor_instance.start = MagicMock()
    mock_cpu_monitor_instance.stop = MagicMock()

    driver = ExpDriver(
        app=app,
        deployment=mock_deployment,
        executor=mock_executor,
        cpu_monitor_factory=mock_cpu_monitor_cls,
    )

    with patch("exp_runner.runner.apps.mssim.CPUMonitor", mock_cpu_monitor_cls):
        driver.run_workload(
            repo_root=repo_root,
            config=config,
            policy="prio_global",
            iteration=0,
            output_dir=output_dir,
            no_cache=False,
            dry_run=False,
        )

    # Verify command history
    history = mock_executor.history
    commands = [cmd.args for cmd in history]

    # 1. Check build commands
    # Should build loadgen
    assert any("mssim_load_generator" in str(cmd) for cmd in commands)
    # Should build generic service
    assert any("generic_service" in str(cmd) for cmd in commands)

    # 2. Check simulator generation command
    gen_cmd = next(
        (
            cmd
            for cmd in commands
            if "simulator.main" in str(cmd)
            or ("python" in cmd[0] and "simulator.main" in cmd)
        ),
        None,
    )
    assert gen_cmd is not None
    assert "--docker-compose-output-path" in gen_cmd
    assert "--deployment-output-path" in gen_cmd

    # 3. Check docker compose up
    # ExpDriver calls deployment.start
    mock_deployment.start.assert_called()
    _, kwargs = mock_deployment.start.call_args
    assert kwargs["deployment_config"] == "docker-compose.yml"
    assert "mssim-test-exp-" in kwargs["project_name"]

    # 4. Check log streaming
    mock_deployment.stream_logs.assert_called()

    # 5. Check cleanup
    mock_deployment.stop.assert_called()

    # 6. Check loadgen task (ExpDriver calls run_task)
    mock_deployment.run_task.assert_called()
    task_spec = mock_deployment.run_task.call_args[0][0]
    assert task_spec.name == "mssim-loadgen"


def test_mssim_orchestration_k8s(tmp_path, mock_executor):
    """Test orchestration logic when running on K8s (calls _run_k8s_workload)"""
    from exp_runner.runner.k8s_manager import K8sManager

    app = MssimApp(executor=mock_executor)
    repo_root = tmp_path / "repo"
    repo_root.mkdir()
    app_dir = repo_root / "apps" / "mssim"
    app_dir.mkdir(parents=True)

    # Dockerfile needed
    (app_dir / "generic-service" / "Dockerfile").parent.mkdir(
        parents=True, exist_ok=True
    )
    (app_dir / "generic-service" / "Dockerfile").touch()

    in_dir = repo_root / "exp" / "mssim" / "in" / "test_exp_k8s"
    in_dir.mkdir(parents=True)

    real_k8s_manager = K8sManager(repo_root=repo_root, executor=mock_executor)
    # We need to mock methods that do real work
    real_k8s_manager.start = MagicMock()
    real_k8s_manager.stop = MagicMock()
    real_k8s_manager.run_task = MagicMock()
    real_k8s_manager._run_cmd = MagicMock()

    # Setup Config
    config = MagicMock(spec=ExperimentConfig)
    config.experiment_name = "test_exp_k8s"
    config.app_name = "mssim"
    config.repo_root = repo_root
    config.app_dir = app_dir
    config.in_dir = in_dir
    config.gen_config = {"Rps": [100], "DurationSecs": 10}
    graphs_dir = tmp_path / "graphs"
    graphs_dir.mkdir()
    (graphs_dir / "edges.csv").write_text("caller,callee,weight\nsvc_a,svc_b,1\n")
    (graphs_dir / "latency_percentiles.json").write_text("{}\n")
    config.app_config = {"callgraph_dirs": [str(graphs_dir)], "slo_ms": 50}

    # Run
    output_dir = tmp_path / "out_k8s"
    output_dir.mkdir()

    # Pre-create deployment.json since the mocked executor won't create it
    # ExpDriver uses output_dir directly
    (output_dir / "deployment.json").write_text(
        json.dumps(
            {
                "services": {
                    "frontend": {"port": 80, "replicas": 1},
                    "backend": {"port": 8080, "replicas": 2},
                }
            }
        )
    )
    (output_dir / "frontend.json").write_text(
        "[]"
    )  # Also create frontend.json as it's modified

    driver = ExpDriver(
        app=app,
        deployment=real_k8s_manager,
        executor=mock_executor,
    )

    # We need to stub out CPU monitor inside ExpDriver if it uses it (it defaults to CPUMonitor)
    # But for K8s it might be fine or we can patch it
    with patch("exp_runner.runner.experiment_driver.CPUMonitor"):
        driver.run_workload(
            repo_root=repo_root,
            config=config,
            policy="fifo",
            iteration=0,
            output_dir=output_dir,
            no_cache=True,
            dry_run=False,
        )

    # Verification
    # 1. Should generate configs
    history = mock_executor.history
    commands = [cmd.args for cmd in history]
    gen_cmd = next(
        (
            cmd
            for cmd in commands
            if "simulator.main" in str(cmd)
            or ("python" in cmd[0] and "simulator.main" in cmd)
        ),
        None,
    )
    assert gen_cmd is not None

    # 2. Should deploy helm chart
    real_k8s_manager.start.assert_called()
    # Check that it installed the chart from charts/mssim
    start_args = real_k8s_manager.start.call_args[1]
    assert "charts/mssim" in str(start_args.get("app_dir", "")) or "mssim" in str(
        start_args.get("deployment_config", "")
    )

    # 3. Should run loadgen task
    real_k8s_manager.run_task.assert_called()
    task_spec = real_k8s_manager.run_task.call_args[0][0]
    assert task_spec.image == "mssim_load_generator"
    assert task_spec.name == "mssim-loadgen"

    # 4. Should cleanup
    real_k8s_manager.stop.assert_called()
