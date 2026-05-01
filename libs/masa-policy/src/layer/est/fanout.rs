// Fanout-aware after-child wall-clock estimation.
//
// Per-edge EMAs (`(parent, child)`) absorb sibling-position noise under
// parallel fanout: the fastest sibling sees a long "after-child time" that
// is really just waiting for the slowest sibling, even though that wait is
// not post-join work. The fix is to estimate post-join time per fanout
// *group* — the maximal set of children whose intervals overlap — and have
// all siblings in a group share one estimate.
//
// At handler exit we recover groups via connected-components-by-overlap on
// the observed start/end intervals (always correct, since intervals are
// observable). We also derive an observable path prefix from earlier completed
// groups so repeated sequential fanouts with the same signature do not collapse
// into one estimate. At child-issue time we predict which group the child will
// join from a lower bound (the still-open earlier siblings + this child) and
// estimate from the closest compatible signature under the same prefix,
// falling back to a signature-only aggregate when the prefix-specific pattern
// is cold.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Mutex;
use std::time::Instant;

use masa_core::LatencyEstimator;

use crate::registry::MethodId;

use super::state::AfterChildEstimates;

/// One fanout child observed within a handler invocation.
#[derive(Debug, Clone)]
pub(crate) struct ChildRecord {
    pub child_id: MethodId,
    pub start: Instant,
    pub end: Option<Instant>,
    /// True once the RPC has returned, including early returns that leave
    /// `end=None` and are ignored by group recovery.
    pub terminal: bool,
}

/// Prefix of fanout groups that have completed before the current group.
///
/// This is intentionally derived only from observable RPC timing: each
/// completed group appends its sorted child signature to a rolling hash. It
/// separates repeated sequential fanouts without application-provided branch
/// tags. If two code paths are observationally identical up to the current
/// child issue point, they necessarily share a prefix and are estimated as an
/// expected value over those hidden paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct PathPrefix {
    hash: u64,
    depth: u16,
}

impl Default for PathPrefix {
    fn default() -> Self {
        Self::root()
    }
}

impl PathPrefix {
    const ROOT_HASH: u64 = 0xcbf2_9ce4_8422_2325;

    pub(crate) fn root() -> Self {
        Self {
            hash: Self::ROOT_HASH,
            depth: 0,
        }
    }

    pub(crate) fn append_signature(self, signature: &[MethodId]) -> Self {
        let mut hash = self.hash;
        mix_u64(&mut hash, 0xff51_afd7_ed55_8ccd);
        mix_u64(&mut hash, signature.len() as u64);
        for child_id in signature {
            mix_u64(&mut hash, stable_hash(child_id));
        }
        Self {
            hash,
            depth: self.depth.saturating_add(1),
        }
    }

    pub(crate) fn label(self) -> String {
        if self.depth == 0 {
            "root".to_string()
        } else {
            format!("{}:{:016x}", self.depth, self.hash)
        }
    }
}

#[derive(Default)]
struct StableHasher {
    hash: u64,
}

impl Hasher for StableHasher {
    fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.hash ^= u64::from(*byte);
            self.hash = self.hash.wrapping_mul(0x100_0000_01b3);
        }
    }

    fn finish(&self) -> u64 {
        self.hash
    }
}

fn stable_hash<T: Hash>(value: &T) -> u64 {
    let mut hasher = StableHasher {
        hash: PathPrefix::ROOT_HASH,
    };
    value.hash(&mut hasher);
    hasher.finish()
}

fn mix_u64(hash: &mut u64, value: u64) {
    for byte in value.to_le_bytes() {
        *hash ^= u64::from(byte);
        *hash = hash.wrapping_mul(0x100_0000_01b3);
    }
}

/// Scope for a learned group pattern. Prefix-scoped patterns are preferred at
/// lookup time; aggregate patterns preserve the previous signature-only
/// behavior as a cold-start fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum PatternScope {
    Aggregate,
    Path(PathPrefix),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct GroupPatternKey {
    scope: PatternScope,
    signature: Vec<MethodId>,
}

/// Tracked group pattern: the EMA of post-join remaining work observed for a
/// sorted multiset of child method ids.
#[derive(Debug)]
struct GroupPattern<E> {
    estimator: E,
    occurrence_count: u64,
}

/// Server-level fanout pattern table: `(root_api, parent_method) -> known group patterns`.
/// Indexing on root API keeps observations from different ingress API types
/// from contaminating each other when they share a downstream parent.
#[derive(Debug)]
pub(crate) struct FanoutPatternTable<E: LatencyEstimator + Default + 'static> {
    by_root_parent: Mutex<HashMap<(MethodId, MethodId), HashMap<GroupPatternKey, GroupPattern<E>>>>,
}

impl<E: LatencyEstimator + Default + 'static> Default for FanoutPatternTable<E> {
    fn default() -> Self {
        Self {
            by_root_parent: Mutex::new(HashMap::new()),
        }
    }
}

impl<E: LatencyEstimator + Default + 'static> FanoutPatternTable<E> {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Issue-time lookup. `base_signature` is sorted and is a lower bound on
    /// the final group signature. Prefer the closest compatible patterns under
    /// the same observable path prefix, and fall back to the aggregate
    /// signature-only table if the prefix-scoped pattern is cold.
    pub(crate) fn lookup_estimate(
        &self,
        root: MethodId,
        parent: MethodId,
        path_prefix: PathPrefix,
        base_signature: &[MethodId],
        cap: u64,
    ) -> AfterChildEstimates {
        let by_root_parent = self.by_root_parent.lock().unwrap();
        let groups = match by_root_parent.get(&(root, parent)) {
            Some(g) => g,
            None => {
                return AfterChildEstimates {
                    full: 0,
                    mean: 0,
                    floor: 0,
                };
            }
        };

        closest_compatible_lookup(groups, PatternScope::Path(path_prefix), base_signature, cap)
            .or_else(|| {
                closest_compatible_lookup(groups, PatternScope::Aggregate, base_signature, cap)
            })
            .unwrap_or_default()
    }

    /// Exit-time update for one observed group under a path prefix.
    pub(crate) fn update(
        &self,
        root: MethodId,
        parent: MethodId,
        scope: PatternScope,
        signature: Vec<MethodId>,
        after_child_us: u64,
    ) {
        let mut by_root_parent = self.by_root_parent.lock().unwrap();
        let groups = by_root_parent.entry((root, parent)).or_default();
        let key = GroupPatternKey { scope, signature };
        if let Some(p) = groups.get_mut(&key) {
            p.estimator.track(after_child_us);
            p.occurrence_count += 1;
        } else {
            let mut estimator = E::default();
            estimator.track(after_child_us);
            groups.insert(
                key,
                GroupPattern {
                    estimator,
                    occurrence_count: 1,
                },
            );
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.by_root_parent.lock().unwrap().is_empty()
    }

    pub(crate) fn for_each_pattern<
        F: FnMut(MethodId, MethodId, PatternScope, &[MethodId], &E, u64),
    >(
        &self,
        mut f: F,
    ) {
        let by_root_parent = self.by_root_parent.lock().unwrap();
        for ((root, parent), groups) in by_root_parent.iter() {
            for (key, p) in groups {
                f(
                    *root,
                    *parent,
                    key.scope,
                    &key.signature,
                    &p.estimator,
                    p.occurrence_count,
                );
            }
        }
    }
}

fn closest_compatible_lookup<E: LatencyEstimator + Default + 'static>(
    groups: &HashMap<GroupPatternKey, GroupPattern<E>>,
    scope: PatternScope,
    base_signature: &[MethodId],
    cap: u64,
) -> Option<AfterChildEstimates> {
    let exact_key = GroupPatternKey {
        scope,
        signature: base_signature.to_vec(),
    };
    if let Some(p) = groups.get(&exact_key) {
        if p.estimator.can_estimate() {
            return Some(pattern_estimates(p, cap));
        }
    }

    let closest_signature_len = groups
        .iter()
        .filter(|(key, p)| {
            key.scope == scope
                && is_multiset_subset(base_signature, &key.signature)
                && p.estimator.can_estimate()
        })
        .map(|(key, _)| key.signature.len())
        .min()?;

    let mut total_weight = 0u128;
    let mut full = 0u128;
    let mut mean = 0u128;
    let mut floor = 0u128;

    for (key, p) in groups {
        if key.scope != scope
            || key.signature.len() != closest_signature_len
            || !is_multiset_subset(base_signature, &key.signature)
            || !p.estimator.can_estimate()
        {
            continue;
        }
        let weight = u128::from(p.occurrence_count.max(1));
        total_weight += weight;
        full += u128::from(p.estimator.estimate().min(cap)) * weight;
        mean += u128::from(p.estimator.mean_estimate().min(cap)) * weight;
        floor += u128::from(p.estimator.mean_floor_estimate().min(cap)) * weight;
    }

    if total_weight == 0 {
        return None;
    }

    Some(AfterChildEstimates {
        full: (full / total_weight) as u64,
        mean: (mean / total_weight) as u64,
        floor: (floor / total_weight) as u64,
    })
}

fn pattern_estimates<E: LatencyEstimator + Default + 'static>(
    pattern: &GroupPattern<E>,
    cap: u64,
) -> AfterChildEstimates {
    AfterChildEstimates {
        full: pattern.estimator.estimate().min(cap),
        mean: pattern.estimator.mean_estimate().min(cap),
        floor: pattern.estimator.mean_floor_estimate().min(cap),
    }
}

/// `needle` is a multiset-subset of `hay` (both sorted ascending).
pub(crate) fn is_multiset_subset(needle: &[MethodId], hay: &[MethodId]) -> bool {
    if needle.len() > hay.len() {
        return false;
    }
    let mut i = 0;
    let mut j = 0;
    while i < needle.len() && j < hay.len() {
        match needle[i].cmp(&hay[j]) {
            std::cmp::Ordering::Equal => {
                i += 1;
                j += 1;
            }
            std::cmp::Ordering::Greater => {
                j += 1;
            }
            std::cmp::Ordering::Less => return false,
        }
    }
    i == needle.len()
}

/// One recovered fanout group, ordered by the time the group began.
#[derive(Debug, Clone)]
pub(crate) struct FanoutGroupObservation {
    pub signature: Vec<MethodId>,
    pub post_join_us: u64,
}

/// One recovered fanout group annotated with the observable path prefix that
/// precedes it in this parent invocation.
#[derive(Debug, Clone)]
pub(crate) struct PathFanoutObservation {
    pub path_prefix: PathPrefix,
    pub signature: Vec<MethodId>,
    pub post_join_us: u64,
}

/// Recover fanout groups from completed child records via
/// connected-components on interval overlap. Returns one (signature,
/// post_join_us) per group; siblings within a group share the post-join
/// time `parent_end - max(child.end)`. Children with no end_time
/// (cancelled / never-completed) are ignored.
#[allow(dead_code)]
pub(crate) fn recover_groups(
    children: &[ChildRecord],
    parent_end: Instant,
) -> Vec<(Vec<MethodId>, u64)> {
    recover_group_observations(children, parent_end)
        .into_iter()
        .map(|g| (g.signature, g.post_join_us))
        .collect()
}

/// Recover fanout groups in temporal order from completed child records.
pub(crate) fn recover_group_observations(
    children: &[ChildRecord],
    parent_end: Instant,
) -> Vec<FanoutGroupObservation> {
    if children.is_empty() {
        return Vec::new();
    }

    let mut completed: Vec<&ChildRecord> = children.iter().filter(|c| c.end.is_some()).collect();
    if completed.is_empty() {
        return Vec::new();
    }
    completed.sort_by_key(|c| c.start);

    let mut out = Vec::new();
    let mut group_max_end = completed[0].end.unwrap();
    let mut signature = vec![completed[0].child_id];

    for c in completed.into_iter().skip(1) {
        let end = c.end.unwrap();
        if c.start <= group_max_end {
            signature.push(c.child_id);
            group_max_end = group_max_end.max(end);
            continue;
        }

        push_group_observation(
            &mut out,
            std::mem::take(&mut signature),
            group_max_end,
            parent_end,
        );
        group_max_end = end;
        signature.push(c.child_id);
    }

    push_group_observation(&mut out, signature, group_max_end, parent_end);
    out
}

fn push_group_observation(
    out: &mut Vec<FanoutGroupObservation>,
    mut signature: Vec<MethodId>,
    group_max_end: Instant,
    parent_end: Instant,
) {
    signature.sort();
    let post_join = parent_end
        .saturating_duration_since(group_max_end)
        .as_micros() as u64;
    out.push(FanoutGroupObservation {
        signature,
        post_join_us: post_join,
    });
}

/// Recover fanout groups and annotate each group with the path prefix built
/// from the groups that precede it.
pub(crate) fn recover_path_groups(
    children: &[ChildRecord],
    parent_end: Instant,
) -> Vec<PathFanoutObservation> {
    let groups = recover_group_observations(children, parent_end);
    let mut prefix = PathPrefix::root();
    let mut out = Vec::with_capacity(groups.len());
    for group in groups {
        out.push(PathFanoutObservation {
            path_prefix: prefix,
            signature: group.signature.clone(),
            post_join_us: group.post_join_us,
        });
        prefix = prefix.append_signature(&group.signature);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use masa_core::LatencyEstimator;
    use std::time::Duration;

    fn mid(n: u64) -> MethodId {
        // Test-only: build a MethodId via the registry by registering
        // synthetic method names.
        use crate::MethodRegistry;
        use tonic_core::CowGrpcMethod;
        MethodRegistry::global()
            .get_or_register(CowGrpcMethod::new("FanoutTest", format!("m{}", n)))
    }

    #[derive(Debug, Default)]
    struct LastValueEstimator {
        value: u64,
        has_value: bool,
    }

    impl LatencyEstimator for LastValueEstimator {
        fn track(&mut self, value: u64) {
            self.value = value;
            self.has_value = true;
        }

        fn can_estimate(&self) -> bool {
            self.has_value
        }

        fn estimate(&self) -> u64 {
            self.value
        }
    }

    fn child(child_id: MethodId, start: Instant, end: Instant) -> ChildRecord {
        ChildRecord {
            child_id,
            start,
            end: Some(end),
            terminal: true,
        }
    }

    #[test]
    fn multiset_subset_basic() {
        let a = mid(1);
        let b = mid(2);
        let c = mid(3);
        assert!(is_multiset_subset(&[a], &[a, b]));
        assert!(is_multiset_subset(&[a, b], &[a, b, c]));
        assert!(!is_multiset_subset(&[a, a], &[a, b]));
        assert!(is_multiset_subset(&[a, a], &[a, a, b]));
        assert!(!is_multiset_subset(&[c], &[a, b]));
    }

    #[test]
    fn recover_groups_parallel() {
        let a = mid(101);
        let b = mid(102);
        let t0 = Instant::now();
        let children = vec![
            child(a, t0, t0 + Duration::from_millis(10)),
            child(
                b,
                t0 + Duration::from_millis(1),
                t0 + Duration::from_millis(20),
            ),
        ];
        let parent_end = t0 + Duration::from_millis(25);
        let groups = recover_groups(&children, parent_end);
        assert_eq!(groups.len(), 1);
        let (sig, post_join) = &groups[0];
        let mut want = vec![a, b];
        want.sort();
        assert_eq!(sig, &want);
        // post_join ≈ 5ms = 5000us; allow rounding.
        assert!((4500..=5500).contains(post_join), "post_join={}", post_join);
    }

    #[test]
    fn recover_groups_sequential() {
        let a = mid(201);
        let b = mid(202);
        let t0 = Instant::now();
        let children = vec![
            child(a, t0, t0 + Duration::from_millis(5)),
            child(
                b,
                t0 + Duration::from_millis(6),
                t0 + Duration::from_millis(10),
            ),
        ];
        let parent_end = t0 + Duration::from_millis(11);
        let groups = recover_groups(&children, parent_end);
        assert_eq!(groups.len(), 2);
    }

    #[test]
    fn recover_path_groups_tracks_observable_prefixes() {
        let x = mid(301);
        let y = mid(302);
        let a = mid(303);
        let b = mid(304);
        let t0 = Instant::now();
        let children = vec![
            child(x, t0, t0 + Duration::from_millis(10)),
            child(
                y,
                t0 + Duration::from_millis(1),
                t0 + Duration::from_millis(20),
            ),
            child(
                a,
                t0 + Duration::from_millis(30),
                t0 + Duration::from_millis(40),
            ),
            child(
                b,
                t0 + Duration::from_millis(31),
                t0 + Duration::from_millis(50),
            ),
            child(
                a,
                t0 + Duration::from_millis(60),
                t0 + Duration::from_millis(70),
            ),
            child(
                b,
                t0 + Duration::from_millis(61),
                t0 + Duration::from_millis(80),
            ),
        ];
        let parent_end = t0 + Duration::from_millis(90);
        let groups = recover_path_groups(&children, parent_end);

        let mut sig_xy = vec![x, y];
        sig_xy.sort();
        let mut sig_ab = vec![a, b];
        sig_ab.sort();
        let root = PathPrefix::root();
        let after_xy = root.append_signature(&sig_xy);
        let after_first_ab = after_xy.append_signature(&sig_ab);

        assert_eq!(groups.len(), 3);
        assert_eq!(groups[0].path_prefix, root);
        assert_eq!(groups[0].signature, sig_xy);
        assert_eq!(groups[0].post_join_us, 70_000);
        assert_eq!(groups[1].path_prefix, after_xy);
        assert_eq!(groups[1].signature, sig_ab);
        assert_eq!(groups[1].post_join_us, 40_000);
        assert_eq!(groups[2].path_prefix, after_first_ab);
        assert_eq!(groups[2].signature, sig_ab);
        assert_eq!(groups[2].post_join_us, 10_000);
    }

    #[test]
    fn prefix_scoped_lookup_separates_repeated_signatures() {
        let root = mid(401);
        let parent = mid(402);
        let a = mid(403);
        let b = mid(404);
        let mut sig_ab = vec![a, b];
        sig_ab.sort();
        let first_prefix = PathPrefix::root();
        let second_prefix = first_prefix.append_signature(&sig_ab);

        let table = FanoutPatternTable::<LastValueEstimator>::new();
        table.update(
            root,
            parent,
            PatternScope::Path(first_prefix),
            sig_ab.clone(),
            67_000,
        );
        table.update(
            root,
            parent,
            PatternScope::Path(second_prefix),
            sig_ab.clone(),
            7_000,
        );

        let first = table.lookup_estimate(root, parent, first_prefix, &sig_ab, u64::MAX);
        let second = table.lookup_estimate(root, parent, second_prefix, &sig_ab, u64::MAX);

        assert_eq!(first.full, 67_000);
        assert_eq!(second.full, 7_000);
    }

    #[test]
    fn lookup_prefers_exact_optional_signature_under_same_prefix() {
        let root = mid(501);
        let parent = mid(502);
        let a = mid(503);
        let b = mid(504);
        let sig_a = vec![a];
        let mut sig_ab = vec![a, b];
        sig_ab.sort();
        let prefix = PathPrefix::root();

        let table = FanoutPatternTable::<LastValueEstimator>::new();
        for _ in 0..3 {
            table.update(root, parent, PatternScope::Path(prefix), sig_a.clone(), 100);
        }
        table.update(root, parent, PatternScope::Path(prefix), sig_ab, 200);

        let estimate = table.lookup_estimate(root, parent, prefix, &sig_a, u64::MAX);

        assert_eq!(estimate.full, 100);
    }

    #[test]
    fn lookup_uses_smallest_compatible_signature_when_exact_is_cold() {
        let root = mid(511);
        let parent = mid(512);
        let a = mid(513);
        let b = mid(514);
        let c = mid(515);
        let mut sig_ab = vec![a, b];
        sig_ab.sort();
        let mut sig_abc = vec![a, b, c];
        sig_abc.sort();
        let prefix = PathPrefix::root();

        let table = FanoutPatternTable::<LastValueEstimator>::new();
        table.update(
            root,
            parent,
            PatternScope::Path(prefix),
            sig_ab.clone(),
            200,
        );
        table.update(root, parent, PatternScope::Path(prefix), sig_abc, 900);

        let estimate = table.lookup_estimate(root, parent, prefix, &[a], u64::MAX);

        assert_eq!(estimate.full, 200);
    }

    #[test]
    fn lookup_falls_back_to_aggregate_when_prefix_is_cold() {
        let root = mid(601);
        let parent = mid(602);
        let a = mid(603);
        let b = mid(604);
        let x = mid(605);
        let mut sig_ab = vec![a, b];
        sig_ab.sort();
        let cold_prefix = PathPrefix::root().append_signature(&[x]);

        let table = FanoutPatternTable::<LastValueEstimator>::new();
        table.update(
            root,
            parent,
            PatternScope::Aggregate,
            sig_ab.clone(),
            42_000,
        );

        let estimate = table.lookup_estimate(root, parent, cold_prefix, &sig_ab, u64::MAX);

        assert_eq!(estimate.full, 42_000);
    }
}
