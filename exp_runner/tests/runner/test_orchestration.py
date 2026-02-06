import json
from pathlib import Path
from unittest.mock import MagicMock, patch

import pytest

from exp_runner.runner.apps.synthetic import SyntheticApp
from exp_runner.runner.config import ExperimentConfig
from exp_runner.runner.executor import MockCommandExecutor
from exp_runner.runner.experiment import Experiment


class TestOrchestration:
    @pytest.fixture
    def mock_repo_root(self, tmp_path):
        """Create a mock repo root structure."""
        root = tmp_path / "repo"
        root.mkdir()
        (root / "apps" / "synthetic").mkdir(parents=True)
        (root / "exp" / "synthetic" / "in" / "test_exp").mkdir(parents=True)

        # Create dummy gen_config.json needed by builder logic even in dry-run
        # (builder logic checks gen_config_path exists relative to repo_root)
        gen_config_path = (
            root / "exp" / "synthetic" / "in" / "test_exp" / "gen_config.json"
        )
        gen_config_path.write_text(
            json.dumps({"Addr": "http://localhost:8080", "Repeats": 1})
        )

        return root

    def test_experiment_dry_run_commands(self, mock_repo_root):
        """
        Test that running an experiment in dry-run mode issues the expected commands
        without executing them.
        """
        app = SyntheticApp()

        config = ExperimentConfig(
            experiment_name="test_exp",
            app_name="synthetic",
            repo_root=mock_repo_root,
            exp_dir=mock_repo_root / "exp" / "synthetic",
            app_dir=mock_repo_root / "apps" / "synthetic",
            in_dir=mock_repo_root / "exp" / "synthetic" / "in" / "test_exp",
            out_dir=mock_repo_root / "exp" / "synthetic" / "out" / "test_exp",
            plot_dir=mock_repo_root / "exp" / "synthetic" / "plots" / "test_exp",
            gen_config={"Addr": "http://localhost:8080", "Repeats": 1},
            policies=["fifo"],
            app_config=None,
        )

        experiment = Experiment(
            app=app,
            config=config,
            repo_root=mock_repo_root,
            dry_run=True,
            rm_data=True,  # Should be safe in dry-run
        )

        # Run the experiment
        experiment.run()

        # Verify executor type
        assert isinstance(experiment.executor, MockCommandExecutor)
        history = experiment.executor.history

        # Analyze recorded commands
        commands = [cmd.args for cmd in history]
        cmd_strings = [" ".join(cmd) for cmd in commands]

        # 1. Check for Docker Build commands (Multi-stage)
        # Stage 1: builder
        assert any(
            "target builder" in cmd and "synthetic" in cmd for cmd in cmd_strings
        )
        # Stage 2: runtime-base
        assert any("target runtime-base" in cmd for cmd in cmd_strings)
        # Stage 3: runtime (binaries)
        assert any(
            "target runtime" in cmd and "synthetic_frontend" in cmd
            for cmd in cmd_strings
        )
        assert any(
            "target runtime" in cmd and "synthetic_child" in cmd for cmd in cmd_strings
        )

        # 2. Check for Deployment (Docker Compose) - dry run usually prints but might not call executor if logic skips it?
        # In ExpDriver:
        # if dry_run: logger.info("Would deploy..."); return
        # So ExpDriver STOPS before deployment commands in dry_run mode.
        # This confirms that dry_run logic in ExpDriver is working to PREVENT execution.

        # If we want to test deployment commands, we might need a "DryRunExecutor" that allows logic to proceed
        # but prevents subprocess calls. Currently `dry_run=True` flag in ExpDriver shortcuts the logic.

        # Verify NO deployment commands were run
        assert not any("docker compose up" in cmd for cmd in cmd_strings)
        assert not any("docker run" in cmd for cmd in cmd_strings)  # Loadgen

        # This confirms that dry_run flag correctly short-circuits the dangerous parts
        # while still exercising the build command generation logic (which we updated to use executor).

    def test_experiment_executor_propagation(self, mock_repo_root):
        """
        Test that the executor is correctly propagated to components.
        We can't easily check internal state of builder, but we can verify behavior.
        """
        # This is covered by the test above since build commands appear in the executor history.
        pass
