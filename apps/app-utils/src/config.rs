use structopt::StructOpt;
use masa::{QueueType, DeadlinePolicyType};

#[derive(StructOpt, Debug, Clone)]
pub struct PolicyArgs {
    #[structopt(long, default_value = "fifo")]
    pub queue: QueueType,

    #[structopt(long)]
    pub early_return: bool,

    #[structopt(long, default_value = "none")]
    pub deadline_policy: DeadlinePolicyType,
}
