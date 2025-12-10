use crate::{
    body::BoxBody, masa::context::read_context, Code, GrpcMethod, Request, Response, Status,
};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, RwLock, atomic::{AtomicBool, AtomicU64, Ordering}
    },
    task::Poll,
    time::Instant,
};

use super::super::{ClientHooks, MasaHooks, ParentHooks, ServerHooks};
use masa::{Context, EARLY_RETURN, Estimator, MethodId, PercentileEstimator, time_now};

mod priority_model {
    use std::{sync::atomic::AtomicUsize, time::SystemTime};

    use super::*;

    const ACTION_TABLE: [usize; 3] = [0, 10000, 20000];
    const ALPHA: f64 = 0.1;

    #[derive(Debug)]
    pub struct QLearning {
        old_state: RwLock<State>,
        pub action: AtomicUsize,
        q_table: RwLock<[[f64; 3]; 2]>, // discretized state space
    }

    #[derive(Debug)]
    pub struct State {
        // e_rem estimate percentile
        pub downstream_latency: u64,
        // service goodput rate
        pub goodput_rate: f64,
    }

    #[derive(Debug)]
    pub struct Record {
        old_state: State,
        action: usize,
        reward: f64,
        next_state: State,
    }

    impl QLearning {
        pub fn new() -> Self {
            Self {
                old_state: RwLock::new(State {
                    downstream_latency: 0,
                    goodput_rate: 0.0,
                }),
                action: AtomicUsize::new(0),
                q_table: RwLock::new([[1.0; 3]; 2]),
            }
        }

        fn state_to_index(&self, state: &State) -> usize {
            if state.goodput_rate < 0.5 {
                0
            } else {
                1
            }
        }

        pub fn update(&self, reward: f64, action: usize, next_state: State) {
            let old_index = self.state_to_index(&self.old_state.read().unwrap());
            let next_index = self.state_to_index(&next_state);

            let mut q_table = self.q_table.write().unwrap();
            let old_q = q_table[old_index][action];
            let max_next_q = q_table[next_index]
                .iter()
                .cloned()
                .fold(f64::MIN, f64::max);

            let new_q = (1.0 - ALPHA) * old_q + ALPHA * (reward + max_next_q - old_q);
            q_table[old_index][action] = new_q;

            // Choose next action
            let (best_action, _) = q_table[next_index]
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
                .unwrap();
            self.action.store(best_action, Ordering::Relaxed);            
        }

        pub fn assign_priority(&self, deadline: u64) -> u64 {
            let instant = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_micros() as u64;
            let remaining_time = deadline.saturating_sub(instant);
            let action = ACTION_TABLE[self.action.load(Ordering::Relaxed)] as u64;
            if remaining_time < action {
                deadline + 1000
            } else {
                deadline
            }
        }
    }
}

use priority_model::*;

use priority_model::QLearning;

#[derive(Debug)]
/// This policy computes the deadline d of a child request as  
///   d = d_p - e_rem
/// where d_p is the deadline of the parent request and e_rem is an estimate for the remaining time
/// left in the request after this child request executes. e_rem is estimated by sampling from the
/// distribution of observed values for e_rem.
#[allow(dead_code)]
#[allow(unreachable_pub)]
pub struct LocalDeadlineLearned;

impl MasaHooks for LocalDeadlineLearned {
    type ServerContext = ServerContext;
    type ChildContext = ChildContext;
    type ParentContext = ParentContext;
}

#[derive(Debug)]
#[allow(dead_code)]
#[allow(unreachable_pub)]
pub struct ServerContext {
    child_latency_estimator: RwLock<HashMap<String, PercentileEstimator>>,
    /// Service goodput
    goodputs: AtomicU64,
    /// Number of early returns issued by this service
    served_requests: AtomicU64,
    /// RL Model
    model: QLearning,
}


impl ServerHooks for ServerContext {
    fn new(_service_name: &'static str) -> Self {
        Self {
            child_latency_estimator: RwLock::new(HashMap::new()),
            goodputs: AtomicU64::new(0),
            served_requests: AtomicU64::new(0),
            model: QLearning::new(),
        }
    }
}

impl ServerContext {
    pub fn goodput_rate(&self) -> f64 {
        let served = self.served_requests.load(Ordering::Relaxed) as f64;
        if served == 0.0 {
            0.0
        } else {
            let goodputs = self.goodputs.load(Ordering::Relaxed) as f64;
            goodputs / served
        }
    }
}

impl ServerContext {
    pub fn goodputs(&self) -> u64 {
        self.goodputs.load(Ordering::Relaxed)
    }
}

#[derive(Debug)]
#[allow(dead_code)]
#[allow(unreachable_pub)]
pub struct ParentContext {
    method: GrpcMethod,
    ctx: Context,
    server: Arc<ServerContext>,

    will_early_return: AtomicBool,
    // TODO: Can these be replaced by a Cell?
    child_end_times: Mutex<Vec<(MethodId, Instant)>>,
}

impl ParentContext {
    #[inline]
    fn check_early_return(&self) -> bool {
        if EARLY_RETURN {
            self.check_early_return_impl()
        } else {
            false
        }
    }

    fn check_early_return_impl(&self) -> bool {
        if self.will_early_return.load(Ordering::Relaxed) {
            return true;
        }

        let now = time_now();
        let should_early_return = now >= self.ctx.deadline();

        if should_early_return {
            if self
                .will_early_return
                .compare_exchange_weak(false, true, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {}
        }

        should_early_return
    }

    #[inline]
    fn issue_early_return(&self) -> Status {
        Status::new(Code::DeadlineExceeded, format!("/EarlyReturn"))
    }
}

impl ParentHooks<ChildContext, ServerContext> for ParentContext {
    fn begin<B>(
        method: GrpcMethod,
        req: &http::Request<B>,
        server_ctx: Arc<ServerContext>,
    ) -> Self {
        Self {
            method,
            ctx: read_context(req),
            server: server_ctx,
            will_early_return: AtomicBool::new(false),
            child_end_times: Mutex::new(Vec::new()),
        }
    }

    fn before_poll<Ret>(&self) -> Result<(), Result<Response<Ret>, Status>> {
        if self.check_early_return() {
            return Err(Err(self.issue_early_return()));
        }

        Ok(())
    }

    fn after_poll<Ret>(
        &self,
        poll: &Poll<Result<Response<Ret>, Status>>,
    ) -> Result<(), Result<Response<Ret>, Status>> {
        if let Poll::Pending = poll {
            if self.check_early_return() {
                return Err(Err(self.issue_early_return()));
            }
        }

        Ok(())
    }

    fn before_child_rpc<T>(
        &self,
        child_method: GrpcMethod,
        request: &mut Request<T>,
        _child_ctx: &mut ChildContext,
    ) -> Result<(), Status> {
        if self.check_early_return() {
            return Err(self.issue_early_return());
        }

        // NOTE: if we don't have enough data to estimate the duration of the parent or child
        // request, we set child deadline = parent deadline
        // NOTE: we need to include the parent method in the key, because the duration until the
        // end of the parent request after this child request completes will vary for different
        // parent methods (i.e. endpoints on this server)
        let estimate_remaining = self.server.child_latency_estimator.read().unwrap().get(&format!(
            "{}/{}",
            self.method.id(),
            child_method.id()
        )).map(|e| e.estimate(50)).flatten().unwrap_or(0);

        let state = State {
            downstream_latency: estimate_remaining,
            goodput_rate: self.server.goodput_rate(),
        };

        // NOTE(vic): could we have passed the deadline at this point?
        let deadline = self.ctx.deadline() - estimate_remaining;

        let deadline = self.server.model.assign_priority(deadline);

        let child_recv_ctx = Context::new(
            self.ctx.api().clone(),
            self.ctx.request_id(),
            self.ctx.slo(),
            self.ctx.start_at(),
            deadline,
        );
        request.metadata_mut().insert_ctx("ctx", &child_recv_ctx);

        Ok(())
    }

    fn after_child_rpc<T>(
        &self,
        child_method: GrpcMethod,
        response: &mut Result<Response<T>, Status>,
        _child_ctx: ChildContext,
    ) -> Result<(), Status> {
        // TODO(yingbo): If child early returns, should it be counted as goodput?
        if let Err(status) = response {
            // NOTE(vic): could we avoid cloning here?
            return Err(status.clone());
        }

        self.child_end_times
            .lock()
            .unwrap()
            .push((child_method.id(), Instant::now()));

        Ok(())
    }

    fn finalize(&self, _response: &mut http::Response<BoxBody>) {
        // Update server context information.
        let parent_end = Instant::now();
        let mut m = self.server.child_latency_estimator.write().unwrap();
        // track remaining time for each child
        for (child_method, child_end) in self.child_end_times.lock().unwrap().iter() {
            // TODO: The LatencyDistribution instances will regularly sort their data. Should this
            // work be done asynchronously?
            let k = format!("{}/{}", self.method.id(), child_method);
            if let Some(estimator) = m.get_mut(&k) {
                let remaining = parent_end
                    .saturating_duration_since(*child_end)
                    .as_micros() as u64;
                estimator.update(remaining);
            } else {
                let mut estimator = PercentileEstimator::default();
                let remaining = parent_end
                    .saturating_duration_since(*child_end)
                    .as_micros() as u64;
                estimator.update(remaining);
                m.insert(k,estimator);
            }
        }

        // Update goodput and served request count
        if !self.will_early_return.load(Ordering::Relaxed) {
            self.server
                .goodputs
                .fetch_add(1, Ordering::Relaxed);
        }
        let old = self.server.served_requests.load(Ordering::Relaxed);
        self.server.served_requests.fetch_add(1, Ordering::Relaxed);

        // Update RL model every 100 requests
        if old % 100 == 0 && old > 0 {
            let goodput_rate = self.server.goodput_rate();
            let next_state = State {
                downstream_latency: 0, // Not used in goodput rate
                goodput_rate,
            };
            let reward = goodput_rate; // Reward is the goodput rate
            let action = self.server.model.action.load(Ordering::Relaxed);
            self.server.model.update(reward, action, next_state);
        }
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
#[allow(unreachable_pub)]
pub struct ChildContext {}

impl ClientHooks for ChildContext {
    fn new<T>(_method: GrpcMethod, _request: &Request<T>) -> Self {
        Self {}
    }
}
