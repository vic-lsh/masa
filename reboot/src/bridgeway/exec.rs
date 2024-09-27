use futures_lite::future;
use hyper::rt::Executor;
use std::sync::Arc;
use tonic_masa::PriorityHint;

#[derive(Debug)]
pub struct ExecImpl<'a> {
    ex: Arc<smol::Executor<'a>>,
}

impl<'a> ExecImpl<'a> {
    pub fn new(ex: Arc<smol::Executor<'a>>) -> Self {
        Self { ex }
    }

    pub async fn run(&self) {
        // [NOTE] Only a global queue is used in smol::Executor::tick().
        loop {
            self.ex.tick().await;
            // [TODO] Change to tokio yield.
            // [NOTE] Yield to tokio runtime.
            future::yield_now().await;
        }
    }
}

impl<'a, F> Executor<F> for ExecImpl<'a>
where
    F: std::future::Future + Send + 'static,
    F::Output: Send,
{
    fn execute(&self, fut: F, ddl: PriorityHint) {
        // [NOTE] Deadline is passed from H2Stream.
        self.ex.spawn_with_ddl(fut, ddl).fallible().detach();
    }
}
