# pyrefly: ignore  # import-error
import goodput
# pyrefly: ignore  # import-error
import latency
# pyrefly: ignore  # import-error
from util import parse_args

if __name__ == "__main__":
    args = parse_args()
    goodput.generate_plots(args)
    latency.generate_plots(args)
