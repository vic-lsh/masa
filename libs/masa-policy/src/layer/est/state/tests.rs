use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use masa_core::{Context, LatencyEwma};
use tonic_core::{CowGrpcMethod, Response, Status};

use super::estimators::blend_toward_legacy;
use super::metadata::ComputeTracker;
use super::request::FanoutInvocationState;
use super::*;
use crate::layer::est::fanout::{ChildRecord, PathPrefix};
use crate::layer::est::latency_map::ParentToChildKey;
use crate::registry::MethodId;
use crate::MethodRegistry;

fn mid(service: &str, method: &str) -> MethodId {
    MethodRegistry::global()
        .get_or_register(CowGrpcMethod::new(service.to_string(), method.to_string()))
}

fn sid(service: &str) -> MethodId {
    MethodRegistry::global().get_or_register_service(service.to_string())
}

#[test]
fn compute_tracker_accumulates_across_polls() {
    let tracker = ComputeTracker::new();
    assert_eq!(tracker.compute_us(), 0);

    tracker.start();
    std::thread::sleep(std::time::Duration::from_millis(1));
    tracker.stop();

    assert!(tracker.compute_us() > 0);

    let first = tracker.compute_us();
    tracker.start();
    std::thread::sleep(std::time::Duration::from_millis(1));
    tracker.stop();

    assert!(tracker.compute_us() > first);
}

#[test]
fn compute_tracker_stop_without_start_is_noop() {
    let tracker = ComputeTracker::new();
    tracker.stop();
    assert_eq!(tracker.compute_us(), 0);
}

#[test]
fn fanout_deadline_blend_moves_lower_estimates_toward_legacy() {
    assert_eq!(blend_toward_legacy(1_000, 5_000, 0.0), 1_000);
    assert_eq!(blend_toward_legacy(1_000, 5_000, 0.5), 3_000);
    assert_eq!(blend_toward_legacy(1_000, 5_000, 1.0), 5_000);
    assert_eq!(blend_toward_legacy(6_000, 5_000, 0.5), 5_000);
}

#[test]
fn fanout_state_advances_observable_prefix_before_open_group() {
    let a = mid("StateFanout", "a");
    let b = mid("StateFanout", "b");
    let c = mid("StateFanout", "c");
    let t0 = Instant::now();
    let mut state = FanoutInvocationState::default();
    state.children.push(ChildRecord {
        child_id: a,
        child_service_id: sid("StateFanout"),
        start: t0,
        end: Some(t0 + Duration::from_millis(10)),
        terminal: true,
    });
    state.children.push(ChildRecord {
        child_id: b,
        child_service_id: sid("StateFanout"),
        start: t0 + Duration::from_millis(1),
        end: Some(t0 + Duration::from_millis(20)),
        terminal: true,
    });
    state.children.push(ChildRecord {
        child_id: c,
        child_service_id: sid("StateFanout"),
        start: t0 + Duration::from_millis(30),
        end: None,
        terminal: false,
    });

    state.refresh_path_prefix_before_open_group(t0 + Duration::from_millis(30));

    let mut sig = vec![a, b];
    sig.sort();
    assert_eq!(state.path_prefix, PathPrefix::root().append_signature(&sig));
    assert_eq!(state.incorporated_child_count, 2);
}

#[test]
fn estimation_tracker_issues_open_group_with_completed_prefix() {
    let root = mid("StateTracker", "root");
    let parent = mid("StateTracker", "parent");
    let child_a = CowGrpcMethod::new("StateTracker", "child_a");
    let child_b = CowGrpcMethod::new("StateTracker", "child_b");
    let child_c = CowGrpcMethod::new("StateTracker", "child_c");
    let child_a_id = MethodRegistry::global().get_or_register(child_a.clone());
    let child_b_id = MethodRegistry::global().get_or_register(child_b.clone());
    let child_service_id = MethodRegistry::global().get_or_register_service("StateTracker");
    let est = LatencyEstimators::<LatencyEwma>::new();
    let tracker = EstimationTracker::new(parent, Some(root), est);

    let first = tracker.begin_child(&child_a);
    let response: Result<Response<()>, Status> = Ok(Response::new(()));
    tracker.record_child_complete(&first, &response);

    let second = tracker.begin_child(&child_b);
    assert_eq!(second.path_prefix, PathPrefix::root());

    let third = tracker.begin_child(&child_c);
    assert_eq!(
        third.path_prefix,
        PathPrefix::root().append_signature(&[child_a_id])
    );
    assert_eq!(third.base_signature, {
        let mut sig = vec![child_b_id, third.child_id];
        sig.sort();
        sig
    });
    assert_eq!(third.base_service_signature, {
        let mut sig = vec![child_service_id, child_service_id];
        sig.sort();
        sig
    });
}

#[test]
fn estimation_tracker_records_child_completion_and_flushes_parent_observations() {
    let root = mid("StateE2e", "root");
    let parent = mid("StateE2e", "parent");
    let child = CowGrpcMethod::new("StateE2e", "child");
    let child_id = MethodRegistry::global().get_or_register(child.clone());
    let key = ParentToChildKey::root_rpc_method(root)
        .parent_rpc_method(parent)
        .child_rpc_method(child_id);
    let est = LatencyEstimators::<LatencyEwma>::new();
    let tracker = EstimationTracker::new(parent, Some(root), est.clone());

    let child_tracker = tracker.begin_child(&child);
    std::thread::sleep(Duration::from_millis(1));
    let response: Result<Response<()>, Status> = Ok(Response::new(()));
    tracker.record_child_complete(&child_tracker, &response);

    let child_wallclock = est
        .est_child_wallclock(key)
        .expect("child wallclock tracked");
    assert!(child_wallclock > 0);

    std::thread::sleep(Duration::from_millis(1));
    tracker.flush();

    let after_child = est.est_after_child_wallclock(key, u64::MAX);
    assert!(after_child.full > 0);
    assert!(after_child.mean > 0);
    assert!(after_child.floor > 0);
}

#[test]
fn fanout_lookup_clamps_to_legacy_when_fanout_is_higher() {
    let root = mid("StateLookupClamp", "root");
    let parent = mid("StateLookupClamp", "parent");
    let child = mid("StateLookupClamp", "child");
    let sibling = mid("StateLookupClamp", "sibling");
    let service = sid("StateLookupClamp");
    let est = LatencyEstimators::<LatencyEwma>::new();
    let mut signature = vec![child, sibling];
    signature.sort();
    let mut service_signature = vec![service, service];
    service_signature.sort();
    let key = ParentToChildKey::root_rpc_method(root)
        .parent_rpc_method(parent)
        .child_rpc_method(child);

    est.track_after_child_wallclock(key, 1_000);
    for _ in 0..3 {
        est.track_fanout_group(
            root,
            parent,
            PathPrefix::root(),
            signature.clone(),
            PathPrefix::root(),
            service_signature.clone(),
            5_000,
        );
    }

    let estimate = est.est_after_child_wallclock_for_group(
        root,
        parent,
        PathPrefix::root(),
        &signature,
        PathPrefix::root(),
        &service_signature,
        child,
        u64::MAX,
    );

    assert_eq!(estimate.full, 1_000);
    assert_eq!(estimate.mean, 1_000);
    assert_eq!(estimate.floor, 1_000);
}

#[test]
fn fanout_lookup_uses_fanout_when_lower_than_legacy() {
    let root = mid("StateLookupMin", "root");
    let parent = mid("StateLookupMin", "parent");
    let child = mid("StateLookupMin", "child");
    let sibling = mid("StateLookupMin", "sibling");
    let service = sid("StateLookupMin");
    let est = LatencyEstimators::<LatencyEwma>::new();
    let mut signature = vec![child, sibling];
    signature.sort();
    let mut service_signature = vec![service, service];
    service_signature.sort();
    let key = ParentToChildKey::root_rpc_method(root)
        .parent_rpc_method(parent)
        .child_rpc_method(child);

    est.track_after_child_wallclock(key, 5_000);
    for _ in 0..3 {
        est.track_fanout_group(
            root,
            parent,
            PathPrefix::root(),
            signature.clone(),
            PathPrefix::root(),
            service_signature.clone(),
            1_000,
        );
    }

    let estimate = est.est_after_child_wallclock_for_group(
        root,
        parent,
        PathPrefix::root(),
        &signature,
        PathPrefix::root(),
        &service_signature,
        child,
        u64::MAX,
    );

    assert_eq!(estimate.full, 5_000);
    assert_eq!(estimate.mean, 1_000);
    assert_eq!(estimate.floor, 1_000);
}

#[test]
fn fanout_lookup_falls_back_to_service_shape_when_exact_is_sparse() {
    let root = mid("StateLookupCoarseRoot", "root");
    let parent = mid("StateLookupCoarseParent", "parent");
    let a1 = mid("StateLookupCoarseA", "a1");
    let a2 = mid("StateLookupCoarseA", "a2");
    let a3 = mid("StateLookupCoarseA", "a3");
    let b1 = mid("StateLookupCoarseB", "b1");
    let b2 = mid("StateLookupCoarseB", "b2");
    let b3 = mid("StateLookupCoarseB", "b3");
    let service_a = sid("StateLookupCoarseA");
    let service_b = sid("StateLookupCoarseB");
    let est = LatencyEstimators::<LatencyEwma>::new();
    let mut service_signature = vec![service_a, service_b];
    service_signature.sort();
    let key = ParentToChildKey::root_rpc_method(root)
        .parent_rpc_method(parent)
        .child_rpc_method(a1);

    est.track_after_child_wallclock(key, 5_000);
    for (left, right) in [(a1, b1), (a2, b2), (a3, b3)] {
        let mut exact_signature = vec![left, right];
        exact_signature.sort();
        est.track_fanout_group(
            root,
            parent,
            PathPrefix::root(),
            exact_signature,
            PathPrefix::root(),
            service_signature.clone(),
            1_000,
        );
    }

    let mut cold_exact = vec![a1, b1];
    cold_exact.sort();
    let estimate = est.est_after_child_wallclock_for_group(
        root,
        parent,
        PathPrefix::root(),
        &cold_exact,
        PathPrefix::root(),
        &service_signature,
        a1,
        u64::MAX,
    );

    assert_eq!(estimate.full, 5_000);
    assert_eq!(estimate.mean, 1_000);
    assert_eq!(estimate.floor, 1_000);
}

#[test]
fn fanout_lookup_ignores_single_child_groups() {
    let root = mid("StateLookupSingle", "root");
    let parent = mid("StateLookupSingle", "parent");
    let child = mid("StateLookupSingle", "child");
    let service = sid("StateLookupSingle");
    let est = LatencyEstimators::<LatencyEwma>::new();
    let key = ParentToChildKey::root_rpc_method(root)
        .parent_rpc_method(parent)
        .child_rpc_method(child);

    est.track_after_child_wallclock(key, 5_000);
    est.track_fanout_group(
        root,
        parent,
        PathPrefix::root(),
        vec![child],
        PathPrefix::root(),
        vec![service],
        1_000,
    );

    let estimate = est.est_after_child_wallclock_for_group(
        root,
        parent,
        PathPrefix::root(),
        &[child],
        PathPrefix::root(),
        &[service],
        child,
        u64::MAX,
    );

    assert_eq!(estimate.full, 5_000);
    assert_eq!(estimate.mean, 5_000);
    assert_eq!(estimate.floor, 5_000);
}

#[test]
fn deadline_signal_propagates_to_response_meta() {
    let tracker = RequestMetadataTracker::new();
    tracker.mark_deadline_signal();

    let mut ctx = Context::default();
    tracker.inject_response_meta(&mut ctx);

    let meta = ctx.response_meta().expect("response_meta set");
    assert_eq!(meta.deadline_signal_count, 1);
    assert_eq!(meta.early_return_count, 0);
}

#[test]
fn deadline_signal_default_is_zero_when_not_marked() {
    let tracker = RequestMetadataTracker::new();
    let mut ctx = Context::default();
    tracker.inject_response_meta(&mut ctx);

    let meta = ctx.response_meta().expect("response_meta set");
    assert_eq!(meta.deadline_signal_count, 0);
}

#[test]
fn mark_deadline_signal_is_idempotent() {
    let tracker = RequestMetadataTracker::new();
    tracker.mark_deadline_signal();
    tracker.mark_deadline_signal();
    tracker.mark_deadline_signal();

    let mut ctx = Context::default();
    tracker.inject_response_meta(&mut ctx);

    assert_eq!(ctx.response_meta().unwrap().deadline_signal_count, 1);
}

#[test]
fn deadline_signal_saturates_across_children() {
    // A request whose local deadline trips AND whose children also signal
    // should still count as one event (not 1 + N children).
    let tracker = RequestMetadataTracker::new();
    tracker.mark_deadline_signal();
    // Simulate absorbing several children that each reported a signal.
    for _ in 0..5 {
        tracker.child_deadline_signal.store(true, Ordering::Relaxed);
    }

    let mut ctx = Context::default();
    tracker.inject_response_meta(&mut ctx);

    assert_eq!(ctx.response_meta().unwrap().deadline_signal_count, 1);
}

#[test]
fn deadline_signal_propagates_from_child_only() {
    // Even if the local hop did not signal, a single descendant signal
    // should still surface as 1 at this hop's response_meta.
    let tracker = RequestMetadataTracker::new();
    tracker.child_deadline_signal.store(true, Ordering::Relaxed);

    let mut ctx = Context::default();
    tracker.inject_response_meta(&mut ctx);

    assert_eq!(ctx.response_meta().unwrap().deadline_signal_count, 1);
}
