import argparse
import heapq
from dataclasses import dataclass
from typing import *

import numpy as np


class Distribution:
    def sample(self) -> float:
        raise NotImplementedError


class NormalDistribution(Distribution):
    def __init__(self, mu: float, sigma: float):
        self.mu = mu
        self.sigma = sigma
        self.rng = np.random.default_rng(seed=998244353)

    def sample(self) -> float:
        return max(0, self.rng.normal(self.mu, self.sigma))


class ConstantDistribution(Distribution):
    def __init__(self, rate: float):
        self.rate = rate

    def sample(self) -> float:
        return 1 / self.rate


class ExponentialDistribution(Distribution):
    def __init__(self, rate: float):
        self.rate = rate
        self.rng = np.random.default_rng(seed=998244353)

    def sample(self) -> float:
        return self.rng.exponential(1 / self.rate)


class HistoryDistribution(Distribution):
    def __init__(self, history: List[float]):
        self.history = history
        self.rng = np.random.default_rng(seed=998244353)

    def sample(self) -> float:
        index = self.rng.integers(0, len(self.history))
        value = self.history[index]
        return value


@dataclass
class Options:
    mode: str
    rps: int
    secs: int
    output: str


def parse_opts() -> Options:
    parser = argparse.ArgumentParser()
    parser.add_argument("--mode", type=str, required=True, choices=["fcfs", "masa"])
    parser.add_argument("--rps", type=int, required=True)
    parser.add_argument("--secs", type=int, required=True)
    parser.add_argument("--output", type=str, required=True)
    args = parser.parse_args()
    return Options(
        mode=args.mode,
        rps=args.rps,
        secs=args.secs,
        output=args.output,
    )


@dataclass
class Request:
    send_at: float
    finish_at: float
    prev_elapse: float
    total_elapse: float

    def __post_init__(self):
        self.hint = self.send_at - self.prev_elapse

    def __lt__(self, other):
        return self.hint < other.hint


ELAPSE_MU = 20 * 1e-3
ELAPSE_SIGMA = 5 * 1e-3
EXEC_MU = 2 * 1e-3


def sim_fcfs(opts: Options):
    exp = ExponentialDistribution(opts.rps)
    # cst = ConstantDistribution(opts.rps)
    normal = NormalDistribution(ELAPSE_MU, ELAPSE_SIGMA)

    send_at = 0.0
    exec_at = 0.0
    queue: List[Request] = []
    while send_at < opts.secs:
        send_at += exp.sample()
        # send_at += cst.sample()
        req = Request(
            send_at=send_at,
            finish_at=0,
            prev_elapse=normal.sample(),
            total_elapse=0,
        )
        queue.append(req)

    for req in queue:
        exec_at = max(exec_at, req.send_at)
        exec_at += EXEC_MU
        req.finish_at = exec_at
        req.total_elapse = req.finish_at - req.send_at + req.prev_elapse

    queue = sorted(queue, key=lambda req: req.send_at)
    with open(opts.output, "w") as f:
        for req in queue:
            # f.write(f"{req}\n")
            total_elapse = round(req.total_elapse * 1e6)
            f.write(f"{total_elapse}\n")

    # [TODO] Output as a distribution.


def sim_masa(opts: Options):
    exp = ExponentialDistribution(opts.rps)
    # cst = ConstantDistribution(opts.rps)
    normal = NormalDistribution(ELAPSE_MU, ELAPSE_SIGMA)

    send_at = 0.0
    exec_at = 0.0
    queue: List[Request] = []
    while send_at < opts.secs:
        send_at += exp.sample()
        # send_at += cst.sample()
        req = Request(
            send_at=send_at,
            finish_at=0,
            prev_elapse=normal.sample(),
            total_elapse=0,
        )
        queue.append(req)

    exec_queue: List[Request] = []
    done_queue: List[Request] = []
    while True:
        if not queue and not exec_queue:
            break
        while queue and queue[0].send_at <= exec_at:
            req = queue.pop(0)
            heapq.heappush(exec_queue, req)
        if not exec_queue:
            req = queue.pop(0)
            heapq.heappush(exec_queue, req)
        if exec_queue:
            req = heapq.heappop(exec_queue)
            exec_at = max(exec_at, req.send_at)
            exec_at += EXEC_MU
            req.finish_at = exec_at
            req.total_elapse = req.finish_at - req.send_at + req.prev_elapse
            done_queue.append(req)

    queue = sorted(done_queue, key=lambda req: req.send_at)
    with open(opts.output, "w") as f:
        for req in queue:
            # f.write(f"{req}\n")
            total_elapse = round(req.total_elapse * 1e6)
            f.write(f"{total_elapse}\n")

    # [TODO] Output as a distribution.


def main():
    opts = parse_opts()
    if opts.mode == "fcfs":
        sim_fcfs(opts)
    elif opts.mode == "masa":
        sim_masa(opts)


if __name__ == "__main__":
    main()
