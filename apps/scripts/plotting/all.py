import goodput
import latency
from util import parse_args

if __name__ == "__main__":
    args = parse_args()
    goodput.generate_plots(args)
    latency.generate_plots(args)
