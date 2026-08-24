import os
import re
import subprocess
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[1]
RUNNER_COMMAND = re.compile(
    r"exec uv run -m exp_runner run (?P<app>[a-z0-9_-]+) "
    r'(?P<experiment>[a-z0-9_-]+) "\$@"'
)


def test_eval_wrappers_are_documented_executable_shell_scripts() -> None:
    wrappers = sorted((REPO_ROOT / "eval").rglob("run.sh"))

    assert wrappers
    for wrapper in wrappers:
        assert (wrapper.parent / "README.md").is_file()
        assert os.access(wrapper, os.X_OK)
        result = subprocess.run(
            ["bash", "-n", str(wrapper)],
            check=False,
            capture_output=True,
            text=True,
        )
        assert result.returncode == 0, result.stderr


def test_eval_wrapper_names_match_their_runner_configs() -> None:
    for wrapper in sorted((REPO_ROOT / "eval").rglob("run.sh")):
        match = RUNNER_COMMAND.search(wrapper.read_text(encoding="utf-8"))

        assert match, f"Could not find a thin runner invocation in {wrapper}"
        app = match.group("app")
        experiment = match.group("experiment")
        assert experiment == wrapper.parent.name
        assert (REPO_ROOT / "exp" / app / "in" / experiment).is_dir()
