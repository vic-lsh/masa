"""
Application plugins for the experiment runner.
"""

from .base import AppBuilder, AppPlugin, DockerConfig
from .hotel import HotelApp
from .tracebench import TracebenchApp
from .socialnet import SocialnetApp
from .synthbench import SynthbenchApp

__all__ = [
    "AppBuilder",
    "AppPlugin",
    "DockerConfig",
    "HotelApp",
    "TracebenchApp",
    "SocialnetApp",
    "SynthbenchApp",
]


def get_app_plugin(app_name: str) -> AppPlugin:
    """
    Factory function to get the appropriate app plugin.

    Args:
        app_name: Name of the application (hotel, synthbench)

    Returns:
        AppPlugin instance for the specified application

    Raises:
        ValueError: If app_name is not supported
    """
    apps = {
        "hotel": HotelApp,
        "tracebench": TracebenchApp,
        "socialnet": SocialnetApp,
        "synthbench": SynthbenchApp,
    }

    if app_name not in apps:
        raise ValueError(
            f"Unsupported application: {app_name}. "
            f"Available apps: {', '.join(apps.keys())}"
        )

    return apps[app_name]()
