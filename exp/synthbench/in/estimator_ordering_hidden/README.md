# Hidden continuation (`q = 0`)

All requests use API `a`, so the key visible when `shared::run` is issued cannot
distinguish the sampled continuation. Every request executes the 2 ms short tail;
independently, 50% also execute the 23 ms extra tail. Thus the expected request mix
is 50% short and 50% long, with none of the long paths revealed by the API key
(`q = 0`). The exact-continuation arm knows the pre-sampled optional step.

The pilot uses one 220 RPS run, 15 seconds of warmup, 15 seconds of measurement,
matched-deadline bursts of eight, and a 50 ms SLO.
