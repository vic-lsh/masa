"""
Command execution abstraction for exp_runner.

This module provides an interface for executing shell commands, allowing for
easier testing and dry-run capabilities by abstracting away the direct use
of subprocess.
"""

import logging
import shlex
import subprocess
from abc import ABC, abstractmethod
from dataclasses import dataclass
from pathlib import Path
from typing import IO, Dict, List, Optional, Sequence, Union

logger = logging.getLogger(__name__)


class CommandExecutor(ABC):
    """Abstract interface for executing commands."""

    @abstractmethod
    def run(
        self,
        args: Sequence[str],
        *,
        cwd: Optional[Union[str, Path]] = None,
        check: bool = False,
        capture_output: bool = False,
        text: bool = False,
        env: Optional[Dict[str, str]] = None,
        stdout: Optional[Union[int, IO]] = None,
        stderr: Optional[Union[int, IO]] = None,
    ) -> subprocess.CompletedProcess:
        """
        Run a command.
        Matches subprocess.run signature for commonly used arguments.
        """
        pass

    @abstractmethod
    def popen(
        self,
        args: Sequence[str],
        *,
        cwd: Optional[Union[str, Path]] = None,
        stdout: Optional[Union[int, IO]] = None,
        stderr: Optional[Union[int, IO]] = None,
        text: bool = False,
        bufsize: int = -1,
        env: Optional[Dict[str, str]] = None,
    ) -> subprocess.Popen:
        """
        Start a process using Popen.
        Matches subprocess.Popen signature for commonly used arguments.
        """
        pass


class SubprocessExecutor(CommandExecutor):
    """Real implementation using subprocess."""

    def run(
        self,
        args: Sequence[str],
        *,
        cwd: Optional[Union[str, Path]] = None,
        check: bool = False,
        capture_output: bool = False,
        text: bool = False,
        env: Optional[Dict[str, str]] = None,
        stdout: Optional[Union[int, IO]] = None,
        stderr: Optional[Union[int, IO]] = None,
    ) -> subprocess.CompletedProcess:
        return subprocess.run(
            args,
            cwd=cwd,
            check=check,
            capture_output=capture_output,
            text=text,
            env=env,
            stdout=stdout,
            stderr=stderr,
        )

    def popen(
        self,
        args: Sequence[str],
        *,
        cwd: Optional[Union[str, Path]] = None,
        stdout: Optional[Union[int, IO]] = None,
        stderr: Optional[Union[int, IO]] = None,
        text: bool = False,
        bufsize: int = -1,
        env: Optional[Dict[str, str]] = None,
    ) -> subprocess.Popen:
        return subprocess.Popen(
            args,
            cwd=cwd,
            stdout=stdout,
            stderr=stderr,
            text=text,
            bufsize=bufsize,
            env=env,
        )


@dataclass
class RecordedCommand:
    """A command recorded by the MockCommandExecutor."""

    args: List[str]
    cwd: Optional[Union[str, Path]]
    env: Optional[Dict[str, str]]


class MockCommandExecutor(CommandExecutor):
    """
    Mock implementation for testing and dry-runs.
    Records commands instead of executing them.
    """

    def __init__(self):
        self.history: List[RecordedCommand] = []
        self.mock_responses: Dict[str, subprocess.CompletedProcess] = {}

    def run(
        self,
        args: Sequence[str],
        *,
        cwd: Optional[Union[str, Path]] = None,
        check: bool = False,
        capture_output: bool = False,
        text: bool = False,
        env: Optional[Dict[str, str]] = None,
        stdout: Optional[Union[int, IO]] = None,
        stderr: Optional[Union[int, IO]] = None,
    ) -> subprocess.CompletedProcess:
        # Convert args to list of strings
        cmd_list = [str(arg) for arg in args]

        # Record command
        self.history.append(RecordedCommand(cmd_list, cwd, env))

        cmd_str = shlex.join(cmd_list)
        logger.info(f"[MockExecutor] Running: {cmd_str}")

        # Return mock response if configured
        # Simple exact match on command string or first arg
        if cmd_str in self.mock_responses:
            return self.mock_responses[cmd_str]

        # Default success response
        return subprocess.CompletedProcess(
            args=cmd_list,
            returncode=0,
            stdout="Mock stdout" if text else b"Mock stdout",
            stderr="Mock stderr" if text else b"Mock stderr",
        )

    def popen(
        self,
        args: Sequence[str],
        *,
        cwd: Optional[Union[str, Path]] = None,
        stdout: Optional[Union[int, IO]] = None,
        stderr: Optional[Union[int, IO]] = None,
        text: bool = False,
        bufsize: int = -1,
        env: Optional[Dict[str, str]] = None,
    ) -> subprocess.Popen:
        cmd_list = [str(arg) for arg in args]
        self.history.append(RecordedCommand(cmd_list, cwd, env))

        logger.info(f"[MockExecutor] Popen: {shlex.join(cmd_list)}")

        # Return a dummy mock object that behaves like Popen
        # We need to mock Popen behavior properly for things like stdout.readline
        mock_popen = subprocess.Popen.__new__(subprocess.Popen)
        mock_popen.args = cmd_list
        mock_popen.returncode = 0

        # Mock stdout stream
        if stdout == subprocess.PIPE:
            from io import BytesIO, StringIO

            if text:
                mock_popen.stdout = StringIO("Mock stdout line 1\nMock stdout line 2\n")
            else:
                mock_popen.stdout = BytesIO(b"Mock stdout line 1\nMock stdout line 2\n")
        else:
            mock_popen.stdout = None

        mock_popen.stderr = None
        mock_popen.wait = lambda timeout=None: 0
        mock_popen.communicate = lambda input=None, timeout=None: (b"", b"")
        mock_popen.poll = lambda: 0
        mock_popen.kill = lambda: None
        mock_popen.terminate = lambda: None

        return mock_popen
