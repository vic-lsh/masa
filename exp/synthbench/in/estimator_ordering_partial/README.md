# Partially revealed continuation (`q = 0.25`)

API `b` is always long and carries 25% of offered requests. API `a` carries the
other 75%, always executes the 2 ms short tail, and executes the 23 ms extra tail
with probability 1/3. Consequently, 25% of all requests are visibly long, another
25% are hidden long requests under key `a`, and 50% are short. The aggregate mix
remains exactly 50% short and 50% long in expectation while half of the long paths
are exposed by the API key (`q = 0.25`).

The pilot uses one 220 RPS run, 15 seconds of warmup, 15 seconds of measurement,
matched-deadline bursts of eight, and a 50 ms SLO.
