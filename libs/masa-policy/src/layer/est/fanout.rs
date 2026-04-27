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
// observable). At child-issue time we predict which group the child will
// join from a lower bound (the still-open earlier siblings + this child)
// and look up the smallest matching known signature.

use std::collections::HashMap;
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
}

/// Tracked group pattern: a sorted multiset of child method ids and the EMA
/// of post-join remaining work observed for that pattern.
#[derive(Debug)]
struct GroupPattern<E> {
    signature: Vec<MethodId>,
    estimator: E,
    occurrence_count: u64,
}

/// Server-level fanout pattern table: `parent_method -> known group patterns`.
#[derive(Debug)]
pub(crate) struct FanoutPatternTable<E: LatencyEstimator + Default + 'static> {
    by_parent: Mutex<HashMap<MethodId, Vec<GroupPattern<E>>>>,
}

impl<E: LatencyEstimator + Default + 'static> Default for FanoutPatternTable<E> {
    fn default() -> Self {
        Self {
            by_parent: Mutex::new(HashMap::new()),
        }
    }
}

impl<E: LatencyEstimator + Default + 'static> FanoutPatternTable<E> {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Issue-time lookup. `base_signature` is sorted; returns estimates from
    /// the smallest known signature that is a multiset-superset of it,
    /// tie-breaking on occurrence_count. Returns all-zero if no candidates.
    pub(crate) fn lookup_estimate(
        &self,
        parent: MethodId,
        base_signature: &[MethodId],
        cap: u64,
    ) -> AfterChildEstimates {
        let by_parent = self.by_parent.lock().unwrap();
        let groups = match by_parent.get(&parent) {
            Some(g) => g,
            None => {
                return AfterChildEstimates {
                    full: 0,
                    mean: 0,
                    floor: 0,
                };
            }
        };

        let mut best: Option<&GroupPattern<E>> = None;
        for p in groups {
            if !is_multiset_subset(base_signature, &p.signature) {
                continue;
            }
            if !p.estimator.can_estimate() {
                continue;
            }
            best = Some(match best {
                None => p,
                Some(b) => {
                    if p.signature.len() < b.signature.len()
                        || (p.signature.len() == b.signature.len()
                            && p.occurrence_count > b.occurrence_count)
                    {
                        p
                    } else {
                        b
                    }
                }
            });
        }

        match best {
            Some(p) => AfterChildEstimates {
                full: p.estimator.estimate().min(cap),
                mean: p.estimator.mean_estimate().min(cap),
                floor: p.estimator.mean_floor_estimate().min(cap),
            },
            None => AfterChildEstimates {
                full: 0,
                mean: 0,
                floor: 0,
            },
        }
    }

    /// Exit-time update for one observed group.
    pub(crate) fn update(&self, parent: MethodId, signature: Vec<MethodId>, after_child_us: u64) {
        let mut by_parent = self.by_parent.lock().unwrap();
        let groups = by_parent.entry(parent).or_default();
        if let Some(p) = groups.iter_mut().find(|p| p.signature == signature) {
            p.estimator.track(after_child_us);
            p.occurrence_count += 1;
        } else {
            let mut estimator = E::default();
            estimator.track(after_child_us);
            groups.push(GroupPattern {
                signature,
                estimator,
                occurrence_count: 1,
            });
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.by_parent.lock().unwrap().is_empty()
    }

    pub(crate) fn for_each_pattern<F: FnMut(MethodId, &[MethodId], &E, u64)>(&self, mut f: F) {
        let by_parent = self.by_parent.lock().unwrap();
        for (parent, groups) in by_parent.iter() {
            for p in groups {
                f(*parent, &p.signature, &p.estimator, p.occurrence_count);
            }
        }
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

/// Recover fanout groups from completed child records via
/// connected-components on interval overlap. Returns one (signature,
/// post_join_us) per group; siblings within a group share the post-join
/// time `parent_end - max(child.end)`. Children with no end_time
/// (cancelled / never-completed) are ignored.
pub(crate) fn recover_groups(
    children: &[ChildRecord],
    parent_end: Instant,
) -> Vec<(Vec<MethodId>, u64)> {
    let n = children.len();
    if n == 0 {
        return Vec::new();
    }

    // Filter to completed children, keeping original indices into a fresh
    // contiguous list.
    let mut completed: Vec<&ChildRecord> = children.iter().filter(|c| c.end.is_some()).collect();
    if completed.is_empty() {
        return Vec::new();
    }
    completed.sort_by_key(|c| c.start);

    // Union-find over indices into `completed`.
    let m = completed.len();
    let mut parent_uf: Vec<usize> = (0..m).collect();
    fn find(uf: &mut [usize], mut x: usize) -> usize {
        while uf[x] != x {
            uf[x] = uf[uf[x]];
            x = uf[x];
        }
        x
    }
    fn union(uf: &mut [usize], a: usize, b: usize) {
        let ra = find(uf, a);
        let rb = find(uf, b);
        if ra != rb {
            uf[ra] = rb;
        }
    }

    // Sweep by start time; pairs whose intervals overlap get unioned.
    // active[i] is still active iff its end >= current.start.
    let mut active: Vec<usize> = Vec::new();
    for (i, c) in completed.iter().enumerate() {
        let c_start = c.start;
        active.retain(|&j| completed[j].end.unwrap() >= c_start);
        for &j in &active {
            union(&mut parent_uf, i, j);
        }
        active.push(i);
    }

    // Group by root.
    let mut by_root: HashMap<usize, Vec<usize>> = HashMap::new();
    for i in 0..m {
        let r = find(&mut parent_uf, i);
        by_root.entry(r).or_default().push(i);
    }

    let mut out = Vec::with_capacity(by_root.len());
    for (_, members) in by_root {
        let mut sig: Vec<MethodId> = members.iter().map(|&i| completed[i].child_id).collect();
        sig.sort();
        let group_max_end = members
            .iter()
            .map(|&i| completed[i].end.unwrap())
            .max()
            .unwrap();
        let post_join = parent_end
            .saturating_duration_since(group_max_end)
            .as_micros() as u64;
        out.push((sig, post_join));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn mid(n: u64) -> MethodId {
        // Test-only: build a MethodId via the registry by registering
        // synthetic method names.
        use crate::MethodRegistry;
        use tonic_core::CowGrpcMethod;
        MethodRegistry::global()
            .get_or_register(CowGrpcMethod::new("FanoutTest", format!("m{}", n)))
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
            ChildRecord {
                child_id: a,
                start: t0,
                end: Some(t0 + Duration::from_millis(10)),
            },
            ChildRecord {
                child_id: b,
                start: t0 + Duration::from_millis(1),
                end: Some(t0 + Duration::from_millis(20)),
            },
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
            ChildRecord {
                child_id: a,
                start: t0,
                end: Some(t0 + Duration::from_millis(5)),
            },
            ChildRecord {
                child_id: b,
                start: t0 + Duration::from_millis(6),
                end: Some(t0 + Duration::from_millis(10)),
            },
        ];
        let parent_end = t0 + Duration::from_millis(11);
        let groups = recover_groups(&children, parent_end);
        assert_eq!(groups.len(), 2);
    }
}
