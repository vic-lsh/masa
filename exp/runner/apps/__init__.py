"""
Application plugins for the experiment runner.
"""

from .base import AppBuilder, AppPlugin, DockerConfig, LoadGenerator
from .hotel import HotelApp, HotelLoadGenerator
from .mssim import MssimApp
from .synthetic import SyntheticApp, SyntheticLoadGenerator

__all__ = [
    "AppBuilder",
    "AppPlugin",
    "DockerConfig", 
    "LoadGenerator",
    "HotelApp",
    "HotelLoadGenerator",
    "MssimApp",
    "SyntheticApp",
    "SyntheticLoadGenerator",
]


def get_app_plugin(app_name: str) -> AppPlugin:
    """
    Factory function to get the appropriate app plugin.
    
    Args:
        app_name: Name of the application (hotel, synthetic)
        
    Returns:
        AppPlugin instance for the specified application
        
    Raises:
        ValueError: If app_name is not supported
    """
    apps = {
        "hotel": HotelApp,
        "mssim": MssimApp,
        "synthetic": SyntheticApp,
    }
    
    if app_name not in apps:
        raise ValueError(
            f"Unsupported application: {app_name}. "
            f"Available apps: {', '.join(apps.keys())}"
        )
    
    return apps[app_name]()
