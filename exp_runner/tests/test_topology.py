"""Tests for topology module."""

import pytest
import tempfile
from pathlib import Path

from exp_runner.runner.topology import ServiceSpec, TopologySpec, TopologyResolver


def test_service_spec_basic():
    """Test basic ServiceSpec creation."""
    svc = ServiceSpec(id="frontend", port=8080, default_replicas=2)
    assert svc.id == "frontend"
    assert svc.port == 8080
    assert svc.default_replicas == 2
    assert svc.depends_on == []


def test_topology_spec_from_dict():
    """Test TopologySpec creation from dictionary."""
    data = {
        "kind": "Topology",
        "metadata": {
            "app": "hotel",
            "description": "Hotel reservation system",
        },
        "services": {
            "frontend": {
                "port": 8660,
                "default_replicas": 1,
            },
            "rate": {
                "port": 8663,
                "default_replicas": 2,
                "depends_on": ["rate-mongo", "rate-redis"],
            },
        },
        "infrastructure": {
            "rate-mongo": {
                "image": "mongo:7.0",
                "port": 27003,
            },
        },
    }

    topology = TopologySpec.from_dict(data)
    assert topology.app == "hotel"
    assert topology.description == "Hotel reservation system"
    assert "frontend" in topology.services
    assert "rate" in topology.services
    assert topology.services["frontend"].port == 8660
    assert topology.services["rate"].default_replicas == 2
    assert topology.services["rate"].depends_on == ["rate-mongo", "rate-redis"]
    assert "rate-mongo" in topology.infrastructure
    assert topology.infrastructure["rate-mongo"].image == "mongo:7.0"


def test_topology_spec_to_dict():
    """Test TopologySpec serialization to dictionary."""
    services = {
        "frontend": ServiceSpec(id="frontend", port=8080, default_replicas=1),
        "backend": ServiceSpec(
            id="backend",
            port=8081,
            default_replicas=2,
            depends_on=["db"],
        ),
    }
    infrastructure = {
        "db": ServiceSpec(id="db", port=5432, image="postgres:14"),
    }

    topology = TopologySpec(
        app="test",
        description="Test app",
        services=services,
        infrastructure=infrastructure,
    )

    data = topology.to_dict()
    assert data["kind"] == "Topology"
    assert data["metadata"]["app"] == "test"
    assert "frontend" in data["services"]
    assert data["services"]["backend"]["depends_on"] == ["db"]
    assert data["infrastructure"]["db"]["image"] == "postgres:14"


def test_topology_apply_replica_overrides():
    """Test applying replica overrides to topology."""
    services = {
        "frontend": ServiceSpec(id="frontend", default_replicas=1),
        "rate": ServiceSpec(id="rate", default_replicas=1),
        "profile": ServiceSpec(id="profile", default_replicas=1),
    }

    topology = TopologySpec(app="hotel", services=services)

    # Apply overrides
    overrides = {"rate": 3, "profile": 2}
    new_topology = topology.apply_replica_overrides(overrides)

    # Check overrides were applied
    assert new_topology.services["rate"].default_replicas == 3
    assert new_topology.services["profile"].default_replicas == 2
    assert new_topology.services["frontend"].default_replicas == 1  # unchanged

    # Original should be unchanged
    assert topology.services["rate"].default_replicas == 1


def test_topology_resolver_default():
    """Test TopologyResolver with default topology."""
    with tempfile.TemporaryDirectory() as tmpdir:
        repo_root = Path(tmpdir)
        apps_dir = repo_root / "apps" / "hotel"
        apps_dir.mkdir(parents=True)

        # Create a sample topology file
        topology_file = apps_dir / "topology.yaml"
        topology_file.write_text(
            """
kind: Topology
metadata:
  app: hotel
  description: Hotel app
services:
  frontend:
    port: 8660
    default_replicas: 1
"""
        )

        resolver = TopologyResolver(repo_root)
        topology = resolver.resolve("hotel", "default")

        assert topology.app == "hotel"
        assert "frontend" in topology.services


def test_topology_resolver_named():
    """Test TopologyResolver with named topology reference."""
    with tempfile.TemporaryDirectory() as tmpdir:
        repo_root = Path(tmpdir)
        topologies_dir = repo_root / "exp" / "synthetic" / "topologies"
        topologies_dir.mkdir(parents=True)

        # Create a sample topology file
        topology_file = topologies_dir / "fanout-3.yaml"
        topology_file.write_text(
            """
kind: Topology
metadata:
  app: synthetic
  description: 3-level fanout
call_graph:
  entry_points:
    api_a: [{MS_root::handle: 1.0}]
"""
        )

        resolver = TopologyResolver(repo_root)
        topology = resolver.resolve("synthetic", "fanout-3")

        assert topology.app == "synthetic"
        assert topology.call_graph is not None


def test_topology_resolver_not_found():
    """Test TopologyResolver raises error for missing topology."""
    with tempfile.TemporaryDirectory() as tmpdir:
        repo_root = Path(tmpdir)
        resolver = TopologyResolver(repo_root)

        with pytest.raises(FileNotFoundError):
            resolver.resolve("synthetic", "nonexistent")


def test_topology_resolver_apply_overrides():
    """Test TopologyResolver apply_overrides method."""
    services = {
        "svc1": ServiceSpec(id="svc1", default_replicas=1),
        "svc2": ServiceSpec(id="svc2", default_replicas=2),
    }
    topology = TopologySpec(app="test", services=services)

    resolver = TopologyResolver(Path("/tmp"))
    overrides = {"svc1": 5}
    new_topology = resolver.apply_overrides(topology, overrides)

    assert new_topology.services["svc1"].default_replicas == 5
    assert new_topology.services["svc2"].default_replicas == 2
