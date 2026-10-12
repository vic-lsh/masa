# rajomon

Rajomon admission control (NSDI '25, "Rajomon: Decentralized and Coordinated
Overload Control for Latency-Sensitive Microservices") as an `rpcstack` module.
It depends on the framework crates and generic libraries only, never on a Masa
crate or hyper; `scripts/validate_rpcstack_boundary.py` checks this.

- `RajomonModule`: the module. Admits a request iff its tokens cover the
  accumulated price (own price plus the largest downstream price), forwards the
  remaining tokens to child RPCs, learns downstream prices from child responses
  and, with probability `1/price_freq`, advertises its price in its own response.
- `RajomonWire`: the module's wire section (`tokens`, optional `price`).
- `RAJOMON_STATE` / `RajomonSharedState`: process-wide price tables and the
  price-update worker, started by the first request inside a tokio runtime.
- `CLIENT_TOKEN_BUCKET` / `ClientTokenBucket`: the client-side token bucket that
  root clients spend from and a replenishment worker refills.
- `RajomonParams`: tunables. `RajomonParams::global()` reads, once, the JSON file
  named by `RAJOMON_PARAMS_PATH`, or else `MASA_POLICY_PARAMS_PATH`, and takes
  its `"rajomon"` section; other sections are ignored, so a Masa params file
  works unchanged. Unset or unreadable means defaults.

`tests/toy_stack.rs` runs the module in a stack of the framework alone.

## Paper-oriented module

`PaperRajomonModule` is an alternative implementation of NSDI '25 §3, suitable
for a standalone framework stack. It uses the same `rajomon` wire section and
`RajomonParams`, but separate server state per constructed service and separate
queue observations/local prices per interface. `before_root_rpc` automatically
spends from the existing Poisson-refilled client bucket; `after_root_rpc` learns
prices even from error statuses. Explicit root wire tokens allow externally
managed clients/replay without double spending.

Unlike the historical `RajomonModule`, it forwards token holdings unchanged
(§3.4/§3.5), does not consult an unrelated parent's child-price cache, resets the
queue observation each update window, and does not introduce downstream price
aging. Local price rises proportionally above the queue threshold, falls by one
below half the threshold, and holds in between. Total price is local plus the
maximum relevant downstream price. Responses advertise with probability
`1/price_freq`; zero disables advertisements. Rejected requests also finalize.
A missing token section is an invalid request rather than a default allowance.

Compose one Rajomon module at a time: both claim the same wire section. The
existing `ac_rajomon` feature keeps selecting the historical implementation, rather than silently replacing it. Client refill startup now retries after
being invoked without a runtime or after runtime shutdown, and samples full
exponential intervals instead of truncating them at 10 seconds.

See [paper-rpc](../../apps/paper-rpc/) for automatic root-to-leaf integration and
[the report](../../docs/PAPER_RPC_POLICIES.md) for scope and limitations.
