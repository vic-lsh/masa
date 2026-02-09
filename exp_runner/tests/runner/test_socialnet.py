"""
Tests for the socialnet application module.
"""

import tempfile
from pathlib import Path

from exp_runner.runner.apps.socialnet import SocialnetBuilder
from exp_runner.runner.cli import create_parser
from exp_runner.runner.executor import MockCommandExecutor


class TestSocialnetBuilder:
    @patch("exp_runner.runner.executor.subprocess.run")
    def test_build_with_features(self, mock_subprocess):
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
