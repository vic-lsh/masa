pub const QUEUE_EDF: bool = if cfg!(feature = "queue_edf") {
    true
} else {
    false
};

pub const EST_ONLINE: bool = if cfg!(feature = "est_online") {
    true
} else {
    false
};
