pub const PRIO_CLASS: bool = if cfg!(feature = "prio_class") {
    true
} else {
    false
};

pub const PRIO_GLOBAL: bool = if cfg!(feature = "prio_global") {
    true
} else {
    false
};

pub const PRIO_GLOBAL_EARLY: bool = if cfg!(feature = "prio_global_early") {
    true
} else {
    false
};

pub const PRIO_CLASS_GLOBAL: bool = if cfg!(feature = "prio_class_global") {
    true
} else {
    false
};

pub const PRIO_LOCAL: bool = if cfg!(feature = "prio_local") {
    true
} else {
    false
};

pub const PRIO_LOCAL_EARLY: bool = if cfg!(feature = "prio_local_early") {
    true
} else {
    false
};

pub const FIFO: bool = if cfg!(feature = "fifo") { true } else { false };

pub const FIFO_INFRA: bool = if cfg!(feature = "fifo_infra") {
    true
} else {
    false
};
