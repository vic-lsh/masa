# README

Assert `Masa = Fifo` when:

1. All processing latency is static as 2ms.
2. `Masa` uses normal `deadline`.

# Tiny Observations

1. Vary processing latency among 2ms, 4ms, 8ms.
2. Make first hop static and second hop exponential.
3. Make first hop exponential and second hop static.
