use crossbeam_channel::{bounded, unbounded, Receiver, Sender, TryRecvError};
use futures_lite::future;
use hyper::rt::{Exec, Executor};
use rand_distr::{Distribution, Normal};
use std::sync::Arc;
use std::time::{Duration, Instant};
use std::time::{SystemTime, UNIX_EPOCH};
use structopt::StructOpt;
use tonic_deadline::DeadlineHint;

#[derive(StructOpt, Debug, Clone)]
#[structopt(about = "Server for benchmarking")]
pub struct Args {}

#[derive(Debug)]
pub struct Request {
    send_at: u64,
    finish_at: u64,
    prev_elapse: u64,
    total_elapse: u64,
    hint: u64,
}

pub fn time_now() -> u64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_micros();
    now as u64
}

fn busy_spin(duration: Duration) {
    let now = Instant::now();
    while now.elapsed() < duration {}
}

#[derive(Debug)]
struct ExecImpl<'a> {
    ex: Arc<smol::Executor<'a>>,
}

impl<'a> ExecImpl<'a> {
    fn new() -> Self {
        Self {
            ex: Arc::new(smol::Executor::new()),
        }
    }

    async fn run(&self) {
        loop {
            self.ex.tick().await;
            future::yield_now().await;
        }
    }
}

impl<'a, F> Executor<F> for ExecImpl<'a>
where
    F: std::future::Future + Send + 'static,
    F::Output: Send,
{
    fn execute(&self, fut: F, ddl: DeadlineHint) {
        self.ex.spawn_with_ddl(fut, ddl).fallible().detach();
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::from_args();

    let exs = [Arc::new(ExecImpl::new()), Arc::new(ExecImpl::new())];

    for ex in &exs {
        let ex = ex.clone();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(ex.run());
        });
    }

    let (tx, rx) = unbounded();

    let ex = exs[0].clone();
    let client = tokio::spawn(async move {
        for i in 0..10 {
            let tx = tx.clone();
            let future = async move {
                busy_spin(Duration::from_millis(100));
                let req = Request {
                    send_at: time_now(),
                    finish_at: 0,
                    prev_elapse: 0,
                    total_elapse: 0,
                    hint: i,
                };
                tx.send(req).unwrap();
            };
            ex.execute(future, DeadlineHint::new(1));
        }
    });

    let ex = exs[1].clone();
    let server = tokio::spawn(async move {
        while let Ok(req) = rx.recv() {
            let hint = req.hint;
            ex.execute(
                async move {
                    busy_spin(Duration::from_millis(1000));
                    println!("server: {:?}", req);
                },
                DeadlineHint::new(hint),
            );
        }
    });

    client.await?;

    tokio::time::sleep(Duration::from_secs(30)).await;

    server.await?;

    Ok(())
}
