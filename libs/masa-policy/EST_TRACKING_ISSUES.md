# Est Tracking Issues

Issues found during review of `libs/masa-policy/src/layer/est/`.

---

## Done

### 1. Remove unused `est_compute_latency` map

**Fixed in:** 9a9a8950

The `est_compute_latency` field on `EstServerState` was never read for any scheduling or admission decision — only written to and logged by `spawn_method_stats_printer`. Additionally, it had a mixed-key-scheme bug: local tracking (`track_latencies`, state.rs:150) used single method IDs, while child-reported tracking (`after_child_rpc`, state.rs:202) used compound `(parent << 32 | child)` keys. The stats printer formatted all keys as single method IDs, so compound-keyed entries printed as garbage.

The underlying compute data still flows correctly through `ResponseMeta` → `accumulated_child_compute_us` → `est_accumulated_cost` for admission control; that path never touched `est_compute_latency`.

Removed the field, its construction, stats printer spawn, and both tracking call sites.

### 2. Remove dead `admission_check` on `EstRequestState`

Deleted the dead `EstRequestState::admission_check` method (state.rs) and its only caller, the `test_admission_check_floor_based_admits_with_budget` test. Rewrote the test to exercise `PredictiveAdmission::admission_check` directly — the production Layer 1 floor-based feasibility path. Also removed the now-unused `ParentContext::ctx()` test helper.

### 3. Type-safe `LatencyMap` keys

**Fixed in:** 579696e0, 227c913b

Introduced newtype key wrappers (`MethodKey`, `ParentToChildKey`, `RootToLocalKey`) and made `LatencyMap` generic over the key type, so the compiler rejects mismatched keys. Added `MethodId` newtype (issued by `MethodRegistry`) to replace raw `u64` method IDs. Multi-method keys use a builder pattern (`ParentToChildKey::parent_rpc_method(id).child_rpc_method(id)`) that eliminates inline bit-shift constructions. Also introduced `MethodRegistry` with `CowGrpcMethod` for type-safe method registration and lookup.

### 6. `print_counter` ownership split

Moved `log_estimates` from `EstRequestState` to `EstServerState`, co-locating the logging logic with the `print_counter` field and `est_child_latency` map it accesses. The call site in `prepare_before_child_rpc` now calls `self.server.log_estimates(...)`.

---

## TODO

### 4. `EstRequestState` has too many responsibilities

**File:** `state.rs:67-325`

`EstRequestState` is a ~260-line struct handling six distinct concerns:

1. **Compute time tracking** — `start_compute_tracking()`, `stop_compute_tracking()`, `poll_compute_us`, `poll_start`
2. **Child RPC preparation** — `prepare_before_child_rpc()` (method resolution, estimate lookups, child state setup)
3. **Child response processing** — `after_child_rpc()` (extract `ResponseMeta`, utilization tracking, 0-injection, end-time recording)
4. **Response meta injection** — `inject_response_meta()`
5. **Latency tracking on finalization** — `track_latencies()`
6. **Periodic logging** — `log_estimates()` (using `print_counter` from `EstServerState`)

The `prepare_before_child_rpc` method is the most overloaded: it resolves method IDs via `MethodRegistry`, constructs a `ParentToChildId`, looks up three different estimates (`est_after_child_latency` with mean, floor, and full), sets up the child state, logs, and returns a result struct. If any of these concerns need to change independently, the whole struct is in the blast radius.

**Proposed fix:** Extract compute tracking into a small `ComputeTracker` struct (owns `poll_compute_us` and `poll_start`). Consider whether child RPC handling (`prepare_before_child_rpc` + `after_child_rpc`) could be a standalone helper that takes `&EstServerState` and `&mut EstChildState` without needing the full `EstRequestState`. This would make each piece independently testable.

---

### 5. `EstChildState` two-phase initialization

**File:** `state.rs:336-381`

`EstChildState` is constructed fully empty, then populated later via `setup()`:

```rust
// Construction — all None
pub(crate) fn new() -> Self {
    Self {
        start_time: None,
        parent_to_child_id: None,
        child_method: None,
        server: None,
    }
}

// Populated later
pub(crate) fn setup(&mut self, parent_to_child_id, child_method, server) {
    self.start_time = Some(Instant::now());
    self.parent_to_child_id = Some(parent_to_child_id);
    self.child_method = Some(child_method);
    self.server = Some(server);
}
```

Then `finalize()` pattern-matches on `(self.start_time, &self.parent_to_child_id, &self.server)` to check if all three are `Some`. If `setup()` was never called, `finalize()` silently does nothing — no error, no log, just a dropped observation. This means a bug in the call sequence (forgetting to call `setup`) would silently degrade estimate quality without any signal.

This two-phase pattern exists because `ChildContext` (hooks.rs:223) is created by `ClientHooks::new()` before the parent knows which child is being called — `setup()` happens later in `before_child_rpc`.

**Proposed fix:** Split into two types: an opaque `UninitEstChild` (or just `()`) returned by `ClientHooks::new()`, and a fully-initialized `EstChildState` produced by `setup()`. This requires the `Layer` trait's `Child` associated type to accommodate the transition, which may be more refactoring than it's worth. A lighter alternative: add a `debug_assert!` in `finalize()` that `start_time.is_some()`, so missed `setup()` calls are caught in testing.

---

