class ExpRunnerError(Exception):
    """Base exception for ExpRunner."""

    pass


class DeploymentError(ExpRunnerError):
    """Raised when deployment operations fail."""

    pass


class LoadGenError(ExpRunnerError):
    """Raised when load generation fails."""

    pass


class ConfigError(ExpRunnerError):
    """Raised when configuration is invalid."""

    pass
