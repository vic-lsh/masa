import argparse
import json
import os
from pathlib import Path

import pandas as pd

data_dir = None
output_dir = None


def read_data(config_dir, data_dir):
    with open(os.path.join(config_dir, "gen_config.json")) as f:
        config = json.load(f)
    rps_values = config["Rps"]
    apis = config["Apis"]
    policies = os.listdir(data_dir)
    policies = list(
        filter(lambda p: os.path.isdir(os.path.join(data_dir, p)), policies)
    )
    results = {}
    for api in apis + ["ALL"]:
        results[api] = {policy: {} for policy in policies}
    for policy in policies:
        policy_folder = os.path.join(data_dir, policy)

        # process each CSV file
        for rps in rps_values:
            file_path = os.path.join(policy_folder, f"r{rps}.csv")
            df = pd.read_csv(file_path)
            for api in apis:
                results[api][policy][rps] = df[df["api"] == api].copy()
            results["ALL"][policy][rps] = df

    apis.append("ALL")

    return apis, policies, rps_values, results


def parse_args():
    global data_dir, output_dir
    parser = argparse.ArgumentParser()
    parser.add_argument("--config-dir", type=Path, required=True)
    parser.add_argument("--data-dir", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    args = parser.parse_args()

    os.makedirs(args.output_dir, exist_ok=True)

    return args
