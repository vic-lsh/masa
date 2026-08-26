# Fully revealed continuation (`q = 0.5`)

Half of requests use API `a` and always stop after the 2 ms short tail. The other
half use API `b` and always execute the additional 23 ms tail. The aggregate mix
is therefore 50% short and 50% long, and every long path is exposed by the API
key (`q = 0.5`). This is the fully observable endpoint of the sweep.

The pilot uses one 220 RPS run, 15 seconds of warmup, 15 seconds of measurement,
matched-deadline bursts of eight, and a 50 ms SLO.
