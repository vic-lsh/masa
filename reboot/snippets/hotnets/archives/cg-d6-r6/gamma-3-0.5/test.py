import numpy as np

for k in [0.125, 0.25, 0.5, 1]:
    mu = 3
    theta = mu / k

    np.random.seed(998244353)
    values = []
    for _ in range(0, 1000000):
        value = np.random.gamma(k, theta)
        values.append(value)

    print(f"k: {k}")
    print(f"mu: {mu}")

    values.sort()
    for pctl in [90, 99, 99.9]:
        idx = int(len(values) * pctl / 100)
        print(f"P{pctl}: {values[idx]:.3f}")

    print()
