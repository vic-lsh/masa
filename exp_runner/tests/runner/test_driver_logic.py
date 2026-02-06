import shutil
import tempfile
import unittest
from pathlib import Path
from unittest.mock import MagicMock, call

from exp_runner.runner.apps.base import AppBuilder, AppPlugin, DockerConfig
from exp_runner.runner.config import ExperimentConfig
from exp_runner.runner.cpu_monitor import CPUMonitor
from exp_runner.runner.deployment_manager import DeploymentManager, TaskSpec
from exp_runner.runner.experiment_driver import ExpDriver


class MockDeploymentManager(DeploymentManager):
    def __init__(self):
        super().__init__(repo_root=Path("."))
        self.start_calls = []
        self.stop_calls = []
        self.run_task_calls = []
        self.cleanup_task_calls = []
        self.copy_from_container_calls = []
        self.stream_logs_calls = []
        self.check_project_health_calls = []

    def start(
        self,
        app_dir: Path,
        deployment_config: str,
        env_vars: dict,
        project_name: str,
    ) -> None:
        self.start_calls.append(
            {
                "app_dir": app_dir,
                "config": deployment_config,
                "env": env_vars,
                "project": project_name,
            }
        )

    def stop(
        self,
        app_dir: Path,
        deployment_config: str,
        env_vars: dict = None,
        project_name: str = None,
    ) -> None:
        self.stop_calls.append(
            {
                "app_dir": app_dir,
                "config": deployment_config,
                "env": env_vars,
                "project": project_name,
            }
        )

    def run_task(self, task_spec: TaskSpec, log_file: Path = None) -> None:
        self.run_task_calls.append({"spec": task_spec, "log": log_file})
        if task_spec.name == "fail_me":
            raise RuntimeError("Task failed")

    def cleanup_task(self, task_spec: TaskSpec) -> None:
        self.cleanup_task_calls.append(task_spec)

    def get_container_names(
        self, config_path: Path, project_name: str, env_vars: dict = None
    ) -> list[str]:
        return ["container1", "container2"]

    def stream_logs(
        self, container_names: list[str], output_dir: Path, follow: bool = True
    ) -> list:
        self.stream_logs_calls.append(container_names)
        return []

    def check_project_health(
        self, config_path: Path, project_name: str, env_vars: dict = None
    ) -> list:
        self.check_project_health_calls.append(project_name)
        return []

    def copy_from_container(
        self, container_name: str, src_path: str, dest_path: Path
    ) -> None:
        self.copy_from_container_calls.append((container_name, src_path, dest_path))


class MockAppPlugin(AppPlugin):
    def __init__(self):
        self.builder = MagicMock(spec=AppBuilder)
        self.loadgen_spec = TaskSpec(
            name="loadgen",
            image="loadgen:latest",
            cleanup=True,
            artifacts=[("/tmp/trace", "trace.json")],
        )

    def get_app_name(self) -> str:
        return "mockapp"

    def load_app_config(self, config_path: Path) -> dict:
        return {}

    def generate_env_vars(
        self, gen_config: dict, app_config: dict, app_dir: Path
    ) -> dict:
        return {"ENV_VAR": "value"}

    def get_docker_config(self) -> DockerConfig:
        return DockerConfig(
            compose_file="docker-compose.yml",
            network_name="network",
            loadgen_image_name="loadgen",
            loadgen_binary_name="binary",
        )

    def get_container_names(self, env_vars: dict) -> list[str]:
        return ["container1", "container2"]

    def create_load_generator(self, features: str = None):
        return MagicMock()

    def create_builder(self) -> AppBuilder:
        return self.builder

    def prepare_workload(
        self,
        config: ExperimentConfig,
        policy: str,
        iteration: int,
        output_dir: Path,
        repo_root: Path,
        use_k8s: bool = False,
    ) -> dict:
        return {
            "DOCKER_COMPOSE_PROJECT_NAME": "test_project",
            "ENV_VAR": "value",
        }

    def get_deployment_location(
        self, output_dir: Path, use_k8s: bool, repo_root: Path
    ) -> tuple[Path, str]:
        return Path("/deploy"), "compose.yml"

    def get_loadgen_spec(
        self, output_dir: Path, features: str, env_vars: dict, use_k8s: bool
    ) -> TaskSpec:
        return self.loadgen_spec


class MockCPUMonitor(CPUMonitor):
    def __init__(self, output_path, poll_interval, container_names):
        self.start_called = False
        self.stop_called = False

    def start(self):
        self.start_called = True

    def stop(self):
        self.stop_called = True


class TestExpDriverLogic(unittest.TestCase):
    def setUp(self):
        self.test_dir = Path(tempfile.mkdtemp())
        self.app = MockAppPlugin()
        self.deployment = MockDeploymentManager()
        self.executor = MagicMock()
        self.cpu_monitor_factory = MagicMock(side_effect=MockCPUMonitor)

        self.driver = ExpDriver(
            app=self.app,
            deployment=self.deployment,
            executor=self.executor,
            cpu_monitor_factory=self.cpu_monitor_factory,
        )

        self.config = MagicMock(spec=ExperimentConfig)
        self.config.in_dir = self.test_dir / "in"
        self.config.in_dir.mkdir()
        # Create dummy gen_config
        (self.config.in_dir / "gen_config.json").touch()
        self.config.app_dir = self.test_dir / "app"
        self.config.gen_config = {}

    def tearDown(self):
        shutil.rmtree(self.test_dir)

    def test_run_workload_happy_path(self):
        output_dir = self.test_dir / "out"
        repo_root = self.test_dir / "repo"
        self.driver.run_workload(
            config=self.config,
            policy="prio_global",
            iteration=1,
            output_dir=output_dir,
            repo_root=repo_root,
            dry_run=False,
        )

        # Verify Builder
        self.app.builder.build.assert_called()

        # Verify Deployment Start
        self.assertEqual(len(self.deployment.start_calls), 1)
        self.assertEqual(self.deployment.start_calls[0]["project"], "test_project")

        # Verify CPU Monitor
        self.cpu_monitor_factory.assert_called()
        # Get the instance created by factory
        cpu_monitor = self.cpu_monitor_factory.return_value
        self.assertTrue(cpu_monitor.start_called)
        self.assertTrue(cpu_monitor.stop_called)

        # Verify Loadgen
        self.assertEqual(len(self.deployment.run_task_calls), 1)
        self.assertEqual(self.deployment.run_task_calls[0]["spec"].name, "loadgen")

        # Verify Artifacts
        self.assertEqual(len(self.deployment.copy_from_container_calls), 1)
        self.assertEqual(
            self.deployment.copy_from_container_calls[0][0], "loadgen"
        )  # container name

        # Verify Cleanup
        self.assertEqual(len(self.deployment.cleanup_task_calls), 1)
        self.assertEqual(len(self.deployment.stop_calls), 1)

    def test_run_workload_task_failure(self):
        """Verify cleanup happens even if task fails."""
        output_dir = self.test_dir / "out"
        repo_root = self.test_dir / "repo"

        # Setup failure
        self.app.loadgen_spec.name = "fail_me"

        with self.assertRaises(RuntimeError):
            self.driver.run_workload(
                config=self.config,
                policy="prio_global",
                iteration=1,
                output_dir=output_dir,
                repo_root=repo_root,
                dry_run=False,
            )

        # Verify Cleanup still happened
        self.assertEqual(len(self.deployment.stop_calls), 1)
        cpu_monitor = self.cpu_monitor_factory.return_value
        self.assertTrue(cpu_monitor.stop_called)

        # Verify failure checks
        # check_project_health is called once during wait_until (stabilization) and once in exception handler
        self.assertEqual(len(self.deployment.check_project_health_calls), 2)


if __name__ == "__main__":
    unittest.main()
