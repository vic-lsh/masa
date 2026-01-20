# AGENTS.md

> **Purpose**: This file provides context, commands, and guidelines for AI agents (and humans) working on the Masa repository.
> **Repo**: Masa is an RPC system improving goodput via dynamic prioritization, built on Tonic, Hyper, and Tokio.

## 1. Project Structure & Environment

### Directory Layout
- `libs/`: Core Rust libraries (modified `tonic`, `masa` support).
- `apps/`: Microservice applications (`hotel`, `socialnet`, `synthetic`) and simulators.
- `exp/`: Python-based experiment orchestration (`exp.runner`) and analysis tools.
- `scripts/`: Shell scripts for CI/CD workflows (test, check, format).

### Environment Setup
- **Rust**: Ensure `cargo` is installed.
- **Python**: Uses `uv` for dependency management.
    ```bash
    # Install uv (if needed)
    curl -LsSf https://astral.sh/uv/install.sh | sh
    
    # Sync dependencies
    uv sync
    
    # Activate environment
    source .venv/bin/activate
    ```

---

## 2. Workflows & Commands

### Rust Development
*Always run checks from the root directory.*

- **Format Code**:
  ```bash
  ./scripts/format.sh
  # Equivalent to: cargo fmt
  ```

- **Lint / Check**:
  ```bash
  ./scripts/check.sh
  # Checks standard and feature-flagged builds (fifo, prio_global, etc.)
  ```

- **Run All Tests**:
  ```bash
  ./scripts/test.sh
  # Runs specific shell scripts + cargo test for known-good packages
  ```

- **Run Single Package Test**:
  ```bash
  cargo test -p <package_name>
  # Example: cargo test -p masa
  ```

- **Run Specific Test Case**:
  ```bash
  cargo test -p <package_name> -- <test_function_name>
  ```

### Python Development (Experiments)
*Ensure venv is active or use `uv run`.*

- **Run Tests**:
  ```bash
  pytest
  # Configured in pytest.ini to check exp/tests
  ```

- **Experiment Runner**:
  The `exp.runner` module is the entry point for experiments.
  ```bash
  # Run an experiment
  python3 -m exp.runner run <app> <experiment_name> --plot
  
  # Plot existing results
  python3 -m exp.runner plot <app> <experiment_name>
  ```

---

## 3. Code Style & Conventions

### General Rules
- **Formatting**: Strictly follow `rustfmt` for Rust.
- **Cleanup**: Remove unused code immediately; do not comment it out.
- **Complexity**: Prefer simple designs. If a feature makes code complex, question if the feature is needed.
- **Documentation**: 
  - Update `README.md` in relevant directories if CLI interfaces or usage instructions change.
  - Document *why* complex logic exists, not *what* it does.

### Rust Conventions
- **Async/Await**: The codebase relies heavily on `tokio`. Use standard async patterns.
- **Error Handling**: Use `Result` and `Option` idiomatically. Avoid `unwrap()` unless in tests or strictly necessary with comments.
- **Feature Flags**: Masa uses feature flags for scheduling policies (e.g., `fifo`, `prio_global`).
    - When adding core logic, consider if it needs to be behind a feature flag.
    - Check `./scripts/check.sh` to see important flag combinations.
- **Libraries**:
    - **Tonic**: gRPC implementation. Changes often involve `tonic` internals in `libs/tonic`.
    - **Hyper/Tower**: HTTP and Service abstraction layers.

### Python Conventions
- **Type Hints**: Use type hints for function arguments and return values.
- **Dependencies**: Do not use `pip` directly. Use `uv add <package>` or edit `pyproject.toml` and run `uv sync`.
- **Structure**: Experiment logic goes in `exp/runner`, tests in `exp/tests`.

### Testing Guidelines
- **New Features**: Must include new tests.
- **Behavior Changes**: Update existing tests or add new ones.
- **Integration**: `apps/` contains integration testbeds (e.g., `hotel`).
    - Use `test_e2e_hotel.sh` or similar scripts in `scripts/` for end-to-end verification.

---

## 4. Key Configurations

- **`Cargo.toml`**: Workspace definition. `libs/tonic/*` are members.
- **`pyproject.toml`**: Python dependencies and tool config.
- **`scripts/test.sh`**: The source of truth for which packages are currently passing tests. check this file to see which packages to skip if tests are failing.

## 5. Agent Instructions
- **Analysis First**: Before editing, use `grep` and `glob` to understand call sites and dependencies.
- **Scoped Changes**: Do not change code outside the scope of the request unless necessary for correctness.
- **Verification**: 
    - For Rust: Run `./scripts/check.sh` after changes.
    - For Python: Run `pytest` if touching `exp/`.
- **Safety**: Do not commit secrets. Explain destructive `bash` commands before running.
