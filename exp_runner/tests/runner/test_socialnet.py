"""
Tests for the socialnet application module.
"""

import tempfile
import yaml
from pathlib import Path

from exp_runner.runner.apps.socialnet import SocialnetApp, SocialnetBuilder
from exp_runner.runner.cli import create_parser
from exp_runner.runner.executor import MockCommandExecutor


class TestSocialnetBuilder:
    def test_build_with_features(self):
        builder = SocialnetBuilder()
        executor = MockCommandExecutor()

        with tempfile.TemporaryDirectory() as tmpdir:
            repo_root = Path(tmpdir) / "repo"
            app_dir = Path(tmpdir) / "app"
            repo_root.mkdir()
            app_dir.mkdir()

            app_config_path = repo_root / "socialnet.json"
            gen_config_path = repo_root / "gen_config.json"
            features = "policy-a,policy-b"

            app_config_path.write_text("{}")
            gen_config_path.write_text("{}")

            builder.build(
                repo_root=repo_root,
                app_dir=app_dir,
                features=features,
                rust_log="info",
                no_cache=False,
                gen_config_path=gen_config_path,
                executor=executor,
            )

            all_cmds = [record.args for record in executor.history]
            assert all_cmds

            builder_cmd = all_cmds[0]
            assert "-t" in builder_cmd
            tag_index = builder_cmd.index("-t") + 1
            assert builder_cmd[tag_index] == "socialnet_builder:policy-a-policy-b"

            gen_arg = f"GEN_CONFIG_PATH={gen_config_path.relative_to(repo_root)}"
            binary_arg = "BINARY_NAME=compose_post_server"

            flat_cmds = [" ".join(cmd) for cmd in all_cmds]
            assert any(gen_arg in cmd for cmd in flat_cmds)
            assert any(binary_arg in cmd for cmd in flat_cmds)


def test_cli_accepts_socialnet():
    parser = create_parser()
    args = parser.parse_args(["run", "socialnet", "exp1"])
    assert args.app == "socialnet"


def test_get_required_images_skips_public_infra_in_ci(monkeypatch):
    app = SocialnetApp()
    monkeypatch.setenv("CI", "true")

    images = app.get_required_images("sched_slo")

    assert "mongo:7.0" not in images
    assert "redis:7.2" not in images
    assert "memcached:1.6" not in images
    assert "rabbitmq:3.13-management" not in images
    assert "frontend_server:sched_slo" in images


def test_prepare_k8s_workload_uses_low_ci_resource_requests(tmp_path, monkeypatch):
    app = SocialnetApp()
    monkeypatch.setenv("CI", "true")
    policy_params_path = tmp_path / "policy_param.json"
    policy_params_path.write_text("{}")
    env_vars = {}

    app._prepare_k8s_workload(
        output_dir=tmp_path,
        project_name="sn-ci",
        app_config={},
        policy_params_path=policy_params_path,
        image_tag="test-tag",
        log_level="info",
        jwt_secret="test-secret",
        env_vars=env_vars,
    )

    values = yaml.safe_load((tmp_path / "values.yaml").read_text())
    assert values["defaultServiceResources"]["requests"] == {
        "cpu": "25m",
        "memory": "64Mi",
    }
    assert env_vars["HELM_VALUES_FILE"] == str((tmp_path / "values.yaml").resolve())
