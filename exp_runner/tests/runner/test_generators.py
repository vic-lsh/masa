"""
Tests for deployment generators (ComposeGenerator, HelmValuesGenerator).
"""

import tempfile
from pathlib import Path

import pytest
import yaml

from exp_runner.runner.experiment_config_v2 import (
    ApiSpec,
    ExperimentConfigV2,
    ExecutionSpec,
    LoadGenSpec,
)
from exp_runner.runner.generators.compose import ComposeGenerator
from exp_runner.runner.generators.helm import HelmValuesGenerator
from exp_runner.runner.topology import (
    ServiceSpec,
    TopologySpec,
    format_call_graph_service_name,
)


@pytest.fixture
def simple_topology() -> TopologySpec:
    """Create a simple topology for testing."""
    return TopologySpec(
        app="synthetic",
        description="Test topology",
        services={
            "frontend": ServiceSpec(
                id="frontend",
                port=8000,
                default_replicas=1,
                depends_on=["child-service"],
            ),
            "child-service": ServiceSpec(
                id="child-service",
                port=8080,
                default_replicas=2,
            ),
        },
        infrastructure={
            "redis": ServiceSpec(
                id="redis",
                image="redis:7.2",
                port=6379,
                default_replicas=1,
            ),
        },
    )


@pytest.fixture
def hotel_topology() -> TopologySpec:
    """Create a hotel-like topology for testing."""
    return TopologySpec(
        app="hotel",
        description="Hotel reservation app",
        services={
            "frontend": ServiceSpec(
                id="frontend",
                port=8660,
                default_replicas=1,
                depends_on=["rate-service", "profile-service"],
            ),
            "rate-service": ServiceSpec(
                id="rate-service",
                port=8080,
                default_replicas=1,
                depends_on=["rate-mongo", "rate-redis"],
            ),
            "profile-service": ServiceSpec(
                id="profile-service",
                port=8080,
                default_replicas=1,
                depends_on=["profile-mongo"],
            ),
        },
        infrastructure={
            "rate-mongo": ServiceSpec(
                id="rate-mongo",
                image="mongo:7.0",
                port=27017,
                default_replicas=1,
            ),
            "rate-redis": ServiceSpec(
                id="rate-redis",
                image="redis:7.2",
                port=6379,
                default_replicas=1,
            ),
            "profile-mongo": ServiceSpec(
                id="profile-mongo",
                image="mongo:7.0",
                port=27017,
                default_replicas=1,
            ),
        },
    )


@pytest.fixture
def simple_experiment() -> ExperimentConfigV2:
    """Create a simple experiment config for testing."""
    return ExperimentConfigV2(
        name="test-exp",
        app="synthetic",
        execution=ExecutionSpec(
            repeats=1,
            policies=["fifo", "prio_global"],
            warmup_secs=10,
            duration_secs=60,
        ),
        loadgen=LoadGenSpec(
            rps=[100.0, 200.0],
            default_timeout_ms=1000,
            apis=[
                ApiSpec(name="test_api", slo_us=10000, weight=1.0),
            ],
        ),
    )


@pytest.fixture
def experiment_with_overrides() -> ExperimentConfigV2:
    """Create an experiment with replica overrides."""
    return ExperimentConfigV2(
        name="test-exp-overrides",
        app="synthetic",
        replica_overrides={"child-service": 4},
        execution=ExecutionSpec(
            repeats=1,
            policies=["fifo"],
            warmup_secs=5,
            duration_secs=30,
        ),
        loadgen=LoadGenSpec(
            rps=[50.0],
            default_timeout_ms=2000,
        ),
    )


@pytest.fixture
def synthetic_call_graph_topology() -> TopologySpec:
    """Create a synthetic topology with a call graph."""
    return TopologySpec(
        app="synthetic",
        description="call graph topology",
        call_graph={
            "entry_points": {"api_a": [{"MS_root::handle": 1.0}]},
            "services": [
                {"id": "MS_root", "default_replicas": 1},
                {"id": "MS_child1", "default_replicas": 2},
            ],
            "child_cpus_per_replica": 0.5,
        },
    )


class TestComposeGenerator:
    """Tests for ComposeGenerator."""

    def test_generate_basic_compose(
        self, simple_topology: TopologySpec, simple_experiment: ExperimentConfigV2
    ):
        """Test generating a basic docker-compose.yaml."""
        generator = ComposeGenerator()

        with tempfile.TemporaryDirectory() as tmpdir:
            output_dir = Path(tmpdir)

            result = generator.generate(
                topology=simple_topology,
                experiment=simple_experiment,
                output_dir=output_dir,
                project_name="test-project",
                policy="fifo",
                image_tag="latest",
            )

            # Check returned structure
            assert result.deploy_root == output_dir
            assert result.deploy_file == "docker-compose.yaml"
            assert "DOCKER_COMPOSE_PROJECT_NAME" in result.env_vars
            assert result.env_vars["DOCKER_COMPOSE_PROJECT_NAME"] == "test-project"

            # Check file was created
            compose_path = output_dir / "docker-compose.yaml"
            assert compose_path.exists()

            # Parse and validate structure
            with open(compose_path) as f:
                compose_dict = yaml.safe_load(f)

            assert "services" in compose_dict
            assert "networks" in compose_dict

            # Check services exist
            assert "frontend" in compose_dict["services"]
            assert "child-service" in compose_dict["services"]
            assert "redis" in compose_dict["services"]

            # Check frontend service structure
            frontend = compose_dict["services"]["frontend"]
            assert "image" in frontend
            assert "networks" in frontend
            assert "depends_on" in frontend
            assert "child-service" in frontend["depends_on"]

            # Check child service has correct replicas
            child = compose_dict["services"]["child-service"]
            assert child["scale"] == 2

            # Check infrastructure service
        redis = compose_dict["services"]["redis"]
        assert redis["image"] == "redis:7.2"
        assert "volumes" in compose_dict  # Should have redis-data volume

    def test_generate_call_graph_compose(
        self,
        synthetic_call_graph_topology: TopologySpec,
        simple_experiment: ExperimentConfigV2,
    ):
        """Ensure call-graph topologies produce frontend + child services."""
        generator = ComposeGenerator()

        with tempfile.TemporaryDirectory() as tmpdir:
            output_dir = Path(tmpdir)

            result = generator.generate(
                topology=synthetic_call_graph_topology,
                experiment=simple_experiment,
                output_dir=output_dir,
                project_name="test-project",
                policy="fifo",
                image_tag="latest",
            )
            assert result.deploy_file == "docker-compose.yaml"

            compose_path = output_dir / "docker-compose.yaml"
            with open(compose_path) as f:
                compose_dict = yaml.safe_load(f)

            services = compose_dict["services"]
            frontend_name = "synthetic-frontend-service"
            root_name = format_call_graph_service_name("MS_root")
            child_name = format_call_graph_service_name("MS_child1")

            assert frontend_name in services
            assert root_name in services
            assert child_name in services

            child_service = services[child_name]
            assert child_service["scale"] == 2
            env_list = child_service["environment"]
            assert any(entry == "SERVICE_ID=MS_child1" for entry in env_list)

            frontend = services[frontend_name]
            assert "depends_on" in frontend
            assert set(frontend["depends_on"]) == {
                root_name,
                child_name,
            }

    def test_generate_with_replica_overrides(
        self, simple_topology: TopologySpec, experiment_with_overrides: ExperimentConfigV2
    ):
        """Test that replica overrides are applied correctly."""
        generator = ComposeGenerator()

        with tempfile.TemporaryDirectory() as tmpdir:
            output_dir = Path(tmpdir)

            result = generator.generate(
                topology=simple_topology,
                experiment=experiment_with_overrides,
                output_dir=output_dir,
                project_name="test-project",
                policy="fifo",
                image_tag="latest",
            )

            compose_path = output_dir / "docker-compose.yaml"
            with open(compose_path) as f:
                compose_dict = yaml.safe_load(f)

            # Check that override was applied
            child = compose_dict["services"]["child-service"]
            assert child["scale"] == 4  # Overridden from default 2

    def test_generate_hotel_topology(
        self, hotel_topology: TopologySpec, simple_experiment: ExperimentConfigV2
    ):
        """Test generating compose for hotel-like topology with mongo and redis."""
        generator = ComposeGenerator()

        with tempfile.TemporaryDirectory() as tmpdir:
            output_dir = Path(tmpdir)

            result = generator.generate(
                topology=hotel_topology,
                experiment=simple_experiment,
                output_dir=output_dir,
                project_name="hotel-test",
                policy="fifo",
                image_tag="latest",
            )

            compose_path = output_dir / "docker-compose.yaml"
            with open(compose_path) as f:
                compose_dict = yaml.safe_load(f)

            # Check all services exist
            services = compose_dict["services"]
            assert "frontend" in services
            assert "rate-service" in services
            assert "profile-service" in services
            assert "rate-mongo" in services
            assert "rate-redis" in services
            assert "profile-mongo" in services

            # Check rate-service dependencies
            rate_service = services["rate-service"]
            assert "rate-mongo" in rate_service["depends_on"]
            assert "rate-redis" in rate_service["depends_on"]

            # Check infrastructure images
            assert services["rate-mongo"]["image"] == "mongo:7.0"
            assert services["rate-redis"]["image"] == "redis:7.2"

            # Check volumes for databases
            volumes = compose_dict.get("volumes", {})
            assert len(volumes) > 0  # Should have volumes for mongo and redis

    def test_validate_topology_no_app(self):
        """Test that validation fails for topology without app name."""
        generator = ComposeGenerator()
        topology = TopologySpec(app="", services={})

        with pytest.raises(ValueError, match="must specify an app name"):
            generator.validate_topology(topology)

    def test_validate_topology_empty(self):
        """Test that validation fails for completely empty topology."""
        generator = ComposeGenerator()
        topology = TopologySpec(app="test", services={}, infrastructure={})

        with pytest.raises(ValueError, match="at least one service or infrastructure"):
            generator.validate_topology(topology)

    def test_validate_experiment_invalid_override(
        self, simple_topology: TopologySpec, simple_experiment: ExperimentConfigV2
    ):
        """Test that validation fails for invalid replica override."""
        generator = ComposeGenerator()

        # Add invalid override
        simple_experiment.replica_overrides = {"nonexistent-service": 5}

        with pytest.raises(ValueError, match="unknown service"):
            generator.validate_experiment(simple_experiment, simple_topology)


class TestHelmValuesGenerator:
    """Tests for HelmValuesGenerator."""

    def test_generate_basic_values(
        self, simple_topology: TopologySpec, simple_experiment: ExperimentConfigV2
    ):
        """Test generating basic Helm values.yaml."""
        generator = HelmValuesGenerator()

        with tempfile.TemporaryDirectory() as tmpdir:
            output_dir = Path(tmpdir)

            result = generator.generate(
                topology=simple_topology,
                experiment=simple_experiment,
                output_dir=output_dir,
                project_name="test-release",
                policy="fifo",
                image_tag="v1.0",
            )

            # Check returned structure
            assert result.deploy_root == output_dir
            assert result.deploy_file == "values.yaml"
            assert "HELM_VALUES_FILE" in result.env_vars

            # Check file was created
            values_path = output_dir / "values.yaml"
            assert values_path.exists()

            # Parse and validate structure
            with open(values_path) as f:
                values_dict = yaml.safe_load(f)

            assert "fullnameOverride" in values_dict
            assert values_dict["fullnameOverride"] == "test-release"

            assert "image" in values_dict
            assert values_dict["image"]["tag"] == "v1.0"
            assert values_dict["image"]["pullPolicy"] == "IfNotPresent"

            assert "services" in values_dict
            assert "frontend" in values_dict["services"]
            assert "child-service" in values_dict["services"]

            # Check service configuration
            frontend = values_dict["services"]["frontend"]
            assert frontend["replicas"] == 1
            assert frontend["port"] == 8000

            child = values_dict["services"]["child-service"]
            assert child["replicas"] == 2
            assert child["port"] == 8080

            # Check infrastructure
            assert "infrastructure" in values_dict
            assert "redis" in values_dict["infrastructure"]
            redis = values_dict["infrastructure"]["redis"]
            assert redis["image"] == "redis:7.2"
            assert redis["port"] == 6379

    def test_generate_with_replica_overrides(
        self, simple_topology: TopologySpec, experiment_with_overrides: ExperimentConfigV2
    ):
        """Test that replica overrides are applied in Helm values."""
        generator = HelmValuesGenerator()

        with tempfile.TemporaryDirectory() as tmpdir:
            output_dir = Path(tmpdir)

            result = generator.generate(
                topology=simple_topology,
                experiment=experiment_with_overrides,
                output_dir=output_dir,
                project_name="test-release",
                policy="fifo",
                image_tag="latest",
            )

            values_path = output_dir / "values.yaml"
            with open(values_path) as f:
                values_dict = yaml.safe_load(f)

            # Check that override was applied
            child = values_dict["services"]["child-service"]
            assert child["replicas"] == 4  # Overridden from default 2

    def test_generate_with_app_config(
        self, simple_topology: TopologySpec, simple_experiment: ExperimentConfigV2
    ):
        """Test that app config is included in values."""
        generator = HelmValuesGenerator()

        with tempfile.TemporaryDirectory() as tmpdir:
            output_dir = Path(tmpdir)

            result = generator.generate(
                topology=simple_topology,
                experiment=simple_experiment,
                output_dir=output_dir,
                project_name="test-release",
                policy="fifo",
                image_tag="latest",
            )

            values_path = output_dir / "values.yaml"
            with open(values_path) as f:
                values_dict = yaml.safe_load(f)

            # Check app config includes API specs
            assert "appConfig" in values_dict
            app_config = values_dict["appConfig"]
            assert "apis" in app_config
            assert len(app_config["apis"]) == 1
            assert app_config["apis"][0]["name"] == "test_api"
            assert app_config["apis"][0]["slo_us"] == 10000

    def test_generate_synthetic_image_names(
        self, simple_topology: TopologySpec, simple_experiment: ExperimentConfigV2
    ):
        """Test that synthetic app gets correct image names."""
        generator = HelmValuesGenerator()

        with tempfile.TemporaryDirectory() as tmpdir:
            output_dir = Path(tmpdir)

            result = generator.generate(
                topology=simple_topology,
                experiment=simple_experiment,
                output_dir=output_dir,
                project_name="test-release",
                policy="fifo",
                image_tag="latest",
            )

            values_path = output_dir / "values.yaml"
            with open(values_path) as f:
                values_dict = yaml.safe_load(f)

            # Check synthetic-specific image names
            image = values_dict["image"]
            assert image["frontendName"] == "synthetic_frontend"
            assert image["childName"] == "synthetic_child"

    def test_validate_topology_no_services(self):
        """Test that validation fails for topology with no services."""
        generator = HelmValuesGenerator()
        topology = TopologySpec(app="test", services={})

        with pytest.raises(ValueError, match="at least one service"):
            generator.validate_topology(topology)

    def test_validate_experiment_no_execution(
        self, simple_topology: TopologySpec
    ):
        """Test that validation fails for experiment without execution spec."""
        generator = HelmValuesGenerator()

        # Create experiment without execution spec
        experiment = ExperimentConfigV2(
            name="test",
            app="test",
            execution=None,  # type: ignore
            loadgen=LoadGenSpec(rps=[100.0]),
        )

        with pytest.raises(ValueError, match="execution specification"):
            generator.validate_experiment(experiment, simple_topology)

    def test_resource_limits_structure(
        self, simple_topology: TopologySpec, simple_experiment: ExperimentConfigV2
    ):
        """Test that resource limits are properly structured."""
        generator = HelmValuesGenerator()

        with tempfile.TemporaryDirectory() as tmpdir:
            output_dir = Path(tmpdir)

            result = generator.generate(
                topology=simple_topology,
                experiment=simple_experiment,
                output_dir=output_dir,
                project_name="test-release",
                policy="fifo",
                image_tag="latest",
            )

            values_path = output_dir / "values.yaml"
            with open(values_path) as f:
                values_dict = yaml.safe_load(f)

            # Check resources structure
            assert "resources" in values_dict
            resources = values_dict["resources"]
            assert "default" in resources
            assert "frontend" in resources
            assert "infrastructure" in resources

            # Check default has limits and requests
            default = resources["default"]
            assert "limits" in default
            assert "requests" in default
            assert "cpu" in default["limits"]
            assert "memory" in default["limits"]


class TestGeneratorIntegration:
    """Integration tests comparing compose and helm outputs."""

    def test_both_generators_use_same_replicas(
        self, simple_topology: TopologySpec, experiment_with_overrides: ExperimentConfigV2
    ):
        """Test that both generators apply replica overrides consistently."""
        compose_gen = ComposeGenerator()
        helm_gen = HelmValuesGenerator()

        with tempfile.TemporaryDirectory() as tmpdir:
            output_dir = Path(tmpdir)

            # Generate compose
            compose_result = compose_gen.generate(
                topology=simple_topology,
                experiment=experiment_with_overrides,
                output_dir=output_dir / "compose",
                project_name="test",
                policy="fifo",
                image_tag="latest",
            )

            # Generate helm
            helm_result = helm_gen.generate(
                topology=simple_topology,
                experiment=experiment_with_overrides,
                output_dir=output_dir / "helm",
                project_name="test",
                policy="fifo",
                image_tag="latest",
            )

            # Parse both
            with open(output_dir / "compose" / "docker-compose.yaml") as f:
                compose_dict = yaml.safe_load(f)

            with open(output_dir / "helm" / "values.yaml") as f:
                helm_dict = yaml.safe_load(f)

            # Compare replicas
            compose_replicas = compose_dict["services"]["child-service"]["scale"]
            helm_replicas = helm_dict["services"]["child-service"]["replicas"]

            assert compose_replicas == helm_replicas == 4

    def test_both_generators_handle_infrastructure(
        self, hotel_topology: TopologySpec, simple_experiment: ExperimentConfigV2
    ):
        """Test that both generators handle infrastructure services."""
        compose_gen = ComposeGenerator()
        helm_gen = HelmValuesGenerator()

        with tempfile.TemporaryDirectory() as tmpdir:
            output_dir = Path(tmpdir)

            # Generate both
            compose_result = compose_gen.generate(
                topology=hotel_topology,
                experiment=simple_experiment,
                output_dir=output_dir / "compose",
                project_name="test",
                policy="fifo",
                image_tag="latest",
            )

            helm_result = helm_gen.generate(
                topology=hotel_topology,
                experiment=simple_experiment,
                output_dir=output_dir / "helm",
                project_name="test",
                policy="fifo",
                image_tag="latest",
            )

            # Parse both
            with open(output_dir / "compose" / "docker-compose.yaml") as f:
                compose_dict = yaml.safe_load(f)

            with open(output_dir / "helm" / "values.yaml") as f:
                helm_dict = yaml.safe_load(f)

            # Check compose has infrastructure in services
            assert "rate-mongo" in compose_dict["services"]
            assert "rate-redis" in compose_dict["services"]

            # Check helm has infrastructure section
            assert "infrastructure" in helm_dict
            assert "rate-mongo" in helm_dict["infrastructure"]
            assert "rate-redis" in helm_dict["infrastructure"]

            # Verify image names match
            assert (
                compose_dict["services"]["rate-mongo"]["image"]
                == helm_dict["infrastructure"]["rate-mongo"]["image"]
            )
