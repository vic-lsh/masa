#!/usr/bin/env python3
"""
Example script demonstrating how to use the LoadGenerator interface.

This shows how load generators can be used independently of the full
experiment runner, which is useful for testing or manual runs.
"""

import logging
from pathlib import Path

from apps import get_app_plugin

# Setup logging
logging.basicConfig(
    level=logging.INFO,
    format='%(asctime)s - %(name)s - %(levelname)s - %(message)s',
)

logger = logging.getLogger(__name__)


def run_hotel_loadgen():
    """Example: Run hotel load generator."""
    logger.info("Running hotel load generator example")
    
    # Get the hotel app plugin
    app = get_app_plugin("hotel")
    
    # Create the load generator
    loadgen = app.create_load_generator()
    
    # Configure output directory
    output_dir = Path("/tmp/masa-hotel-loadgen-test")
    
    # Optional: Set environment variables
    env_vars = {
        "FRONTEND_PORT": "8659",
    }
    
    # Run the load generator
    try:
        loadgen.run(output_dir=output_dir, env_vars=env_vars)
        logger.info(f"Load generator completed. Check output at: {output_dir}")
    except Exception as e:
        logger.error(f"Load generator failed: {e}")
        raise


def run_synthetic_loadgen():
    """Example: Run synthetic load generator."""
    logger.info("Running synthetic load generator example")
    
    # Get the synthetic app plugin
    app = get_app_plugin("synthetic")
    
    # Create the load generator
    loadgen = app.create_load_generator()
    
    # Configure output directory
    output_dir = Path("/tmp/masa-synthetic-loadgen-test")
    
    # Optional: Set environment variables
    env_vars = {
        "FRONTEND_PORT": "8658",
    }
    
    # Run the load generator
    try:
        loadgen.run(output_dir=output_dir, env_vars=env_vars)
        logger.info(f"Load generator completed. Check output at: {output_dir}")
    except Exception as e:
        logger.error(f"Load generator failed: {e}")
        raise


def inspect_loadgen_config():
    """Example: Inspect load generator configuration without running."""
    logger.info("Inspecting load generator configurations")
    
    for app_name in ["hotel", "synthetic"]:
        app = get_app_plugin(app_name)
        loadgen = app.create_load_generator()
        
        logger.info(f"\n{app_name.upper()} Load Generator Configuration:")
        logger.info(f"  Container Name: {loadgen.get_container_name()}")
        logger.info(f"  Network Name:   {loadgen.get_network_name()}")
        logger.info(f"  Image Name:     {loadgen.get_image_name()}")
        logger.info(f"  Binary Name:    {loadgen.get_binary_name()}")
        logger.info(f"  Trace Path:     {loadgen.get_container_trace_path()}")


if __name__ == "__main__":
    import sys
    
    if len(sys.argv) < 2:
        print("Usage: python example_loadgen.py [hotel|synthetic|inspect]")
        sys.exit(1)
    
    command = sys.argv[1].lower()
    
    if command == "hotel":
        run_hotel_loadgen()
    elif command == "synthetic":
        run_synthetic_loadgen()
    elif command == "inspect":
        inspect_loadgen_config()
    else:
        print(f"Unknown command: {command}")
        print("Available commands: hotel, synthetic, inspect")
        sys.exit(1)
