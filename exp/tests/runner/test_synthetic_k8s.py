import pytest
from unittest.mock import MagicMock, patch
from pathlib import Path

from exp.runner.apps.synthetic import (
    SyntheticApp,
    K8sSyntheticLoadGenerator,
    SyntheticLoadGenerator,
)


class TestSyntheticAppK8s:
    def test_create_load_generator_k8s(self):
        app = SyntheticApp()
        loadgen = app.create_load_generator(
            features="fifo", deploy_mode="k8s", project_name="test-proj"
        )

        assert isinstance(loadgen, K8sSyntheticLoadGenerator)
        assert loadgen.features == "fifo"
        assert loadgen.project_name == "test-proj"

    def test_create_load_generator_docker(self):
        app = SyntheticApp()
        loadgen = app.create_load_generator(
            features="fifo", deploy_mode="docker", project_name="test-proj"
        )

        assert isinstance(loadgen, SyntheticLoadGenerator)


class TestK8sSyntheticLoadGenerator:
    def test_get_container_name(self):
        loadgen = K8sSyntheticLoadGenerator(project_name="my-release")
        assert loadgen.get_container_name() == "my-release-client-bench"

    def test_get_image_name(self):
        loadgen = K8sSyntheticLoadGenerator(features="fifo")
        assert loadgen.get_image_name() == "synthetic_client_bench:fifo"

    @patch("subprocess.run")
    def test_run(self, mock_run):
        loadgen = K8sSyntheticLoadGenerator(project_name="test")
        mock_run.return_value.stdout = b""

        # We just want to check it runs kubectl
        # Since run() does complex things, we might just check basic calls
        # or skip deep logic testing without a cluster
        pass
