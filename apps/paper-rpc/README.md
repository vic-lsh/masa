# Standalone paper policy example

```sh
cargo run -p paper-rpc --features orf
cargo test -p paper-rpc --features orf
```

Starts two ephemeral localhost gRPC servers, calls frontend → leaf, and prints
the leaf's immutable arrival timestamp and token count. Generated client/server
stubs select `PolicyHooks<policy_stack![TailClipperModule, PaperRajomonModule]>`.
There is no Masa budget or deadline module, nor application-level token logic.
The `orf` feature selects the TailClipper queue and the transport scheduling
adapter; it is opt-in to avoid changing scheduler selection in workspace builds.

The transport test supplies replay metadata, confirms identical holdings and
arrival time after two hops, verifies price feedback on rejection, and checks
that a subsequent unfunded root call is rejected locally. Library tests cover
fanout, per-interface pricing and queue scheduling. This is a correctness
fixture, not a reproduction of the papers' performance experiments.
