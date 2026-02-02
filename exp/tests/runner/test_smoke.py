import pytest
from unittest.mock import MagicMock, patch
from pathlib import Path

from exp.runner.experiment import Experiment
from exp.runner.config import ExperimentConfig
from exp.runner.apps.base import AppPlugin
from exp.runner.apps.utils import verify_standard_workload

class TestSmokeTest:
    @pytest.fixture
    def mock_app(self):
        app = MagicMock(spec=AppPlugin)
        app.verify_results.return_value = True
        return app

    @pytest.fixture
    def mock_config(self, tmp_path):
        config = MagicMock(spec=ExperimentConfig)
        config.experiment_name = "test_exp"
        config.app_name = "test_app"
        config.policies = ["policy1"]
        config.get_repeats.return_value = 1
        config.out_dir = tmp_path / "out"
        config.exp_dir = tmp_path / "exp"
        config.app_dir = tmp_path / "app"
        config.plot_dir = tmp_path / "plot"
        config.in_dir = tmp_path / "in"
        config.gen_config = {"Rps": [100], "Apis": ["api1"], "Repeats": 1, "DurationSecs": 1}
        return config

    def test_experiment_calls_verify_results(self, mock_app, mock_config, tmp_path):
        repo_root = tmp_path
        
        # Mock DockerManager to avoid actual docker calls
        with patch("exp.runner.experiment.DockerManager") as MockDocker:
            exp = Experiment(
                app=mock_app,
                config=mock_config,
                repo_root=repo_root,
                smoke_test=True,
                dry_run=False
            )
            
            # Mock internal methods to avoid running actual logic
            exp._prepare_experiment = MagicMock()
            exp._copy_configs = MagicMock()
            exp._run_iterations = MagicMock()
            exp._mark_complete = MagicMock()
            exp._generate_plots = MagicMock()
            
            exp.run()
            
            mock_app.verify_results.assert_called_once_with(mock_config)

    def test_experiment_raises_on_failure(self, mock_app, mock_config, tmp_path):
        repo_root = tmp_path
        mock_app.verify_results.return_value = False
        
        with patch("exp.runner.experiment.DockerManager") as MockDocker:
            exp = Experiment(
                app=mock_app,
                config=mock_config,
                repo_root=repo_root,
                smoke_test=True,
                dry_run=False
            )
            
            exp._prepare_experiment = MagicMock()
            exp._copy_configs = MagicMock()
            exp._run_iterations = MagicMock()
            exp._mark_complete = MagicMock()
            exp._generate_plots = MagicMock()
            
            with pytest.raises(RuntimeError, match="Smoke test verification failed"):
                exp.run()

    def test_verify_standard_workload_success(self, mock_config, tmp_path):
        # Setup directory structure
        mock_config.out_dir.mkdir(parents=True)
        (mock_config.out_dir / "done").touch()
        
        policy_dir = mock_config.out_dir / "0" / "policy1"
        policy_dir.mkdir(parents=True)
        (policy_dir / "loadgen.log").touch()
        
        # Create CSV with valid goodput
        # Target RPS = 100, Duration = 1, APIs = 1 -> Target Total = 100
        # Goodput range: 80 - 120
        # We need 100 lines of "/None"
        csv_path = policy_dir / "r100_api1.csv"
        with open(csv_path, "w") as f:
            for _ in range(100):
                f.write("0,0,0,0,0,0,/None\n")
                
        assert verify_standard_workload(mock_config) is True

    def test_verify_standard_workload_failure_missing_done(self, mock_config, tmp_path):
        mock_config.out_dir.mkdir(parents=True)
        # Missing done file
        assert verify_standard_workload(mock_config) is False

    def test_verify_standard_workload_failure_goodput(self, mock_config, tmp_path):
        # Setup directory structure
        mock_config.out_dir.mkdir(parents=True)
        (mock_config.out_dir / "done").touch()
        
        policy_dir = mock_config.out_dir / "0" / "policy1"
        policy_dir.mkdir(parents=True)
        (policy_dir / "loadgen.log").touch()
        
        # Create CSV with low goodput (1)
        # Expected is 100 (+/- 20%)
        csv_path = policy_dir / "r100_api1.csv"
        with open(csv_path, "w") as f:
            f.write("0,0,0,0,0,0,/None\n")
                
        assert verify_standard_workload(mock_config) is False

class TestMssimSmoke:
    @pytest.fixture
    def mock_config(self, tmp_path):
        config = MagicMock(spec=ExperimentConfig)
        config.experiment_name = "test_mssim"
        config.app_name = "mssim"
        config.policies = ["policy1"]
        config.get_repeats.return_value = 1
        config.out_dir = tmp_path / "out"
        config.gen_config = {"Rps": [100], "DurationSecs": 1, "Repeats": 1}
        return config

    def test_mssim_verify_results_success(self, mock_config, tmp_path):
        from exp.runner.apps.mssim import MssimApp
        app = MssimApp()
        
        # Setup directory structure for MSSIM
        # {out_dir}/{iteration}/{policy}/run_0/
        run_dir = mock_config.out_dir / "0" / "policy1" / "run_0"
        run_dir.mkdir(parents=True)
        (mock_config.out_dir / "done").touch()
        (run_dir / "metadata.json").touch()
        
        # Create CSV with valid goodput
        # Target RPS = 100, Duration = 1 -> Target Total = 100
        # MSSIM uses root_latencies_{rps}rps.csv
        # Format: index 2 is is_err (false for success)
        csv_path = run_dir / "root_latencies_100rps.csv"
        with open(csv_path, "w") as f:
            for _ in range(100):
                f.write("0,0,false\n")
                
        assert app.verify_results(mock_config) is True

    def test_mssim_verify_results_failure_missing_metadata(self, mock_config, tmp_path):
        from exp.runner.apps.mssim import MssimApp
        app = MssimApp()
        
        run_dir = mock_config.out_dir / "0" / "policy1" / "run_0"
        run_dir.mkdir(parents=True)
        (mock_config.out_dir / "done").touch()
        # Missing metadata.json
        
        assert app.verify_results(mock_config) is False

    def test_mssim_verify_results_failure_goodput(self, mock_config, tmp_path):
        from exp.runner.apps.mssim import MssimApp
        app = MssimApp()
        
        run_dir = mock_config.out_dir / "0" / "policy1" / "run_0"
        run_dir.mkdir(parents=True)
        (mock_config.out_dir / "done").touch()
        (run_dir / "metadata.json").touch()
        
        # Create CSV with low goodput
        csv_path = run_dir / "root_latencies_100rps.csv"
        with open(csv_path, "w") as f:
            f.write("0,0,false\n")
                
        assert app.verify_results(mock_config) is False
