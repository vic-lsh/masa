# TailClipper

Independent `rpcstack::Module` implementing global arrival time propagation from
[TailClipper (SoCC 2024)](https://doi.org/10.1145/3698038.3698554), §§3–4.

Compose `TailClipperModule` using `policy_stack!`, select
`rpcstack_tonic::PolicyHooks<YourStack>` in generated stubs, and enable
`tokio/tailclipper` plus `tonic/masa` at the application. The root hook stamps a
Unix nanosecond arrival time; every child carries exactly that timestamp in the
`tailclipper` wire section and the generic scheduling header. An explicit root
stamp supports upstream ingress timestamping and deterministic replay. Entry
points must agree on time/order; downstream clocks do not matter. Zero is
reserved and rejected. Absent server metadata creates a fresh arrival stamp,
but clients should stamp before transport enqueue to obtain first-poll ordering.

The run queue is `rpcstack_sched::builtin::tailclipper::BinaryHeapRoundRobinQueue`.
Its third const parameter sets the sharing degree (default 6, preserving Masa's
existing setting); 1 gives global FCFS. It maintains the oldest C runnable tasks
and rotates them when they yield. Vacancies are filled without draining the
whole window, and newly runnable older tasks displace the youngest member.

**This is a cooperative Tokio adaptation, not the paper's ghOSt scheduler.**
One future poll is the slice. A long CPU-bound poll cannot be interrupted, and
there is no fixed CPU-time quantum. Use bounded work with `tailclipper::yield_slice().await` to get limited
processor sharing. Tokio's `yield_now` defers wakes until other runnable tasks
drain, temporarily taking the caller out of the active sharing window. OS-thread preemption, arbitrary blocking work, and
cross-thread multi-core sharing are not implemented. See
[the expressiveness report](../../docs/PAPER_RPC_POLICIES.md).

Run the independent two-hop example and tests with:

```sh
./scripts/test_paper_rpc.sh
```
