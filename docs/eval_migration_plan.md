# Deployment Scheme Redesign for Masa Benchmarking

## Problem Summary

The current `exp_runner` deployment system is complex and brittle due to:

1. **Plugin Duplication** - 4 app plugins (~2,700 LOC) with significant code overlap
2. **Multi-Level Configuration** - gen_config.json → app config → docker-compose → env vars with complex transformations
3. **Inconsistent Patterns** - Each app handles topology, naming, K8s, and load generation differently
4. **Brittle Namespace Isolation** - SHA256 hashing for project names implemented in 3+ places
5. **Partial K8s Support** - Only Synthetic works with K8s
6. **Scattered Responsibilities** - AppPlugin has 12+ abstract methods mixing config, build, and deployment concerns

## Design Principle: Separate Topology from Experiment Config

**Key insight:** Topology and experiment parameters have different change frequencies:

| App Category | Topology Source | Change Frequency |
|-------------|-----------------|------------------|
| **Real apps** (hotel, socialnet) | Fixed by app code | Never - implicitly defined |
| **Simulation** (mssim) | Generated from Alibaba traces | Rarely - occasional tweaks |
| **Synthetic** | Experiment parameter | Often - fanout, latency models vary per experiment |

This means:
- **Experiment config** (RPS, SLO, duration, policies) → changes per experiment
- **Topology definition** (services, replicas, dependencies) → changes rarely or never

## Proposed Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                    TOPOLOGY LAYER (changes rarely)              │
├─────────────────────────────────────────────────────────────────┤
│ Real Apps:     apps/hotel/topology.yaml (or implicit in code)   │
│ Mssim:         exp/mssim/topologies/<trace-name>.yaml           │
│ Synthetic:     exp/synthetic/topologies/<name>.yaml             │
└─────────────────────────────────────────────────────────────────┘
                              │
                              │ reference
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│               EXPERIMENT CONFIG (changes often)                 │
│              exp/<app>/in/<exp>/experiment.yaml                 │
├─────────────────────────────────────────────────────────────────┤
│ - policies, repeats, warmup, duration                           │
│ - RPS sweep, SLO targets, timeouts                              │
│ - topology_ref: "default" | "custom-fanout-3"                   │
│ - replica_overrides: {rate: 4, profile: 2}                      │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                    DEPLOYMENT GENERATOR                         │
├─────────────────────────────────────────────────────────────────┤
│ Merges: topology + experiment config + runtime (project name)   │
│ Outputs: docker-compose.yaml or Helm values.yaml                │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                    DEPLOYMENT MANAGER                           │
│                 (existing Docker/K8s abstraction)               │
└─────────────────────────────────────────────────────────────────┘
```

## Key Changes

### 1. Topology Definition (Separate from Experiments)

**For Real Apps (hotel, socialnet):** Topology is implicit in app code. Define a single canonical topology file that documents what the app expects:

```yaml
# apps/hotel/topology.yaml (reference/documentation, rarely edited)
kind: Topology
metadata:
  app: hotel
  description: "Hotel reservation microservices"

services:
  frontend:
    port: 8660
    default_replicas: 1
  rate:
    port: 8663
    default_replicas: 1
    depends_on: [rate-mongo, rate-redis]
  profile:
    port: 8662
    default_replicas: 1
    depends_on: [profile-mongo, profile-redis]
  # ... other services

infrastructure:
  rate-mongo:
    image: mongo:7.0
    port: 27003
  rate-redis:
    image: redis:7.2
    port: 11003
  # ... other infra
```

**For Mssim:** Topology generated from trace analysis, stored separately:

```yaml
# exp/mssim/topologies/S_14677443.yaml (generated from trace, occasionally tweaked)
kind: Topology
metadata:
  app: mssim
  trace: "alibaba-2022/S_14677443"

services:
  - id: MS_56394
    default_replicas: 1
    methods:
      - name: method_a
        latency_us: 5000
  # ... generated from trace
```

**For Synthetic:** Topologies are experiment parameters, stored in shared location:

```yaml
# exp/synthetic/topologies/fanout-3.yaml
kind: Topology
metadata:
  app: synthetic
  description: "3-level fanout topology for latency testing"

call_graph:
  entry_points:
    api_a: [{MS_root::handle: 1.0}]

  services:
    - id: MS_root
      default_replicas: 1
      methods:
        - name: handle
          latency_distribution: {Exponential: {mean: 1000}}
          call_sequence:
            - {MS_child1::process: 1.0}
            - {MS_child2::process: 1.0}
            - {MS_child3::process: 1.0}
    # ... child services
```

### 2. Experiment Configuration (Changes Often)

Lightweight config that references topology and specifies experiment parameters:

```yaml
# exp/hotel/in/baseline/experiment.yaml
kind: Experiment
metadata:
  name: baseline
  app: hotel

spec:
  # Reference topology (optional - uses app default if omitted)
  topology_ref: default

  # Override replicas for this experiment (optional)
  replica_overrides:
    rate: 2
    profile: 2

  # Experiment execution
  execution:
    repeats: 3
    policies: [fifo, prio_global]
    warmup_secs: 10
    duration_secs: 60

  # Load generation
  loadgen:
    rps: [100, 200, 300, 400, 500]
    default_timeout_ms: 1000  # Default timeout, can be overridden per-API

    # Per-API config: SLO is required, timeout and weight are optional
    apis:
      - name: Search
        slo_us: 200000      # Required: SLO for this API
        weight: 0.6         # Optional: request weight (defaults to equal distribution)
        # timeout_ms: 1500  # Optional: override default timeout

      - name: Reservation
        slo_us: 300000      # Different SLO for reservations (more complex operation)
        weight: 0.4
        timeout_ms: 2000    # Longer timeout for reservations
```

```yaml
# exp/synthetic/in/fanout-test/experiment.yaml
kind: Experiment
metadata:
  name: fanout-test
  app: synthetic

spec:
  # Reference a specific topology (synthetic experiments often vary this)
  topology_ref: fanout-3

  execution:
    repeats: 1
    policies: [fifo, prio_global, "prio_local,early"]
    warmup_secs: 5
    duration_secs: 30

  loadgen:
    rps: [50, 100, 150]
    default_timeout_ms: 500

    apis:
      - name: api_a
        slo_us: 100000      # Required: must explicitly specify SLO
        # weight defaults to 1.0 when single API
```

### 3. Topology Resolution

```python
# exp_runner/runner/topology.py
class TopologyResolver:
    """Resolves topology references to concrete topology specs."""

    def resolve(
        self,
        app_name: str,
        topology_ref: str,  # "default", "fanout-3", "S_14677443"
        repo_root: Path,
    ) -> TopologySpec:
        """
        Resolution order:
        1. For "default": Look in apps/{app}/topology.yaml
        2. For named ref: Look in exp/{app}/topologies/{ref}.yaml
        3. For mssim traces: Look in trace-analysis/golden/{ref}/
        """
        ...

    def apply_overrides(
        self,
        topology: TopologySpec,
        replica_overrides: dict[str, int],
    ) -> TopologySpec:
        """Apply per-experiment replica overrides."""
        ...
```

### 4. Deployment Generator Interface

Generates platform-specific manifests by merging topology + experiment config:

```python
# exp_runner/runner/generators/base.py
class DeploymentGenerator(ABC):
    @abstractmethod
    def generate(
        self,
        topology: TopologySpec,
        experiment: ExperimentConfig,
        output_dir: Path,
        project_name: str,
        policy: str,
        image_tag: str,
    ) -> GeneratedDeployment:
        """Generate deployment files (compose yaml or helm values)."""
        pass

@dataclass
class GeneratedDeployment:
    deploy_root: Path
    deploy_file: str
    env_vars: dict[str, str]
    cleanup_files: list[Path]
```

Implementations:
- `ComposeGenerator` - Generates docker-compose.yaml from topology + experiment
- `HelmValuesGenerator` - Generates Helm values.yaml from topology + experiment

### 5. Simplified Plugin Interface

Reduce AppPlugin from 12+ abstract methods to ~6:

```python
class AppPlugin(ABC):
    @abstractmethod
    def get_app_name(self) -> str: ...

    @abstractmethod
    def get_binaries(self) -> list[str]: ...

    @abstractmethod
    def get_frontend_name(self) -> str: ...

    def get_default_topology_path(self, repo_root: Path) -> Optional[Path]:
        """Return path to default topology file, or None if implicit."""
        return repo_root / "apps" / self.get_app_name() / "topology.yaml"

    def customize_topology(self, topology: TopologySpec) -> TopologySpec:
        """Hook for app-specific topology transformations."""
        return topology

    def customize_env_vars(self, topology, experiment, base_env) -> dict:
        """Hook for app-specific env vars."""
        return base_env

    def validate_experiment(self, topology, experiment) -> None:
        """Validate experiment against topology."""
        pass
```

### 6. Centralized Namespace Isolation

Single function replaces 3+ duplicate implementations:

```python
# exp_runner/runner/naming.py
def generate_project_name(
    app: str,
    experiment_name: str,
    iteration: int,
    policy: str,
    rps: Optional[float] = None,
) -> str:
    """Format: {app_prefix}-{slug}-{digest}"""
    ...
```

### 7. Unified Build Orchestrator

The build system has two parts:

**Part A: AppPlugin provides app-specific build metadata**

```python
class AppPlugin(ABC):
    @abstractmethod
    def get_binaries(self) -> list[str]:
        """Return list of binary names to build.

        Example for hotel: ["hotel_frontend", "hotel_rate", "hotel_profile", ...]
        Example for synthetic: ["synthetic_frontend", "synthetic_child", "synthetic_client_bench"]
        """
        pass

    def get_cargo_package(self) -> str:
        """Return the cargo package name (defaults to app name)."""
        return self.get_app_name()

    def get_build_parallelism(self) -> int:
        """Max parallel runtime image builds (hotel=10, synthetic=3)."""
        return 10
```

**Part B: BuildOrchestrator uses shared Dockerfile pattern**

All apps use the same 3-stage Dockerfile (already in `exp_runner/common/docker-build/Dockerfile`):

```python
class BuildOrchestrator:
    """
    Builds Docker images for any app using the shared multi-stage Dockerfile.

    The Dockerfile expects these build args:
    - APP: cargo package name (e.g., "hotel")
    - FEATURES: cargo features (e.g., "prio_global,early")
    - BINARY_NAME: which binary to copy into runtime image
    - LOG_LEVEL: rust log level
    - GEN_CONFIG_PATH: path to gen_config.json
    """

    def build(
        self,
        app: AppPlugin,
        features: Optional[str],
        gen_config_path: Path,
        no_cache: bool = False,
    ) -> None:
        binaries = app.get_binaries()
        package = app.get_cargo_package()
        parallelism = app.get_build_parallelism()

        # Stage 1: Build all binaries (shared, cached)
        # `docker buildx build --target builder --build-arg APP={package} --build-arg FEATURES={features}`

        # Stage 2: Runtime base (shared, cached)
        # `docker buildx build --target runtime-base --build-arg GEN_CONFIG_PATH={gen_config_path}`

        # Stage 3: Per-binary runtime images (parallel)
        # For each binary in binaries:
        #   `docker buildx build --target runtime --build-arg BINARY_NAME={binary} --tag {binary}:{tag}`
        # Uses ThreadPoolExecutor(max_workers=parallelism)
```

**Why this works:**

The shared Dockerfile already handles all apps because:
1. Stage 1 (`builder`) runs `cargo build --release -p {APP} --features {FEATURES}` - builds all binaries for the package
2. Stage 2 (`runtime-base`) copies shared dependencies (libssl, gen_config.json)
3. Stage 3 (`runtime`) copies a single binary specified by `BINARY_NAME`

The only app-specific knowledge needed is:
- Which binaries exist (from `get_binaries()`)
- The cargo package name (from `get_cargo_package()`)
- Parallelism preference (from `get_build_parallelism()`)

## Migration Strategy

### Phase 1: Core Data Models (Non-breaking)
- Implement `TopologySpec` and `ExperimentConfig` dataclasses
- Implement `TopologyResolver` with resolution logic
- Add `from_legacy()` converters for existing configs
- No changes to existing plugins yet

### Phase 2: Extract Canonical Topologies
- Create `apps/hotel/topology.yaml` from existing docker-compose + hotel.json patterns
- Create `apps/socialnet/topology.yaml` from docker-compose
- Migrate existing mssim traces to `exp/mssim/topologies/`
- Create sample synthetic topologies in `exp/synthetic/topologies/`

### Phase 3: Deployment Generators (Parallel Path)
- Implement `ComposeGenerator` and `HelmValuesGenerator`
- Add opt-in flag `--use-new-generator`
- Test against existing compose files for equivalence

### Phase 4: Plugin Simplification (Per-app)
- Start with Synthetic (most flexible, already has call_graph)
- Migrate Hotel, Socialnet, Mssim
- Move common code to BuildOrchestrator and generators

### Phase 5: K8s Support (Incremental)
- Create Helm charts for hotel, socialnet, mssim
- Enable K8s for all apps using same topology files

## Files to Modify/Create

**New Files:**
- `exp_runner/runner/topology.py` - TopologySpec, TopologyResolver
- `exp_runner/runner/experiment_config.py` - ExperimentConfig dataclass
- `exp_runner/runner/generators/base.py` - DeploymentGenerator interface
- `exp_runner/runner/generators/compose.py` - ComposeGenerator
- `exp_runner/runner/generators/helm.py` - HelmValuesGenerator
- `exp_runner/runner/naming.py` - Centralized project naming
- `exp_runner/runner/build_orchestrator.py` - Unified build logic
- `exp_runner/runner/legacy.py` - Legacy config converters
- `apps/hotel/topology.yaml` - Canonical hotel topology
- `apps/socialnet/topology.yaml` - Canonical socialnet topology
- `exp/synthetic/topologies/` - Directory for synthetic topologies

**Modify:**
- `exp_runner/runner/apps/base.py` - Simplify AppPlugin interface
- `exp_runner/runner/apps/synthetic.py` - First plugin migration target
- `exp_runner/runner/experiment_driver.py` - Integrate with generators
- `exp_runner/runner/cli.py` - Add `--use-new-generator` flag

## Expected Benefits

1. **Clear separation of concerns** - Topology (what) vs Experiment (how) are independent
2. **Reusable topologies** - Multiple experiments can reference same topology
3. **~60-70% reduction in plugin code** - Common logic extracted to shared components
4. **Consistent K8s support** - All apps work on both Docker and K8s from same topology
5. **Easier experimentation** - Change RPS/SLO without touching topology definitions
6. **Better for synthetic** - Topology variations become first-class experiment parameters
7. **Incremental adoption** - Existing experiments continue working during migration

## Acceptance Criteria

All 4 app experiment test scripts must pass:

```bash
./scripts/test_e2e_hotel.sh          # Hotel end-to-end experiment
./scripts/test_socialnet.sh          # Socialnet experiment
./scripts/test_synthetic_experiment.sh  # Synthetic experiment
./scripts/test_mssim_experiment.sh   # MSSIM experiment
```

## Verification Steps

1. **Unit tests:** `uv run pytest` after each phase
2. **Dry-run comparison:** Run existing experiments with `--dry-run` to compare generated vs existing compose files
3. **E2E tests:** All 4 test scripts above must pass
4. **Equivalence check:** Compare experiment results (goodput, latency) before/after migration to ensure no regression
