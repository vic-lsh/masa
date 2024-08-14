# README

Assert `Masa <= Fifo` when:

1. All processing latency is static as 16ms.
2. Keep one hop.
3. Let `Masa` enable `deadline`.

# Tiny Observations

1. Very interesting results. Even in one hop, tail difference still matches the processing latency as 16ms.
2. Try constant load generation.

# Tiny Sparks

1. Make first hop static and second hop exponential. Make first hop exponential and second hop static.
