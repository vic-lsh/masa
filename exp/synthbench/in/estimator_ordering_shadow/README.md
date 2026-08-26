# Shadow feasibility loads

This run reuses the fully revealed 50/50 short/long workload at 100 and 280 RPS.
Together with the 220 RPS visible audit run, it supplies low-load, knee, and overload
completed requests for an offline shadow test. A request is marked infeasible at the
shared-child issue point when time left is below either the learned or pre-sampled
remaining continuation; the request is still allowed to finish, so precision, miss
recall, and false-positive rate can be measured without admission or abort feedback.
