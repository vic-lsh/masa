# Design Doc: Masa Unified Experiment Framework

This document outlines the refactoring of the Masa experiment orchestration system into a unified, platform-agnostic framework. This architecture decouples experiment logic from infrastructure, enabling seamless execution across Docker and Kubernetes for all applications.

## 1. High-Level Architecture

The framework transitions from application-specific experiment logic to a **Three-Tier Architecture**.

### 1.1 Components

#### **A. ExpDriver (The Orchestrator)**
*   **Role**: A single, platform-agnostic class that defines the universal sequence of an experiment.
*   **Responsibilities**: 
    *   Managing the experiment lifecycle (Initialization -> Build -> Deploy -> Execute -> Teardown).
    *   Coordinating between the `AppPlugin` and the `DeploymentManager`.
    *   Handling cross-cutting concerns like logging, timeouts, and result verification.
*   **Key Property**: It contains no application-specific or platform-specific branching logic.

#### **B. AppPlugin (The Knowledge Base)**
*   **Role**: Encapsulates the configuration and metadata of the application being tested.
*   **Responsibilities**:
    *   **Config Generation**: Transforms template configs into runtime artifacts (e.g., generating `hotel.json` or `synthetic.json`).
    *   **Environment Mapping**: Defines environment variables for both the application services and the load generator.
    *   **Deployment Mapping**: Returns platform-specific artifact paths (e.g., a path to a Helm Chart for K8s or a Docker Compose file for local runs).
    *   **Task Specification**: Provides the `LoadGenSpec` (image, command, environment) for the Load Generator.
*   **Key Property**: It defines the "What" (the images and configs) but delegates the "How" to the Manager.

#### **C. DeploymentManager (The Executor)**
*   **Role**: Acts as the interface to the underlying infrastructure (Docker or K8s).
*   **Responsibilities**:
    *   **Lifecycle Management**: Starting and stopping service groups (Helm releases or Compose projects).
    *   **Task Execution**: Executing one-off tasks (like Load Generators) and blocking until completion.
    *   **Observability**: Streaming logs from containers/pods and collecting resource metrics (CPU/Memory).
*   **Key Property**: It knows how to use `kubectl`, `helm`, or `docker compose` but knows nothing about Masa's gRPC policies.

---

## 2. The Experiment Sequence

The `ExpDriver` drives the process using a standardized loop:

1.  **Setup**: `ExpDriver` triggers the `AppPlugin` to generate unique configuration files for the specific iteration/policy into the experiment's output directory.
2.  **Build**: `ExpDriver` calls the `AppBuilder` (provided by the `AppPlugin`) to ensure images are built with the correct feature flags.
3.  **Deploy**: `ExpDriver` asks `AppPlugin` for its deployment artifact (Compose file or Helm path) based on the current platform. It passes this to the `DeploymentManager` to start the services.
4.  **Execute**: `ExpDriver` requests a `LoadGenSpec` from the `AppPlugin` and tells the `DeploymentManager` to "Run Task". This blocks until the load generator finishes.
5.  **Monitor**: While the task runs, the `DeploymentManager` streams logs and health status to the `ExpDriver`.
6.  **Teardown**: `ExpDriver` ensures the `DeploymentManager` stops and cleans up all services, even if the task fails.

---

## 3. Migration Plan

### Phase 1: Foundation (Core Refactor)
*   **Today**: `SyntheticWorkloadRunner` handles its own orchestration; `HotelApp.run_workload` is a massive function containing manual docker commands and threading.
*   **Migration**: Implement `ExpDriver` in `libs/exp_runner/runner/apps/base.py`. Move monitoring and log-streaming logic from app-specific files into the `DeploymentManager` base class and its implementations.

### Phase 2: Synthetic App (The Pilot)
*   **Today**: Uses `DockerSyntheticRunner` and `K8sSyntheticRunner` subclasses. It has a separate `K8sSyntheticLoadGenerator` class that manually handles Pod manifests.
*   **Migration**: 
    *   Merge runners into a single `SyntheticApp` plugin.
    *   Move the logic for creating K8s Pods for load generation into a generic `K8sManager.run_task` method.
    *   The `SyntheticApp` will simply return its Helm chart path when queried for K8s deployment info.

### Phase 3: Hotel App (The Port)
*   **Today**: The `run_workload` method in `hotel.py` is a monolithic Docker-only function. It explicitly throws `NotImplementedError` for K8s.
*   **Migration**:
    *   Refactor `run_workload` to instantiate and call the `ExpDriver`.
    *   Implement `get_deployment_info` to return the Docker Compose path.
    *   **To Enable K8s**: Create a `charts/hotel` directory and update the plugin to return that path when the platform is K8s. The existing `K8sManager` handles the deployment without any new Python code.

---

## 4. Key Advantages

1.  **Standardized Observability**: Using one `ExpDriver` ensures metrics and logs are collected identically across all apps, removing variability in experimental results.
2.  **Increased Developer Velocity**: Adding a new application no longer requires implementing orchestration, threading, or cleanup logic. Only the `AppPlugin` interface and a Helm chart are needed.
3.  **Infrastructure Portability**: If the system needs to support a new platform (e.g., AWS ECS), only one new `DeploymentManager` needs to be written. All apps will support the new platform immediately.