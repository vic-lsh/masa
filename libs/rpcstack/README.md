# rpcstack

A module framework for RPC policies. A policy is a stack of modules; the
framework owns every mechanism and a module states only what it does at each
hook. The framework knows a request only by its service and method name: it has
no notion of deadlines, priorities or any other value a module carries, and it
never defaults or interprets one.

Depends on `rpcstack-wire` and on tonic's `Request`, `Response`, `Status` and
`CowGrpcMethod`. It depends on no Masa crate, hyper or tokio runtime policy
crate (`scripts/validate_rpcstack_boundary.py` checks this).

Public surface:

- `Module`: the module trait (`NAME`, `Wire`, `Server`, `requires`, `new`,
  `before_poll`, `before_child_rpc`, `seal_child_rpc`, `after_child_rpc`,
  `after_poll`, `finalize`). `ModuleServer` is its server-level state.
- `Stack`, `()` and `policy_stack!`: composition. Pre-hooks run in stack order
  and the first `Err` short-circuits; `seal_child_rpc`, `after_child_rpc` and
  `finalize` run in reverse order, only for modules whose pre-hook ran;
  `after_poll` runs in stack order. `()` is the empty module for a disabled
  slot. `ModuleStack` is the trait stacks implement; modules never do.
- `build_server::<S>(service_name)`: builds a stack's server state after
  checking that every `Module::NAME` is unique (the empty module is exempt;
  a module with wire data also needs a name that fits a section) and every
  declared dependency is met. Panics on a duplicate name.
- `Requires`, `MissingDependency`, `ServerInit`: declared dependencies between
  modules (`Module::requires`) and server state shared through
  `ServerInit::provide` and `require`.
- `Extensions` and `ChildState`: typed per-request and per-child-RPC maps.
  Decision points on them: `propose`, `proposals` and `resolve`, with
  `Proposal`, `Proposals` and `DecisionClosed`. Several modules propose to a
  decision and its owner resolves it by its own rule.
- `Outcome` and `ChildOutcome`: how a request ended, and how a child RPC ended,
  as `finalize` and `after_child_rpc` see them.
- `WireIn`, `WireOut`, `EncodedSection`, `WireError`, `peek`, `describe`,
  `HEADER_NAME`: the typed wire codec. `WireIn` is a borrowed view of an
  inbound header that decodes one section on demand; `WireOut` builds the
  outbound header in place from the sections a module puts (up to 128 bytes
  without allocating). A payload is `bincode`, so a wire type writes every
  field every time and uses an `Option` for "may be absent". A missing section
  is `None`; what that means is up to the module. `EncodedSection` carries a
  received section on unchanged, without encoding it again; `describe` and
  `Display` for `WireIn` print the sections in hex.
- `Module::POLL_HOOKS` (default `true`): a module that keeps both poll hooks at
  their defaults sets it to `false`, and a stack in which no module has poll
  hooks skips them and the per-request lock they need. Debug builds panic if a
  module declares `false` and overrides a poll hook.
- `Early` and `Rejection`: what a stack returns when a module ends a request or
  rejects a child RPC; used by hook adapters such as `rpcstack-tonic`.

`rpcstack-tonic` runs a stack as tonic's request hooks.

Root clients use `Module::before_root_rpc` and `after_root_rpc` (static hooks;
there is no server request instance). They run in stack/reverse-stack order,
respectively, with per-call `ChildState`. Rejection short-circuits and unwinds
only entered modules with `ChildOutcome::Rejected`. A sent call reports both
successful and error response metadata. Shared client state belongs to the
module. The tonic adapter invokes these automatically for unary calls without
a parent; child RPCs continue to use the existing child hooks. Cancellation
before a response still drops state without an after hook, as for child RPCs.
