# Paper policies as an rpcstack expressiveness test

Baseline: Masa `363db12dc` (2026-10-12). The baseline already includes a
TailClipper-inspired queue and a Rajomon benchmark port. This work implements
independent module integration and tests the papers' mechanisms, rather than
counting the presence of those names as evidence of framework expressiveness.

Sources:

- Ng et al., [TailClipper, SoCC 2024](https://doi.org/10.1145/3698038.3698554),
  §§3.1–3.2, §4 and Algorithm 1;
  [author-hosted full text](https://research.chalmers.se/publication/544883/file/544883_Fulltext.pdf).
- Xing et al., [Rajomon, NSDI 2025](https://www.usenix.org/system/files/nsdi25-xing.pdf),
  §§3.1–3.5 and §4. The paper is authoritative where the existing benchmark port
  or [Go reference](https://github.com/pennsail/rajomon) differs.

## Findings

| Required behavior | Baseline | Implementation |
| --- | --- | --- |
| Policy-owned wire data, child hooks, rejection feedback | Supported | Both policies use existing sections/hooks |
| Root-client admission and response-price learning | No module hooks; application-managed | Generic before/after root hooks, including local rejection unwind |
| Scheduling before first handler poll | Hyper reads Masa's budget schema | Opaque scheduling header maps directly to task metadata |
| Immutable global arrival timestamp | Coupled to Masa budget policy | Independent `tailclipper` module; root stamp and unchanged fanout propagation |
| ORF limited sharing | Fixed six-task batch; evicts even with vacancies | Configurable const C; continuously maintained oldest runnable window |
| Per-interface overload control in co-located services | Global own price/queue signal | Isolated server controllers, interface-local measurements/prices |
| Rajomon token propagation | Subtracts own cost at every hop | Paper module preserves holdings across all hops (§3.4/§3.5) |
| Rejected server followed by a root client on the same thread | Stale parent TLS after before-poll rejection | Clear TLS on the early-return path |
| Preemptive CPU-time slices | Unsupported by cooperative Tokio | Still unsupported; explicit limitation below |

**Conclusion:** rpcstack's existing server/child hooks express Rajomon's server
controller and policy metadata. End-to-end autonomous policy construction needed
root hooks and a scheduling bridge. Those additions support Rajomon and a
cooperative adaptation of TailClipper. They do **not** make rpcstack capable of
faithfully reproducing TailClipper's preemptive OS scheduler for arbitrary code.

## Implementation and paper fidelity

`libs/tailclipper` owns only arrival metadata and its propagation. The existing
runtime queue in `libs/rpcstack-sched/src/builtin/tailclipper.rs` now keeps C oldest
runnable tasks active and rotates them at yield points. C=1 behaves as global
FCFS; sufficiently large C behaves as round robin. Equal timestamps have no
specified total order. Multiple ingress nodes must coordinate their clock/order,
as the paper requires. The default C remains 6 for compatibility, not a claimed
optimal value. Agents can instantiate a different C in their selected queue.

The paper uses ghOSt threads and enforced CPU quanta; Tokio polls futures
cooperatively. This port uses one poll as a slice, so unequal poll costs are not
equal CPU-time sharing. A CPU-bound non-yielding handler defeats this adaptation. The supplied
`tailclipper::yield_slice` immediately self-wakes; Tokio's `yield_now` instead
defers its wake until other runnable work drains and would release its ORF slot.
A real-runtime scheduling test exposed that distinction.
Faithful arbitrary-code TailClipper requires a preemptive execution substrate
(e.g. a ghOSt worker backend), thread-level arrival identity, and block/wake/exit
and quantum-expiry events. Merely adding a timer hook cannot safely interrupt a
Rust future. Multi-thread ORF, dynamic C adjustment and ghOSt integration are not
implemented here. This is a remaining framework capability gap, not an untested
claim of equivalence.

`libs/rajomon/src/paper.rs` implements the paper policy separately from the
historical `ac_rajomon` benchmark path (the shared client worker lifecycle and
refill-interval fixes affect both). Each service owns its controller; each
interface has local price and a maximum queue-delay observation over the current
window. A periodic worker applies the proportional increase / one-token decrease
rule and hysteresis. Advertised prices are own plus max downstream. The same
holdings reach every branch; responses, including overload errors, lazily carry
prices upstream. Root hooks use the existing uniform-random token spending and
exponential refill mechanism. Explicit root tokens bypass spending to support
external controllers and reproducible tests. Missing server tokens fail closed.

The policy reads the runtime's measured enqueue-to-poll delay, not end-to-end
handler duration. It uses a window maximum as the aggregation choice; the paper
does not prescribe an exact statistic. Default parameters are inherited from
Masa, not retuned to reproduce either paper's published numbers. A client token
pool remains process-wide. This supports several client processes but does not
model independent tenants inside a single process. Call topology is learned
from responses; a previously unseen edge has no cached price until feedback.

## Hook and transport changes

Root hooks are static module methods because there is no inbound request/module
instance. `ChildState` carries per-call data; modules own shared client state.
Before hooks run head-first; a rejection notifies exactly the entered modules in
reverse order. Response hooks run tail-first and receive error metadata too.
Generated unary clients use the new fallible `try_before_send` hook, whose default
preserves existing `before_send` implementations. Regenerate stubs when upgrading.
Caller-supplied root wire sections are preserved. Parent-created child RPCs do
not run root hooks or spend root tokens again.

The optional `x-rpcstack-scheduling` header contains an opaque decimal u64. Hyper
can schedule the first poll without importing a particular policy's wire type.
Missing or malformed scheduling headers retain the old Masa-context fallback;
this is not a new universal ingress protocol. The example stamps before sending
and needs no Masa budget. Changing the selected queue remains a compile-time
application/runtime choice.

Existing cancellation semantics remain: after hooks observe sent responses or
local rejection, not dropped RPC futures. These implementations do not maintain
per-child resources that need cancellation refunds. Streaming RPCs, arbitrary
wire protocols and kernel scheduling are not covered by the unary module API.

## Validation and agent workflow

Run `./scripts/test_paper_rpc.sh` for unit, hook-composition, scheduling and real
HTTP/2 tests plus a runnable two-hop demo. `./scripts/check.sh` checks the existing
feature matrix with warnings denied. The dependency boundary validator also
ensures TailClipper imports no Masa policy crate and framework crates import no
paper policy.

Tests cover root short-circuit/unwind, error metadata feedback, exclusion of root
hooks from child calls, GAT fanout propagation, C=1 and C=2 scheduling, repeated
self-wakes, vacancy refill, older wakeup displacement, price hysteresis, service
and interface isolation, saturation, and two-hop token/arrival preservation.
The HTTP/2 test caught the stale-parent-context rejection bug; hook-only tests
would not have exposed that runtime interaction.

For VibeSys, a useful candidate surface is the policy module plus its queue and
stack selection. Keep the transport tests and scheduling invariants outside the
candidate's editable surface when using them as a judge. The example build file
shows selecting an agent-owned stack without patching generated application
handlers. No changes to the VibeSys repository are required by this experiment.

No paper benchmark, throughput improvement, tail-latency improvement or
production-readiness claim follows from these correctness tests. Reproducing
performance requires the workload, hardware and execution model used by each
paper; TailClipper's substrate mismatch must be resolved first for a faithful
preemptive comparison.
