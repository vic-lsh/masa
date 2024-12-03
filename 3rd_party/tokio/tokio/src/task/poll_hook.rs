use crate::runtime::task::{current_task_header, poll_hook::PollHook};

///
pub fn configure_child_task_poll_hook(hook: PollHook) -> bool {
    if let Some(header) = current_task_header() {
        // SAFETY: this function has exclusive access to the poll hook field.
        // This is because the poll hook is only used in the thread where the
        // task is running.
        unsafe { header.set_poll_hook(hook) };
        true
    } else {
        false
    }
}
