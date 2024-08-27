# README

Assert `Masa <= Fifo` when:

1. All processing latency is static as 8ms.
2. `Masa` uses normal `deadline`.

# Tiny Hypothesis

1. P99 difference is close to 8ms.

# Tiny Observations

1. Vary processing latency among 2ms, 4ms, 8ms.
2. Make first hop static and second hop exponential.
3. Make first hop exponential and second hop static.
