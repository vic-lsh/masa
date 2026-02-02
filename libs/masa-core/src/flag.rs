pub const PRIO_GLOBAL: bool = cfg!(feature = "prio_global");

pub const PRIO_OLDEST: bool = cfg!(feature = "prio_oldest");

pub const PRIO_LOCAL: bool = cfg!(feature = "prio_local");

pub const FIFO: bool = cfg!(feature = "fifo");

pub const EARLY_RETURN: bool = cfg!(feature = "early");

#[cfg(any(
    all(feature = "fifo", feature = "prio_global"),
    all(feature = "fifo", feature = "prio_oldest"),
    all(feature = "fifo", feature = "prio_local"),
    all(feature = "prio_global", feature = "prio_oldest"),
    all(feature = "prio_global", feature = "prio_local"),
    all(feature = "prio_oldest", feature = "prio_local"),
))]
compile_error!("Enable at most one policy feature: fifo | prio_global | prio_oldest | prio_local");
