# Fanout-Aware Downstream Work Estimation
 
## Problem
 
Masa estimates each handler's *post-child remaining work* — the time a parent
handler will still spend after a child RPC returns — to derive child RPC
deadlines as `D_parent − E[remaining_work]`. The estimator must handle both
sequential and parallel children correctly.
 
A naive per-edge EMA (one estimate per *(parent_method, child_method)* pair)
fails on parallel fanout: siblings that ran concurrently see different
"after-child times" depending on their finish order, even though they share
the same post-join cost. This produces:
 
- **Inverted priorities**: the fastest sibling gets the tightest deadline,
  even though it is not on the critical path.
- **Inflated variance**: the per-edge EMA absorbs sibling-position noise that
  is unrelated to downstream work.
The fix is to estimate per *fanout group*, not per edge. Sequential children
are the singleton case.
 
## Definitions
 
A **fanout group** is a maximal set of child RPCs whose wall-clock lifetime
intervals overlap. Equivalently: the parent was awaiting all of them
simultaneously at some point. Two children belong to the same group iff
their intervals overlap, regardless of how the application expressed the
concurrency (`join!`, `JoinSet`, bare `tokio::spawn`).
 
The **post-join remaining work** for a group is
 
```
handler_exit_time − max(child.end_time for child in group)
```
 
This is one number per group per handler invocation: all members of the
group share it.
 
The **signature** of a group is the sorted multiset of its members'
`child_method` identifiers. Multiplicities matter: `(Geo, Geo, Geo)` is
distinct from `(Geo,)`.
 
## Data Structures and Lifetimes
 
### Server scope (long-lived, shared across all requests)
 
```
pattern_table: HashMap[parent_method -> ParentPatternState]
 
ParentPatternState:
    known_groups:    list of GroupPattern
 
GroupPattern:
    signature:        sorted multiset of child_method
    ema:              EMAState
    occurrence_count: int
```
 
`known_groups` records the fanout patterns this parent has been observed to
exhibit. In steady state it contains 1–3 entries per parent method,
bounded by the number of structurally distinct fanout shapes in the
handler's code.
 
### Handler scope (one per parent handler invocation)
 
```
HandlerInvocation:
    parent_method:   string
    children:        list of ChildRecord
    handler_start:   timestamp
```
 
Created at handler entry, destroyed at handler exit.
 
### Child-RPC scope (one per child RPC issued)
 
```
ChildRecord:
    child_method:    string
    start_time:      timestamp
    end_time:        timestamp or NIL
```
 
Created at `before_child`, finalized at `after_child`. Lives within its
parent's `HandlerInvocation`.
 
## Algorithm
 
The algorithm has two halves that decouple cleanly:
 
- **Issue time** (`on_before_child`): predict which fanout group this child
  will join, look up that group's EMA, set the child's deadline.
- **Exit time** (`on_handler_exit`): recover ground-truth groups from
  observed intervals, fold one sample into each group's EMA.
```
on_before_child(invocation, child_method, parent_deadline) -> deadline:
    record = ChildRecord(child_method, start=now(), end=NIL)
    invocation.children.append(record)
 
    # Compute lower bound on this child's eventual fanout group:
    # every still-open earlier child must overlap us (their end times are
    # in the future), so they are guaranteed group members.
    open_earlier = [c for c in invocation.children
                    if c.end_time is NIL and c is not record]
    base_signature = sorted([c.child_method for c in open_earlier]
                            + [child_method])
 
    # Find known patterns whose signatures are multiset-supersets of
    # base_signature: those are the patterns this fanout could still be.
    state = pattern_table.get(invocation.parent_method)
    candidates = [
        p for p in (state.known_groups if state else [])
        if is_multiset_subset(base_signature, p.signature)
    ]
 
    if candidates is empty:
        est = 0  # cold start or novel pattern: no subtraction
    else:
        # Smallest superset wins (closest to fitting exactly);
        # tie-break on occurrence count.
        best = argmax(candidates,
                      by=(-len(p.signature), p.occurrence_count))
        est = best.ema.value
 
    return parent_deadline - est
 
 
on_after_child(record):
    record.end_time = now()
 
 
on_handler_exit(invocation):
    handler_exit_time = now()
    state = pattern_table.get_or_init(invocation.parent_method)
 
    # Recover ground-truth groups: connected components on interval overlap.
    groups = connected_components_by_overlap(invocation.children)
 
    for group in groups:
        signature   = sorted([c.child_method for c in group])
        join_time   = max(c.end_time for c in group)
        after_child = handler_exit_time - join_time
 
        pattern = state.known_groups.find(p => p.signature == signature)
        if pattern is NIL:
            pattern = GroupPattern(signature, EMAState.new(), count=0)
            state.known_groups.append(pattern)
 
        pattern.ema.update(after_child)
        pattern.occurrence_count += 1
 
 
connected_components_by_overlap(children) -> list of groups:
    # Sweep by start_time; union-find any pair whose intervals overlap.
    sorted_children = sort children by start_time
    uf = UnionFind(child indices)
    active = []
    for child in sorted_children:
        active = [c for c in active if c.end_time >= child.start_time]
        for c in active:
            uf.union(child, c)
        active.append(child)
    return uf.components()
```
 
## Why It Works
 
The algorithm rests on three facts:
 
**Wall-clock interval overlap is the ground-truth definition of fanout.**
Two children belong to the same group iff their intervals overlap,
regardless of how the application expressed the concurrency. At handler
exit, all start/end times are observable, so connected-components on
overlap recovers groups correctly with no help from the application.
 
**Within a group, all members share the same post-join remaining work.**
The post-join time `handler_exit − max(child_end)` is one quantity per
group, so all siblings should share one EMA. This is why the EMA is keyed
on group signature, not on the per-(parent, child) edge: the edge-keyed
EMA would absorb sibling-position variance that has nothing to do with
downstream work.
 
**At issue time, what we have already issued is a multiset-subset of the
eventual group.** Every still-open earlier child must overlap the child
being issued (its end time is still in the future), so they are guaranteed
group members. Future siblings might join the group too, but we do not
know yet. So `base_signature` is a lower bound on the eventual group's
signature, and known patterns whose signatures are multiset-supersets of
`base_signature` are exactly the patterns this fanout could still turn out
to be.
 
The two halves decouple cleanly: exit time updates EMAs from observed
structure (always correct, because intervals are observable), while issue
time predicts from history (sometimes wrong on the first invocation of a
new pattern, but self-corrects within a few invocations because EMAs are
updated from ground truth, not from predictions). Mispredictions affect
deadlines on the wire for one invocation; they do not corrupt the state
that drives future predictions.
 
## Worked Examples
 
### Pure sequential handler
 
Handler issues `Geo`, awaits, then issues `Rate`, awaits.
 
| Event                  | open_earlier | base_signature |
|------------------------|--------------|----------------|
| `before_child(Geo)`    | ∅            | `(Geo,)`       |
| `after_child(Geo)`     | —            | —              |
| `before_child(Rate)`   | ∅            | `(Rate,)`      |
 
Two singleton groups, recovered as separate components at exit.
 
### Pure parallel fanout
 
Handler issues `Geo`, `Rate`, `Profile` concurrently, awaits all three.
 
| Event                    | open_earlier      | base_signature           |
|--------------------------|-------------------|--------------------------|
| `before_child(Geo)`      | ∅                 | `(Geo,)`                 |
| `before_child(Rate)`     | `{Geo}`           | `(Geo, Rate)`            |
| `before_child(Profile)`  | `{Geo, Rate}`     | `(Geo, Profile, Rate)`   |
 
Each successive call sees a growing signature. All three are
multiset-subsets of the same eventual pattern `(Geo, Profile, Rate)`,
which they all match in the pattern table once it is populated.
 
### Mixed sequential + parallel
 
Handler fans out to `{Geo, Rate}`, awaits both, then sequentially calls
`Cache`.
 
| Event                   | open_earlier   | base_signature   |
|-------------------------|----------------|------------------|
| `before_child(Geo)`     | ∅              | `(Geo,)`         |
| `before_child(Rate)`    | `{Geo}`        | `(Geo, Rate)`    |
| `after_child(Geo)`      | —              | —                |
| `after_child(Rate)`     | —              | —                |
| `before_child(Cache)`   | ∅              | `(Cache,)`       |
 
`Cache` does not see `Geo` and `Rate` in its `base_signature` because they
are closed by the time `Cache` is issued. This is what lets the algorithm
distinguish the second fanout from the first.
 
## Complexity
 
Let *N* = children issued by one handler invocation, *K* = known patterns
for this parent_method, *M* = max signature length among known patterns.
 
| Operation              | Time                       |
|------------------------|----------------------------|
| `on_before_child`      | O(N + K·M)                 |
| `on_after_child`       | O(1)                       |
| `on_handler_exit`      | O(N log N + N·α(N) + G·M)  |
 
Memory:
 
- Per invocation (handler scope): O(N), discarded at handler exit.
- Per server (long-lived): O(P · K · M), where *P* = distinct parent
  methods. *K* and *M* are bounded by application complexity, not by
  traffic.
In practice *N* is 1–20, *K* is 1–3, *M* is 1–10. Per-RPC overhead is a
small number of microseconds — well below gRPC marshalling cost.
 
## Failure Modes
 
**Cold start.** No patterns recorded yet for this parent method. Issue
time falls back to `est = 0` (child gets parent's deadline, no
prioritization advantage). The first invocation pays this cost; the
pattern is registered at handler exit and subsequent invocations benefit.
 
**Novel pattern.** Handler produces a fanout shape never seen before.
Issue-time prediction may match a related but larger pattern, giving
slightly inaccurate deadlines for that one invocation. The new pattern is
registered at handler exit; subsequent invocations match correctly.
 
**Pattern drift.** Post-join work for an existing pattern shifts (e.g.,
deployment changes downstream behavior). The EMA tracks the new mean
within a few invocations. Predictions stay correctly *typed* — they
identify the right pattern — and just track shifting values.
 
**Frequent novel patterns.** A handler whose fanout shape genuinely
varies invocation-to-invocation. The pattern table grows without bound.
Mitigation: LRU eviction on `occurrence_count`. Not implemented by
default; only needed if evaluation surfaces this case.
 
**Misprediction at issue time when multiple known patterns share a
prefix.** Disambiguation rule (smallest superset; tie-break on occurrence
count) picks the maximum-likelihood guess in steady state. Affects only
the deadlines on one invocation's wire; does not corrupt the EMAs
themselves.
 
## Notes on the Open/Closed Distinction
 
`base_signature` includes only *open* earlier children, not all earlier
children. Closed children belonged to *earlier* groups in this
invocation, which have already wrapped up (interval-wise) before this
child started. This is what lets the algorithm distinguish a second
fanout from a first in a handler that has both.
 
This is a correctness property, not a performance optimization. If we
naively included all earlier children in `base_signature`, we would never
match any single-fanout pattern after the first fanout completed.
