pub const PRIO_GLOBAL: bool = cfg!(feature = "prio_global");

pub const PRIO_LOCAL: bool = cfg!(feature = "prio_local");

pub const FIFO: bool = cfg!(feature = "fifo");

pub const FIFO_SPAN_TRACING: bool = cfg!(feature = "fifo_span_tracing");

pub const FIFO_QUEUE_TRACING: bool = cfg!(feature = "fifo_queue_tracing");

pub const PRIO_GLOBAL_QUEUE_TRACING: bool = cfg!(feature = "prio_global_queue_tracing");
