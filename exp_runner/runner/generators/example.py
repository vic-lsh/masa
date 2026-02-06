"""
Example usage of deployment generators.

This script demonstrates how to use ComposeGenerator and HelmValuesGenerator
to generate deployment manifests from topology and experiment configs.
"""

import tempfile
from pathlib import Path

from exp_runner.runner.experiment_config_v2 import (
    ApiSpec,
    ExperimentConfigV2,
    ExecutionSpec,
    LoadGenSpec,
)
from exp_runner.runner.generators import ComposeGenerator, HelmValuesGenerator
from exp_runner.runner.topology import ServiceSpec, TopologySpec


def create_example_topology() -> TopologySpec:
    """Create an example topology for demonstration."""
    return TopologySpec(
        app="demo",
        description="Example microservices application",
        services={
            "frontend": ServiceSpec(
                id="frontend",
                port=8000,
                default_replicas=1,
                depends_on=["api-service", "auth-service"],
            ),
            "api-service": ServiceSpec(
                id="api-service",
                port=8080,
                default_replicas=2,
                depends_on=["database"],
            ),
            "auth-service": ServiceSpec(
                id="auth-service",
                port=8080,
                default_replicas=1,
                depends_on=["redis", "database"],
            ),
        },
        infrastructure={
            "database": ServiceSpec(
                id="database",
                image="postgres:16",
                port=5432,
                default_replicas=1,
            ),
            "redis": ServiceSpec(
                id="redis",
                image="redis:7.2",
                port=6379,
                default_replicas=1,
            ),
        },
    )


def create_example_experiment() -> ExperimentConfigV2:
    """Create an example experiment configuration."""
    return ExperimentConfigV2(
        name="load-test",
        app="demo",
        replica_overrides={
            "api-service": 4,  # Scale up API service
        },
        execution=ExecutionSpec(
            repeats=3,
            policies=["fifo", "prio_global"],
            warmup_secs=30,
            duration_secs=300,
        ),
        loadgen=LoadGenSpec(
            rps=[100.0, 200.0, 400.0],
            default_timeout_ms=5000,
            apis=[
                ApiSpec(name="list_items", slo_us=50000, weight=0.7),
                ApiSpec(name="get_item", slo_us=20000, weight=0.2),
                ApiSpec(name="create_item", slo_us=100000, weight=0.1),
            ],
        ),
    )


def main():
    """Demonstrate generator usage."""
    print("=" * 80)
    print("Deployment Generator Example")
    print("=" * 80)
    print()

    # Create example topology and experiment
    topology = create_example_topology()
    experiment = create_example_experiment()

    print(f"Application: {topology.app}")
    print(f"Services: {list(topology.services.keys())}")
    print(f"Infrastructure: {list(topology.infrastructure.keys())}")
    print(f"Experiment: {experiment.name}")
    print(f"Policies: {experiment.execution.policies}")
    print(f"Replica overrides: {experiment.replica_overrides}")
    print()

    with tempfile.TemporaryDirectory() as tmpdir:
        output_dir = Path(tmpdir)

        # Generate Docker Compose deployment
        print("-" * 80)
        print("Generating Docker Compose deployment...")
        print("-" * 80)

        compose_gen = ComposeGenerator()
        compose_result = compose_gen.generate(
            topology=topology,
            experiment=experiment,
            output_dir=output_dir / "compose",
            project_name="demo-load-test",
            policy="fifo",
            image_tag="v1.0.0",
        )

        compose_path = output_dir / "compose" / compose_result.deploy_file
        print(f"Generated: {compose_path}")
        print()
        print("Content preview:")
        with open(compose_path) as f:
            content = f.read()
            lines = content.split("\n")
            for i, line in enumerate(lines[:30], 1):
                print(f"{i:3}: {line}")
            if len(lines) > 30:
                print(f"... ({len(lines) - 30} more lines)")
        print()

        # Generate Helm values
        print("-" * 80)
        print("Generating Helm values...")
        print("-" * 80)

        helm_gen = HelmValuesGenerator()
        helm_result = helm_gen.generate(
            topology=topology,
            experiment=experiment,
            output_dir=output_dir / "helm",
            project_name="demo-load-test",
            policy="fifo",
            image_tag="v1.0.0",
        )

        helm_path = output_dir / "helm" / helm_result.deploy_file
        print(f"Generated: {helm_path}")
        print()
        print("Content preview:")
        with open(helm_path) as f:
            content = f.read()
            lines = content.split("\n")
            for i, line in enumerate(lines[:30], 1):
                print(f"{i:3}: {line}")
            if len(lines) > 30:
                print(f"... ({len(lines) - 30} more lines)")
        print()

        print("=" * 80)
        print("Example completed successfully!")
        print("=" * 80)


if __name__ == "__main__":
    main()
