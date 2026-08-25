# Deadline and Slack Estimators

This synthetic ablation tests why hard RPC deadlines and soft scheduling slack
use different remaining-work estimates. Three short sequential hops lead to a
lognormal final hop whose p95 service time is six times its p50.

The authoritative runner configuration is
[`exp/synthbench/in/deadline_slack_estimators`](../../../exp/synthbench/in/deadline_slack_estimators/).
It compares default Masa, whose hard deadline uses a conservative estimate,
with `deadline_equals_slack`, which tightens the hard deadline using the same
variance-aware estimate as scheduling priority. The sweep runs at 100, 200,
400, and 600 RPS with a 100 ms SLO.

```bash
./eval/ablations/deadline_slack_estimators/run.sh --k8s --plot
```

For deployment validation on local Kubernetes:

```bash
./eval/ablations/deadline_slack_estimators/run.sh --kind
```
