# Synthetic Application

The synthetic application is meant to be a benchmark that is as simple as possible, so that we can better understand the basic behavior of deadline policies.

## Endpoints

### a

This endpoint makes two requests sequentially. The first request has a random latency, sampled using a normal distribution with mean `ChildRandomMean` us (from the app config file) and standard deviation `ChildRandomStd` us. The second request has a constant latency of `ChildConstantLatency` us.
