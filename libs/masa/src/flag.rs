pub const PRIO_CLASS: bool = cfg!(feature = "prio_class");

pub const PRIO_GLOBAL: bool = cfg!(feature = "prio_global");

pub const PRIO_GLOBAL_EARLY: bool = cfg!(feature = "prio_global_early");

pub const PRIO_CLASS_GLOBAL: bool = cfg!(feature = "prio_class_global");

pub const PRIO_LOCAL: bool = cfg!(feature = "prio_local");

pub const PRIO_LOCAL_EARLY: bool = cfg!(feature = "prio_local_early");

pub const FIFO: bool = cfg!(feature = "fifo");

pub const FIFO_EARLY: bool = cfg!(feature = "fifo_early");

pub const FIFO_INFRA: bool = cfg!(feature = "fifo_infra");

pub const FIFO_SPAN_TRACING: bool = cfg!(feature = "fifo_span_tracing");

pub const FIFO_QUEUE_TRACING: bool = cfg!(feature = "fifo_queue_tracing");

pub const PRIO_GLOBAL_QUEUE_TRACING: bool = cfg!(feature = "prio_global_queue_tracing");
