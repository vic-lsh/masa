# README

Expect to assert `Masa = Fifo` but failed when:

1. All processing latency is static as 8ms.
2. `Masa` uses an equal `deadline` as `1`.

# Tiny Observations

1. gRPC also has a latency around 2ms. I should change processing latency to verify the impacts of gRPC.
