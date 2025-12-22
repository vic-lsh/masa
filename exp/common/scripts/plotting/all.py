# pyrefly: ignore  # import-error
import goodput
# pyrefly: ignore  # import-error
import latency
# pyrefly: ignore  # import-error
from util import parse_args
import os

if __name__ == "__main__":
    args = parse_args()
    
    # Remove existing plot files before generating new ones
    if args.output_dir.exists():
        for png_file in args.output_dir.rglob("*.png"):
            try:
                os.remove(png_file)
            except OSError as e:
                print(f"Warning: Could not remove {png_file}: {e}")
    
    goodput.generate_plots(args)
    latency.generate_plots(args)
