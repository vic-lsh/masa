import argparse
import json
import os
from typing import *


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--config", type=str)
    parser.add_argument("--input-path", type=str)
    parser.add_argument("--output-path", type=str)
    args = parser.parse_args()
    return args


args = parse_args()
cfg = json.load(open(args.config))
os.makedirs(args.output_path, exist_ok=True)


def replace_placeholders(content: str, replacements: Dict[str, str]) -> str:
    for key, value in replacements.items():
        content = content.replace(f"${{{key}}}", value)
    return content


input_files = [f for f in os.listdir(args.input_path) if f.endswith(".yaml")]

for input_file in input_files:
    with open(os.path.join(args.input_path, input_file), "r") as f:
        content = f.read()

    new_content = replace_placeholders(content, cfg)

    output_file = os.path.join(args.output_path, input_file)
    with open(output_file, "w") as f:
        f.write(new_content)
