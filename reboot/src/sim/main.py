import argparse
import copy
import heapq
from dataclasses import dataclass
from typing import *

import numpy as np


@dataclass
class GlobalRequest:
    s1_send_at: float
    s2_lat: float
    s2_send_at: float
    s3_lat: float
    s3_finish_at: float

    def __lt__(self, other):
        return self.s1_send_at < other.s1_send_at


# @dataclass
# class LocalRequest:
#     send_at: float
#     finish_at: float


@dataclass
class Options:
    rps: int
    secs: int
    output1: str
    output2: str


def parse_opts() -> Options:
    parser = argparse.ArgumentParser()
    parser.add_argument("--rps", type=int, required=True)
    parser.add_argument("--secs", type=int, required=True)
    parser.add_argument("--output1", type=str, required=True)
    parser.add_argument("--output2", type=str, required=True)
    args = parser.parse_args()
    return Options(
        rps=args.rps, secs=args.secs, output1=args.output1, output2=args.output2
    )


MU = 2 * 1e-3  # 2ms
SIGMA = 0.5 * 1e-3  # 0.5ms


def load_gen(opts: Options) -> List[GlobalRequest]:
    rng0 = np.random.default_rng(seed=1136)
    rng1 = np.random.default_rng(seed=1422)
    rng2 = np.random.default_rng(seed=1701)

    rate = opts.rps
    duration = opts.secs
    elapse = 0.0

    queue: List[GlobalRequest] = []

    # s: s1 -> s2 -> s3
    #    s1_send_at (s_ddl)
    #          s2_send_at
    #                s3_finish_at

    while elapse < duration:
        value0 = rng0.exponential(1 / rate)
        # value1 = max(rng1.normal(MU, SIGMA), 0)
        # value2 = max(rng2.normal(MU, SIGMA), 0)
        value1 = MU
        value2 = MU

        elapse += value0

        req = GlobalRequest(
            s1_send_at=elapse,
            s2_lat=value1,
            s2_send_at=0,
            s3_lat=value2,
            s3_finish_at=0,
        )
        queue.append(req)

    return queue


def fcfs(queue: List[GlobalRequest], output: str):
    queue = copy.deepcopy(queue)

    queue = sorted(queue, key=lambda x: x.s1_send_at)
    elapse = 0.0
    for req in queue:
        elapse = max(elapse, req.s1_send_at)
        elapse += req.s2_lat
        req.s2_send_at = elapse

    queue = sorted(queue, key=lambda x: x.s2_send_at)
    elapse = 0.0
    for req in queue:
        elapse = max(elapse, req.s2_send_at)
        elapse += req.s3_lat
        req.s3_finish_at = elapse

    queue = sorted(queue, key=lambda x: x.s1_send_at)
    with open(output, "w") as f:
        for req in queue:
            f.write(f"{req}\n")
            # latency = req.s3_finish_at - req.s1_send_at
            # latency = round(latency * 1e6)
            # f.write(f"{latency}\n")


def masa(queue: List[GlobalRequest], output: str):
    queue = copy.deepcopy(queue)
    next_queue: List[GlobalRequest] = []
    priority_queue: List[GlobalRequest] = []

    queue = sorted(queue, key=lambda x: x.s1_send_at)
    elapse = 0.0
    for req in queue:
        elapse = max(elapse, req.s1_send_at)
        elapse += req.s2_lat
        req.s2_send_at = elapse

    queue = sorted(queue, key=lambda x: x.s2_send_at)
    elapse = 0.0
    while True:
        if not queue and not priority_queue:
            break
        while queue and queue[0].s2_send_at <= elapse:
            req = queue.pop(0)
            heapq.heappush(priority_queue, req)
        if not priority_queue:
            req = queue.pop(0)
            heapq.heappush(priority_queue, req)
            elapse = max(elapse, req.s2_send_at)
            continue
        if priority_queue:
            req = heapq.heappop(priority_queue)
            assert req.s2_send_at <= elapse
            elapse += req.s3_lat
            req.s3_finish_at = elapse
            next_queue.append(req)

    queue = next_queue
    queue = sorted(queue, key=lambda x: x.s1_send_at)
    with open(output, "w") as f:
        for req in queue:
            f.write(f"{req}\n")
            # latency = req.s3_finish_at - req.s1_send_at
            # latency = round(latency * 1e6)
            # f.write(f"{latency}\n")


def main():
    opts = parse_opts()
    queue = load_gen(opts)
    fcfs(queue, opts.output1)
    masa(queue, opts.output2)


if __name__ == "__main__":
    main()
