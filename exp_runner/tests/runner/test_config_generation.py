import json
from pathlib import Path
import pytest
from exp_runner.runner.apps.hotel import (
    create_gen_config_dict as hotel_create_gen,
    create_hotel_config_dict,
)
from exp_runner.runner.apps.synthetic import SyntheticApp


class TestHotelConfigGeneration:
    def test_create_gen_config_dict(self):
        template = {"Addr": "http://localhost:8080"}
        result = hotel_create_gen(template_config=template)
        assert result["Addr"] == "http://hotel_frontend:8080"

        # Test with port extraction
        template = {"Addr": "http://1.2.3.4:9000"}
        result = hotel_create_gen(template_config=template)
        assert result["Addr"] == "http://hotel_frontend:9000"

        # Test defaults
        template = {"Addr": "http://host"}
        result = hotel_create_gen(template_config=template)
        assert result["Addr"] == "http://hotel_frontend:8660"

    def test_create_hotel_config_dict(self):
        template = {
            "rate": {
                "ip": "local-rate-service",
                "mongodbAddr": "mongodb://rate-mongo:27017",
            },
            "frontend": {"ip": "local-frontend-service"},
        }
        project_name = "test-proj"
        result = create_hotel_config_dict(
            template_config=template, project_name=project_name
        )

        # Check scaled service
        assert result["rate"]["ip"] == "test-proj-rate-service"
        # Check infra
        assert result["rate"]["mongodbAddr"] == "mongodb://test-proj-rate-mongo-1:27017"
        # Check frontend (unscaled)
        assert result["frontend"]["ip"] == "test-proj-hotel-frontend-service-1"


class TestSyntheticConfigGeneration:
    def test_create_gen_config_dict(self, tmp_path):
        app = SyntheticApp()
        template = {"Addr": "http://localhost:8000"}
        template_path = tmp_path / "gen_config.json"
        with open(template_path, "w") as f:
            json.dump(template, f)

        result = app.create_gen_config_dict(
            template_config_path=template_path,
            project_name="test-syn",
            service_name_override="custom-svc",
        )
        assert result["Addr"] == "http://custom-svc:8000"

    def test_create_call_graph_compose_dict(self):
        app = SyntheticApp()
        app_config = {
            "call_graph": {
                "services": [{"id": "A", "replicas": 2}, {"id": "B", "replicas": 1}]
            }
        }
        image_tag = "test-tag"

        result = app.create_call_graph_compose_dict(app_config, image_tag)

        services = result["services"]
        assert "synthetic-frontend-service" in services
        assert "local-a-service" in services
        assert "local-b-service" in services

        assert services["local-a-service"]["scale"] == 2
        assert services["local-b-service"]["scale"] == 1
        assert services["local-a-service"]["image"] == "synthetic_child:test-tag"

    def test_generate_k8s_values(self):
        app = SyntheticApp()
        project_name = "k8s-proj"
        image_tag = "k8s-tag"
        env_vars = {"LOG_LEVEL": "debug"}

        result = app.generate_k8s_values(
            project_name=project_name,
            image_tag=image_tag,
            app_config_path=None,
            env_vars=env_vars,
        )

        assert result["fullnameOverride"] == project_name
        assert result["logLevel"] == "debug"
        assert result["image"]["tag"] == image_tag
