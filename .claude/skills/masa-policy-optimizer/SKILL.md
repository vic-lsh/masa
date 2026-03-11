---
name: masa-policy-optimizer
description: >
  Iteratively optimize a Masa scheduling policy (specified as a Cargo feature flag combination,
  e.g. "prio_local,est_mean_var,early") for goodput. Use this skill whenever the user asks to
  improve, optimize, tune, experiment with, or analyze any Masa policy — even if they say
  something casual like "let's keep iterating on this policy" or "can we try a different approach".
  The skill manages the full hypothesis → code edit → experiment → analysis → decision loop,
  tracking all thinking in a persistent per-policy markdown file.
---

# Masa Policy Optimizer

## Before you start

Ask the user for these inputs if not already provided:

- **Target policy**: the feature flag combination being optimized (e.g., `prio_local,est_mean_var,early`)
- **Baseline policies**: the policies to beat — default is `prio_oldest,early`. Ask if there are others to compare.
- **Number of iterations**: how many hypothesis-experiment cycles to run before stopping for review. Ask if not specified.
- **Tracking markdown**: Should we start a new file, or resume from an existing one? They may have a prior iteration doc from a previous optimization run (e.g., `PRIO_LOCAL_IMPROVEMENTS.md`). If resuming, read the file fully before doing anything — understand what has already been tried and what iteration number to continue from.
- **Experiment config**: Ask the user to either:
  - Provide a path to an existing config directory (`exp/<app>/in/<name>/`) to use as the baseline, or
  - Describe in natural language what the experiment should look like (which app, what load schedule, what SLO, etc.) so you can create a new config from scratch.
- **App**: infer from the config path, or ask if describing from scratch.

**Tracking markdown naming and setup:** Each optimization track gets a randomly chosen single English word as its codeword (e.g., `falcon`, `ember`, `quarry`). Pick something memorable but arbitrary — avoid policy names or technical terms that could cause confusion. The markdown is named `<CODEWORD>.md` and stored in `exp/docs/` (e.g., `exp/docs/falcon.md`). Create the directory if it doesn't exist.

At the very top of a new markdown, write a header like:

```markdown
# <CODEWORD> — <target policy>

## Key questions
- <The central question this track is trying to answer, e.g. "Can prio_local,est_mean_var,early beat prio_oldest,early under realistic load variation?">
- <Any secondary questions, e.g. "Does the EMA estimator create feedback loops under non-monotonic load schedules?">

## Experiment series: <codeword>_1, <codeword>_2, ... (<app>)
```

When resuming an existing markdown, read the key questions at the top to re-anchor your thinking before doing anything else. Never modify the key questions retroactively — if the investigation pivots, add a note under the relevant iteration explaining why.

## Phase 0: Baseline run (always required)

Before any code changes, run an initial experiment to establish the baseline performance of all policies under comparison. This is the reference point for every subsequent iteration.

**If the user provided an existing config path:** Keep the original config name as-is (e.g., `s1467_3`) — do not rename it. Note it as the baseline reference in the tracking markdown. The `<codeword>_N` naming series starts from the first *new* experiment you create (e.g., `<codeword>_2` for the first hypothesis iteration). Check whether results already exist at `exp/<app>/plots/<name>/goodput_ALL_aggregated.csv`. If they do, skip the run and go straight to analysis. If not, verify the `policies` file includes both the target policy and all baselines (add any missing ones), then run.

**If the user described the experiment in natural language:** Create a new config directory named `<codeword>_1` (e.g., `falcon_1`). Use a subagent to:
1. Find the closest existing config in `exp/<app>/in/` as a template
2. Adapt it to match the user's description (load schedule, SLO, replicas, etc.)
3. Write the config files and confirm with the user before running

Run the baseline experiment:
```bash
uv run -m exp_runner run <app> <baseline_exp> --plot
```

Analyze the results with a subagent and write a `## Observed Symptoms (<baseline_exp>)` section to the tracking markdown documenting:
- Goodput table for all policies across all RPS levels
- Which policy is currently best and by how much
- Any obviously broken behavior (e.g., a policy with near-zero goodput, excessive early returns)
- Initial hypotheses about root causes worth investigating

Only after this baseline is documented do you enter the iteration loop.

## The iteration loop

Repeat for the agreed number of iterations, or until the user says to stop, or the goal is clearly achieved:

### Step 1: Hypothesis

**You (the main agent) form the hypothesis.** Draw on the baseline results, the iteration history in the tracking markdown, and what has already been tried. Identifying the most promising improvement direction is your core judgment call — do not delegate it.

You may spawn a subagent to gather analytical depth that informs your judgment — for example: "dig into the early-return breakdown at 800 RPS and tell me what's driving it" or "compare goodput deltas across all RPS levels and flag any anomalies." But the hypothesis itself — what to change and why — is your synthesis.

Once you have formed the hypothesis and decided on the experiment design (see Step 3), write the full pre-experiment markdown section in one pass and append it to the tracking markdown:

```
## Iteration N: <short title> (experiment <exp_name>)

**Status:** Pending

### Change
<what code change is being made>

### Hypothesis
<why this should help; expected mechanism>

### Expected outcomes if hypothesis is correct:
1. ...
2. ...

### Experiment design
<what config you're using and why it best exposes whether the hypothesis is true or false>
```

Never delete or overwrite previous iterations in the tracking markdown.

### Step 2: Code edit and commit

**Use a subagent** (type: general-purpose) to:
- Make the code change described in the hypothesis
- If the change involves a policy or latency estimator, read the relevant source files first: grep for the feature flag name in `libs/tonic/tonic/src/masa/context/` and read `libs/masa-core/src/latency_estimator/` as needed
- Run `./scripts/check.sh` to verify no compilation errors
- Run `./scripts/format.sh` to format
- Do NOT stage experiment config directories (`exp/`) — those are not committed
- Make two commits in order:
  1. **Code commit**: code changes only. Message: `exp: <codeword> N — <description of change>`
  2. **Markdown commit**: `exp/docs/<CODEWORD>.md` only (the full pre-experiment section written in Step 1). Message: `exp: <codeword> N — document hypothesis`

Keeping them separate means you can `git revert` the code commit to undo the implementation without losing the written hypothesis and reasoning in the markdown.

Report back both commit hashes. Record the code commit hash in the tracking markdown iteration header.

### Step 3: Run experiment

The experiment design reasoning was already written to the tracking markdown in Step 1. Now create the config and run.

**Config design principles** (to guide Step 1 design and Step 3 execution):
- The baseline config is a regression anchor — you always want to be able to compare against it. But it may not be the most informative config for testing a specific hypothesis.
- If your change makes the system more responsive to load variation, design a schedule with dramatic load swings (e.g., high → low → high) rather than a monotonic ramp.
- If your change might have downsides in specific regimes (e.g., underload, cold start, bursty traffic), create a config that exercises those regimes to validate the concern.
- A single iteration may warrant multiple configs (e.g., one to test the upside, one to probe the downside risk). Name them `<codeword>_N` and `<codeword>_N_b`, and document why you're running both.

Copy and adapt the config:
```bash
cp -r exp/<app>/in/<ref_exp> exp/<app>/in/<new_exp>
# edit gen_config.json (load schedule), policies, or app config as needed
```

**Note:** Experiment config directories (`exp/<app>/in/<new_exp>/`) are NOT committed to git.

Then run:
```bash
uv run -m exp_runner run <app> <new_exp> --plot
```

This command runs for a long time (typically 5–15 minutes for a 7-RPS-step sweep). Wait for it to complete before proceeding. Do NOT run multiple experiments simultaneously — results are written to shared output directories.

**If the run fails:** Do not blindly retry. Diagnose first: run `./scripts/check.sh` to rule out a compilation error, and check container/process logs for infrastructure failures. Fix the root cause before re-running.

### Step 4: Analyze results

**Use a subagent** (type: general-purpose) to analyze the results:

```
Analyze task:
1. Read exp/<app>/plots/<new_exp>/goodput_ALL_aggregated.csv
2. Read exp/<app>/plots/<new_exp>/early_return_ALL_breakdown.csv (if it exists)
3. Read exp/<app>/plots/<new_exp>/cpu_summary.csv (if it exists)
4. Compare the target policy's goodput vs baseline policies at each RPS level
5. Identify if expected outcomes from the hypothesis were observed
6. Identify any new failure modes or surprising patterns
7. Return: per-RPS goodput table, key findings, recommendation (keep/revert/revise)
```

### Step 5: Document and decide

Append to the tracking markdown under the current iteration:

```
### Actual Outcomes (<exp_name>)

**Status:** [Complete ✅ | Regression ❌ | Mixed]

[Goodput table comparing target policy vs baselines at each RPS]

[Key findings — what worked, what didn't, why]

[Root cause analysis if hypothesis was not confirmed]
```

Then commit the markdown update (results + decision) on its own:
```
git add exp/docs/<CODEWORD>.md
git commit -m "exp: <codeword> N results — <one-line summary>"
```

Then decide:
- **Keep**: hypothesis confirmed, proceed to next iteration with this change in place
- **Revert**: regression, use `git revert <code-commit-hash>` to undo the code change only; the markdown commits (hypothesis + results) are preserved as a record
- **Revise**: partial success or wrong mechanism — refine the implementation without full revert

### Step 6: Check success criteria

After documenting the decision, evaluate whether the target policy now meets the success criteria (see "Success criteria" section below). If it does, surface a summary to the user:
- What the target policy is now achieving vs. each baseline, at each load point
- Which success criteria have been met
- A recommendation: declare success and stop, or continue to squeeze out further gains

Ask the user whether to continue iterating or stop. Do not silently start the next iteration if the goal has been reached.

## Why hypotheses must be tested sequentially

Each experiment run compiles and sweeps **all policies together** from a single source tree. This means:

- You cannot have two code changes in flight at the same time — the second change would contaminate the experiment testing the first.
- Do not stage or stash partial changes while an experiment is running. The experiment runner may recompile mid-sweep.
- If you want to test two independent hypotheses, test them one at a time. After getting results for hypothesis A, either commit it (keep) or revert it, then make the change for hypothesis B.

If multiple promising directions emerge from analysis, note them all in the tracking markdown under an `## Open hypotheses` section, then work through them one by one in subsequent iterations.

## Division of labor: you vs subagents

Your role as the main agent is to own the **arc of the optimization**: understanding what the experiments have proven or disproven, forming hypotheses about what to try next, deciding what experiment design will best test each hypothesis, and interpreting results in light of the full iteration history. This strategic layer requires continuity across iterations — and it fits in your context window precisely because you delegate everything else.

**You own hypothesis formation.** Do not ask a subagent to "identify the most promising improvement opportunity" or "draft the hypothesis." You synthesize that judgment yourself, drawing on the tracking markdown, the baseline results, and your understanding of the policy. You may ask subagents targeted analytical questions to inform your judgment (e.g., "break down early returns at 800 RPS and tell me which service is responsible") — but the hypothesis itself is yours.

**Data analysis always happens in subagents.** You may form a question ("why are early returns spiking at 800 RPS?") but you should not attempt to answer it yourself by reading files. Spawn a subagent with that question and the relevant context; let it do the reading, reasoning, and answering. The same applies to result interpretation after every experiment — do not read CSVs inline.

Delegate to subagents:
- **Data analysis**: reading and interpreting experiment CSVs, building comparison tables, identifying patterns, diagnosing failure modes
- **Implementation**: coming up with a concrete plan for a hypothesis, editing code, running `check.sh`/`format.sh`
- **Codebase exploration**: grep, reading source files across many locations
- **Experiment monitoring**: waiting on and summarizing experiment output

**How to write good subagent prompts for analysis:** Don't just ask for a summary — give the subagent the current hypothesis and the questions you're trying to answer, and ask it to raise and answer its own follow-up questions too. A good analysis subagent should not just report numbers; it should identify anomalies, propose explanations, flag concerns about the next iteration, and surface things you didn't think to ask.

**What subagents should return:** Ask subagents to return only information that belongs in the tracking markdown — the rule of thumb is "would a future reader of this markdown need to know this?" That includes: insights uncovered, bugs found, interesting data phenomena, regressions compared to a prior experiment, hypotheses confirmed or refuted, and open questions worth investigating. Raw data, intermediate calculations, and file contents that merely support a conclusion should stay in the subagent — don't pipe them back to the main agent. The return should be a tight, markdown-ready summary: a results table, key findings, any regressions vs. the baseline, open questions, and a recommendation for what to do next.

Doing detail work inline will overflow your context window well before the optimization converges. The tracking markdown is your persistent memory — subagents are your hands and eyes.

## Tracking markdown conventions

The tracking markdown is the single source of truth across sessions. Follow these conventions:
- One `## Iteration N:` section per hypothesis-experiment cycle
- Never erase previous iterations — they are important artifacts
- Document root causes, not just outcomes — future iterations may revisit
- If you revert a change, note why in the iteration section, not just in git

## Experiment naming convention

Experiments are named `<codeword>_<N>` (e.g., `falcon_1`, `falcon_2`). The codeword ties all runs in this optimization track together and matches the tracking markdown. Increment N for each new experiment in the series.

## Success criteria (default)

The optimization goal is to improve goodput (requests/sec meeting SLO) of the target policy, ideally beating the best baseline across the majority of RPS load points. A win at high-load points (near saturation) is worth more than at low-load points (where all well-functioning policies should converge to near-full throughput).

The policy is "beating the baseline" if it achieves higher goodput at a majority of load points where the system is actually stressed (typically ≥ 80% of offered load, e.g., ≥ 800 RPS if the system saturates around 1000 RPS).

If the user specifies a different goal, use that instead.
