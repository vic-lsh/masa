---
name: saturation-finder
description: >
  Find the saturation point and appropriate SLO for a Masa application API endpoint.
  Use this skill whenever the user wants to characterize an API's performance limits,
  find what RPS it can handle, determine a good SLO, or create an experiment config
  that covers the interesting load range. Triggers on phrases like "find the saturation
  point", "what RPS can this handle", "characterize this API", "find a good SLO",
  "what load range should I test", or "set up an experiment for this endpoint".
---

# Saturation Finder

Given a Masa application and an API endpoint, this skill iteratively discovers:
1. **The SLO** — the tail latency knee point (where p99 starts shooting up)
2. **The saturation point** — the RPS where goodput starts dropping under that SLO
3. **A final experiment config** — an RPS range of 0.5x–2x saturation with ~5 steps

The exploration uses the `fifo` policy (no early return, no admission control) to measure raw system behavior without interference from overload control mechanisms. This gives the truest picture of the system's natural capacity and latency characteristics.

## Inputs

Ask the user for these if not already provided:

- **Application**: which app (e.g., `hotel`, `socialnet`, `synthetic`)
- **API endpoint**: which API to test (e.g., `Search`, `Reservation`)
- **Starting RPS range** (optional): if the user has a rough idea of where saturation might be, start there. Otherwise, start broad.
- **Experiment name**: a descriptive name for the config directory (e.g., `hotel_search`, `hotel_reservation`). Default: `<app>_<api_lowercase>`.

## Phase 0: Create the initial experiment config

Look at an existing experiment config in `exp/<app>/in/` to understand the format. Each config directory contains:
- `gen_config.json` — RPS sweep, SLO, APIs, timing parameters
- `<app>.json` — service topology (copy from an existing config)
- `policies` — one policy per line

Create the config directory at `exp/<app>/in/<experiment_name>/` with:
- **Apis**: just the single API being tested
- **Slos**: start with a generous SLO (e.g., 200ms for hotel, 100ms for socialnet) — the goal is to NOT have the SLO interfere with finding the natural latency knee
- **Rps**: a broad initial sweep (e.g., `[1000, 2000, 4000, 6000, 8000, 10000, 15000]`). If the API is known to be lightweight (few downstream calls, no artificial delays), start higher. If it has heavy processing (fan-out, sleeps, many DB calls), start lower.
- **Gap**: `"exp"` (exponential inter-arrival)
- **WarmupSecs**: 20
- **DurationSecs**: 40 (sufficient for finding saturation; full experiments use 60)
- **policies**: `fifo` only (one line)
- Copy the app topology JSON from any existing config in that app

## Phase 1: Find the ballpark (broad sweep)

Run the experiment:
```bash
uv run -m exp_runner run <app> <experiment_name> --plot
```

Read `exp/<app>/plots/<experiment_name>/0/goodput_ALL_aggregated.csv` and identify:
- The highest RPS where goodput fraction is still ~1.0 (the "safe" zone)
- The RPS where goodput fraction drops below 0.95 (saturation onset)
- The RPS where goodput collapses (fraction < 0.5 or goes to 0)

If the entire sweep shows ~100% goodput, the range was too conservative — double the RPS values and rerun. If the sweep shows collapse at every point, halve the RPS values.

## Phase 2: Narrow the range

Update `gen_config.json` with a denser sweep around the saturation region found in Phase 1. Use 5-6 RPS steps spanning from "clearly fine" to "clearly saturated". For example, if Phase 1 showed 100% goodput at 8000 and collapse at 15000, try `[8000, 9000, 10000, 11000, 12000, 14000]`.

Rerun the experiment and read results again. You should now see a clear transition from ~100% goodput to degradation.

## Phase 3: Find the SLO (latency knee point)

With the narrowed results, compute latency percentiles from the raw CSV files. For each RPS level, read `exp/<app>/out/<experiment_name>/0/fifo/r<RPS>_<API>.csv` and compute mean, p50, p90, p99, and p99.9 latency.

Use this Python snippet:
```python
python3 -c "
import csv, statistics, os

base = 'exp/<app>/out/<experiment_name>/0/fifo'
for rps in [<rps_list>]:
    f = os.path.join(base, f'r{rps}_<API>.csv')
    if not os.path.exists(f):
        continue
    lats = []
    with open(f) as fh:
        reader = csv.DictReader(fh)
        for row in reader:
            lats.append(int(row['latency']))
    lats.sort()
    n = len(lats)
    if n == 0:
        print(f'{rps} RPS: no data')
        continue
    mean = statistics.mean(lats)
    p50 = lats[int(n*0.5)]
    p90 = lats[int(n*0.9)]
    p99 = lats[int(n*0.99)]
    p999 = lats[int(n*0.999)]
    print(f'{rps} RPS (n={n}): mean={mean/1000:.1f}ms  p50={p50/1000:.1f}ms  p90={p90/1000:.1f}ms  p99={p99/1000:.1f}ms  p99.9={p999/1000:.1f}ms')
"
```

**Finding the knee point:** The SLO should be set at the p99 latency where the system is moderately loaded but not yet saturated — roughly where p99 starts accelerating. Look for the RPS level where p99 roughly doubles compared to the low-load baseline. The p99 at that point is a good SLO candidate. Round to a clean number (e.g., 20ms → 20000μs, 50ms → 50000μs).

Concrete heuristic:
- Take the p99 at the lowest RPS (this is the "idle" tail latency)
- Find the first RPS where p99 is ≥ 3x the idle p99
- The SLO is the p99 at the RPS level just *below* that point, rounded up to a clean value

## Phase 4: Validate with CPU utilization

Read `exp/<app>/plots/<experiment_name>/cpu_summary.csv` to corroborate the saturation point. The bottleneck service should show high CPU (>80% at P90) near the saturation RPS. This confirms that saturation is real (CPU-bound) rather than an artifact of misconfiguration.

## Phase 5: Set the final config

Update `gen_config.json` with:
- **Slos**: the SLO found in Phase 3
- **Rps**: 5 steps spanning 0.5x to 2x the saturation point, roughly evenly spaced

Report to the user:
- The discovered SLO and how it was derived
- The saturation point and what's bottlenecking
- The final RPS range
- CPU utilization of key services

Then ask the user if they want to update the `policies` file with the full set of policies they want to compare (the exploration used `fifo` only, but the final config will likely need multiple policies).

## Important notes

- Run only one experiment at a time (shared output directories)
- The `fifo` policy (no `early`) is used deliberately — early return masks the natural latency curve and makes it harder to find the true knee point
- If latency jumps straight from low to client-timeout (1000ms) with no gradual increase, the system has a sharp cliff rather than a knee. In this case, use the p99 at the last "healthy" RPS as the SLO.
- DurationSecs=40 is fine for saturation finding; the user may want to increase to 60 for final experiments
