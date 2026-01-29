"""
Tests for container utilities module.

This module tests the container name parsing and grouping functionality:
- Service name extraction from container names
- Container name parsing
- Grouping containers by service
"""

import pytest
from exp.runner.container_utils import (
    extract_service_name,
    parse_container_name,
    group_containers_by_service,
)


class TestExtractServiceName:
    """Tests for extract_service_name function."""

    def test_hotel_container_names(self):
        """Test extraction from hotel container names."""
        # Hotel pattern: hotel-{slug}-{digest}-{service}-{replica}
        assert (
            extract_service_name("hotel-exp1-abc123def456-rate-service-1")
            == "rate-service"
        )
        assert (
            extract_service_name("hotel-exp1-abc123def456-search-service-2")
            == "search-service"
        )
        assert extract_service_name("hotel-exp1-abc123def456-frontend-1") == "frontend"
        assert (
            extract_service_name("hotel-test-012345678901-rate-mongo-1") == "rate-mongo"
        )

    def test_mssim_container_names(self):
        """Test extraction from MSSIM container names."""
        # MSSIM pattern: mssim-{slug}-{digest}-{service}-{replica}
        assert extract_service_name("mssim-exp1-abc123def456-frontend-1") == "frontend"
        assert (
            extract_service_name("mssim-test-012345678901-service-a-2") == "service-a"
        )
        assert extract_service_name("mssim-exp2-aabbccddee00-backend-3") == "backend"

    def test_synthetic_container_names(self):
        """Test extraction from synthetic container names."""
        # Synthetic patterns
        assert extract_service_name("local-child-service-1") == "child-service"
        assert extract_service_name("local-child-service-10") == "child-service"
        assert extract_service_name("synthetic_frontend") == "frontend"
        assert extract_service_name("synthetic-frontend-1") == "frontend"

        # Real experiment pattern: synthetic-{slug}-{digest}-{service}-{replica}
        assert (
            extract_service_name("synthetic-exp1-abc123def456-child-service-1")
            == "child-service"
        )
        assert (
            extract_service_name("synthetic-my-exp-123456789abc-frontend-service-2")
            == "frontend-service"
        )

    def test_socialnet_container_names(self):
        """Test extraction from socialnet container names."""
        # Socialnet patterns
        assert (
            extract_service_name("socialnet-user-timeline-service-1")
            == "user-timeline-service"
        )
        assert (
            extract_service_name("socialnet_network_frontend_service")
            == "network_frontend_service"
        )
        assert extract_service_name("socialnet-compose-service-2") == "compose-service"

    def test_loadgen_containers(self):
        """Test that load generator containers keep their full name."""
        assert "loadgen" in extract_service_name("hotel-exp1-abc-loadgen").lower()
        assert "client" in extract_service_name("synthetic_client_bench").lower()
        assert "loadgen" in extract_service_name("mssim-exp-abc-loadgen-1").lower()

    def test_network_containers(self):
        """Test that network containers keep their full name."""
        result = extract_service_name("hotel_network")
        assert "network" in result.lower()

    def test_no_replica_number(self):
        """Test containers without replica numbers."""
        assert extract_service_name("hotel-exp1-abc123def456-service") == "service"
        assert extract_service_name("synthetic_frontend") == "frontend"

    def test_multiple_dashes_in_service_name(self):
        """Test service names with multiple dashes."""
        assert (
            extract_service_name("hotel-exp-abc123def456-user-timeline-service-1")
            == "user-timeline-service"
        )
        assert (
            extract_service_name("mssim-exp-012345678901-my-long-service-name-5")
            == "my-long-service-name"
        )

    def test_high_replica_numbers(self):
        """Test containers with high replica numbers."""
        assert extract_service_name("local-child-service-99") == "child-service"
        assert (
            extract_service_name("hotel-exp-abc123def456-rate-service-123")
            == "rate-service"
        )


class TestParseContainerName:
    """Tests for parse_container_name function."""

    def test_basic_parsing(self):
        """Test basic container name parsing."""
        result = parse_container_name("hotel-exp1-abc123def456-rate-service-1")

        assert result["full_name"] == "hotel-exp1-abc123def456-rate-service-1"
        assert result["service_name"] == "rate-service"
        assert result["replica_num"] == 1
        assert result["is_loadgen"] is False

    def test_no_replica_number(self):
        """Test parsing container without replica number."""
        result = parse_container_name("synthetic_frontend")

        assert result["full_name"] == "synthetic_frontend"
        assert result["service_name"] == "frontend"
        assert result["replica_num"] is None
        assert result["is_loadgen"] is False

    def test_loadgen_detection(self):
        """Test load generator detection."""
        # Various loadgen patterns
        result1 = parse_container_name("hotel_client_bench")
        assert result1["is_loadgen"] is True

        result2 = parse_container_name("mssim-exp-abc-loadgen-1")
        assert result2["is_loadgen"] is True

        result3 = parse_container_name("synthetic_client_bench")
        assert result3["is_loadgen"] is True

        result4 = parse_container_name("load_generator_container")
        assert result4["is_loadgen"] is True

    def test_non_loadgen_detection(self):
        """Test that non-loadgen containers are correctly identified."""
        result = parse_container_name("hotel-exp-abc-rate-service-1")
        assert result["is_loadgen"] is False

        result2 = parse_container_name("mssim-exp-abc-frontend-2")
        assert result2["is_loadgen"] is False

    def test_high_replica_numbers(self):
        """Test parsing containers with high replica numbers."""
        result = parse_container_name("local-child-service-99")

        assert result["replica_num"] == 99
        assert result["service_name"] == "child-service"

    def test_various_patterns(self):
        """Test parsing various container naming patterns."""
        patterns = [
            ("hotel-exp-abc123def456-service-1", "service", 1, False),
            ("mssim-exp-012345678901-frontend-5", "frontend", 5, False),
            ("local-child-service-10", "child-service", 10, False),
            ("socialnet-timeline-service-2", "timeline-service", 2, False),
            ("synthetic_client_bench", "client_bench", None, True),
        ]

        for (
            container_name,
            expected_service,
            expected_replica,
            expected_loadgen,
        ) in patterns:
            result = parse_container_name(container_name)
            assert result["service_name"] == expected_service
            assert result["replica_num"] == expected_replica
            assert result["is_loadgen"] == expected_loadgen


class TestGroupContainersByService:
    """Tests for group_containers_by_service function."""

    def test_basic_grouping(self):
        """Test basic container grouping."""
        containers = [
            "hotel-exp-abc123def456-rate-service-1",
            "hotel-exp-abc123def456-rate-service-2",
            "hotel-exp-abc123def456-search-service-1",
            "hotel-exp-abc123def456-search-service-2",
            "hotel-exp-abc123def456-frontend-1",
        ]

        groups = group_containers_by_service(containers)

        assert len(groups) == 3
        assert len(groups["rate-service"]) == 2
        assert len(groups["search-service"]) == 2
        assert len(groups["frontend"]) == 1

        assert "hotel-exp-abc123def456-rate-service-1" in groups["rate-service"]
        assert "hotel-exp-abc123def456-rate-service-2" in groups["rate-service"]

    def test_mixed_app_grouping(self):
        """Test grouping containers from different apps."""
        containers = [
            "hotel-exp-abc123def456-rate-service-1",
            "hotel-exp-abc123def456-rate-service-2",
            "mssim-exp-012345678901-frontend-1",
            "mssim-exp-012345678901-frontend-2",
            "local-child-service-1",
        ]

        groups = group_containers_by_service(containers)

        # Should group by service name regardless of app prefix
        assert "rate-service" in groups
        assert "frontend" in groups
        assert "child-service" in groups

        assert len(groups["rate-service"]) == 2
        assert len(groups["frontend"]) == 2
        assert len(groups["child-service"]) == 1

    def test_single_container_per_service(self):
        """Test grouping when each service has only one container."""
        containers = [
            "hotel-exp-abc123def456-frontend-1",
            "hotel-exp-abc123def456-mongodb-1",
            "hotel-exp-abc123def456-redis-1",
        ]

        groups = group_containers_by_service(containers)

        assert len(groups) == 3
        assert len(groups["frontend"]) == 1
        assert len(groups["mongodb"]) == 1
        assert len(groups["redis"]) == 1

    def test_empty_container_list(self):
        """Test grouping empty container list."""
        groups = group_containers_by_service([])
        assert groups == {}

    def test_loadgen_grouping(self):
        """Test that load generators are grouped correctly."""
        containers = [
            "hotel-exp-abc123def456-rate-service-1",
            "hotel-exp-abc123def456-rate-service-2",
            "hotel_client_bench",
        ]

        groups = group_containers_by_service(containers)

        assert "rate-service" in groups
        assert len(groups["rate-service"]) == 2

        # Load generator should be in its own group
        loadgen_found = False
        for service_name, container_list in groups.items():
            if "client_bench" in service_name or "loadgen" in service_name.lower():
                loadgen_found = True
                assert len(container_list) == 1

        assert loadgen_found

    def test_many_replicas(self):
        """Test grouping with many replicas of the same service."""
        containers = [f"local-child-service-{i}" for i in range(1, 21)]

        groups = group_containers_by_service(containers)

        assert len(groups) == 1
        assert "child-service" in groups
        assert len(groups["child-service"]) == 20

    def test_service_name_consistency(self):
        """Test that service names are consistent across different experiments."""
        # Same service from different experiments should group together
        # Using the same project name/digest so they group together
        containers = [
            "hotel-exp1-abc123def456-rate-service-1",
            "hotel-exp1-abc123def456-rate-service-2",
            "hotel-exp1-abc123def456-rate-service-3",  # Same experiment, more replicas
        ]

        groups = group_containers_by_service(containers)

        # All should be in the same group since service name is the same
        assert len(groups) == 1
        assert "rate-service" in groups
        assert len(groups["rate-service"]) == 3

    def test_complex_service_names(self):
        """Test grouping with complex multi-word service names."""
        containers = [
            "socialnet-user-timeline-service-1",
            "socialnet-user-timeline-service-2",
            "socialnet-user-timeline-service-3",
            "socialnet-home-timeline-service-1",
            "socialnet-home-timeline-service-2",
        ]

        groups = group_containers_by_service(containers)

        assert len(groups) == 2
        assert len(groups["user-timeline-service"]) == 3
        assert len(groups["home-timeline-service"]) == 2


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
