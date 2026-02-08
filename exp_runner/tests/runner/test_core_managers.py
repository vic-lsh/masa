import json
import subprocess
from pathlib import Path
from unittest.mock import patch

import pytest

from exp_runner.runner.deployment_manager import TaskSpec
from exp_runner.runner.docker_manager import DockerManager
from exp_runner.runner.executor import MockCommandExecutor
from exp_runner.runner.k8s_manager import K8sManager


@pytest.fixture
def mock_executor():
    return MockCommandExecutor()


class TestDockerManager:
    def test_run_task(self, mock_executor, tmp_path):
        dm = DockerManager(repo_root=tmp_path, executor=mock_executor)
        task = TaskSpec(
            name="test-task",
            image="busybox",
            command=["echo", "hello"],
            env_vars={"FOO": "BAR"},
            cleanup=True,
        )

        dm.run_task(task)

        commands = [cmd.args for cmd in mock_executor.history]

        # Should remove existing container
        assert any("rm" in cmd and "test-task" in cmd for cmd in commands)

        # Should run container
        run_cmd = next(c for c in commands if c[0] == "docker" and c[1] == "run")
        assert "--name" in run_cmd
        assert "test-task" in run_cmd
        assert "-e" in run_cmd
        assert "FOO=BAR" in run_cmd
        assert "busybox" in run_cmd
        assert "echo" in run_cmd

        # Should cleanup (since cleanup=True)
        # The last command should be rm
        last_cmd = commands[-1]
        assert "rm" in last_cmd and "test-task" in last_cmd

    def test_start_stop(self, mock_executor, tmp_path):
        dm = DockerManager(repo_root=tmp_path, executor=mock_executor)
        app_dir = tmp_path / "app"
        app_dir.mkdir()
        (app_dir / "docker-compose.yml").touch()

        env_vars = {"KEY": "VALUE"}

        dm.start(app_dir, "docker-compose.yml", env_vars, "my-project")

        start_cmds = [cmd.args for cmd in mock_executor.history]
        up_cmd = next(c for c in start_cmds if "up" in c)
        assert "-p" in up_cmd
        assert "my-project" in up_cmd
        assert "KEY" in mock_executor.history[-1].env

        # Clear history for stop test
        mock_executor.history.clear()

        dm.stop(app_dir, "docker-compose.yml", env_vars, "my-project")
        stop_cmds = [cmd.args for cmd in mock_executor.history]
        down_cmd = next(c for c in stop_cmds if "down" in c)
        assert "-p" in down_cmd
        assert "my-project" in down_cmd

    def test_check_project_health(self, mock_executor, tmp_path):
        dm = DockerManager(repo_root=tmp_path, executor=mock_executor)

        # Mock ps output
        mock_output = json.dumps(
            [
                {"Name": "c1", "State": "running", "ExitCode": 0},
                {"Name": "c2", "State": "exited", "ExitCode": 1},  # Failed
                {"Name": "c3", "State": "exited", "ExitCode": 0},  # Success (completed)
            ]
        )

        # Configure mock response
        # The command string constructed in DockerManager uses shlex.join logic
        # We need to match it. But MockCommandExecutor uses exact match or simple checks.
        # It's better to patch executor.run return value if we can't easily predict the key.

        with patch.object(mock_executor, "run") as mock_run:
            mock_run.return_value = subprocess.CompletedProcess(
                args=[], returncode=0, stdout=mock_output, stderr=""
            )

            failed = dm.check_project_health(Path("docker-compose.yml"), "proj")

            assert len(failed) == 1
            assert failed[0] == ("c2", 1)


class TestK8sManager:
    def test_run_task(self, mock_executor, tmp_path):
        km = K8sManager(repo_root=tmp_path, namespace="test-ns", executor=mock_executor)
        task = TaskSpec(
            name="k8s-task",
            image="busybox",
            command=["echo", "hi"],
            env_vars={"A": "B"},
            cleanup=True,
        )

        # Mock pod phase polling
        with patch.object(km, "_get_pod_phase", side_effect=["Running", "Succeeded"]):
            km.run_task(task)

        commands = [cmd.args for cmd in mock_executor.history]

        # Should delete existing pod
        assert any("delete" in cmd and "pod" in cmd for cmd in commands)

        # Should apply manifest
        apply_cmd = next(c for c in commands if "apply" in c)
        assert "-f" in apply_cmd

        # Should cleanup
        assert any("delete" in cmd and "pod" in cmd for cmd in commands[-2:])

    def test_start_helm(self, mock_executor, tmp_path):
        km = K8sManager(repo_root=tmp_path, namespace="test-ns", executor=mock_executor)
        app_dir = tmp_path / "chart"
        app_dir.mkdir()

        km.start(app_dir, ".", {}, "my-release")

        commands = [cmd.args for cmd in mock_executor.history]
        install_cmd = next(c for c in commands if "helm" in c and "upgrade" in c)

        assert "my-release" in install_cmd
        assert "--namespace" in install_cmd
        assert "test-ns" in install_cmd
