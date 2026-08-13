//! Mixed push/pop throughput of MultiQueue vs `Mutex<BinaryHeap>`.
//!
//! Run with:
//! `cargo run -p multiqueue-prio-queue --example contention --release`

use std::collections::BinaryHeap;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

use multiqueue_prio_queue::MultiQueue;

const OPS_PER_THREAD: usize = 200_000;
const THREADS: &[usize] = &[1, 4, 8, 16, 32];

fn bench_mutex(threads: usize) -> f64 {
    let q = Arc::new(Mutex::new(BinaryHeap::new()));
    {
        let mut g = q.lock().unwrap();
        for i in 0..threads * 64 {
            g.push(i as u64);
        }
    }

    let start = Instant::now();
    let workers: Vec<_> = (0..threads)
        .map(|t| {
            let q = Arc::clone(&q);
            thread::spawn(move || {
                for i in 0..OPS_PER_THREAD {
                    if i % 2 == 0 {
                        q.lock().unwrap().push(((t as u64) << 32) | i as u64);
                    } else {
                        let _ = q.lock().unwrap().pop();
                    }
                }
            })
        })
        .collect();
    for w in workers {
        w.join().unwrap();
    }
    let secs = start.elapsed().as_secs_f64();
    (threads * OPS_PER_THREAD) as f64 / secs
}

fn bench_mq(threads: usize) -> f64 {
    let q = Arc::new(MultiQueue::with_threads(threads));
    for i in 0..threads * 64 {
        q.push(i as u64);
    }

    let start = Instant::now();
    let workers: Vec<_> = (0..threads)
        .map(|t| {
            let q = Arc::clone(&q);
            thread::spawn(move || {
                for i in 0..OPS_PER_THREAD {
                    if i % 2 == 0 {
                        q.push(((t as u64) << 32) | i as u64);
                    } else {
                        let _ = q.pop().or_else(|| q.pop_any());
                    }
                }
            })
        })
        .collect();
    for w in workers {
        w.join().unwrap();
    }
    let secs = start.elapsed().as_secs_f64();
    (threads * OPS_PER_THREAD) as f64 / secs
}

fn main() {
    println!(
        "{:>8} {:>14} {:>14} {:>8}",
        "threads", "mutex ops/s", "multiqueue ops/s", "speedup"
    );
    for &n in THREADS {
        let mutex = bench_mutex(n);
        let mq = bench_mq(n);
        println!("{n:>8} {mutex:>14.0} {mq:>14.0} {:>7.2}x", mq / mutex);
    }
}
