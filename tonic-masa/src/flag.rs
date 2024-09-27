pub const PRIO_LOCAL: bool = if cfg!(feature = "prio_local") {
    true
} else {
    false
};

pub const ONLINE_TRACKER: bool = true;
