// Tonic hooks for any module stack.
//
// `PolicyHooks<S>` is the single concrete `Hooks` implementation. It owns
// request plumbing (splitting the inbound wire sections, resolving method
// names, installing the child's and the response's wire sections, remembering
// which module ended a request) and delegates every policy decision to the
// module stack `S`: what a request carries, what a child request carries, and
// what a response carries are all whatever the modules put. The stack applies
// the ordering rules (see `rpcstack`'s `module.rs`).

use std::marker::PhantomData;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::Poll;

use crate::metadata::{
    get_method_name_override_from_headers, get_method_name_override_from_metadata,
    get_service_name_override_from_headers, get_service_name_override_from_metadata,
};
use rpcstack::{
    build_server, ChildOutcome, ChildState, Early, Extensions, MissingDependency, ModuleStack,
    Outcome, WireIn, WireOut,
};
use tonic::masa::{ClientHooks, Hooks, ParentHooks, ServerHooks};
use tonic::{CowGrpcMethod, GrpcMethod, Request, Response, Status};

/// `Hooks` implementation that runs the policy module stack `S`.
pub struct PolicyHooks<S>(PhantomData<fn() -> S>);

impl<S> std::fmt::Debug for PolicyHooks<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PolicyHooks")
    }
}

#[inline]
fn resolve_method_name_impl(
    method: GrpcMethod,
    method_override: Option<&str>,
    service_override: Option<&str>,
) -> CowGrpcMethod {
    if let Some(method_name) = method_override {
        if let Some(service_name) = service_override {
            return CowGrpcMethod::new(service_name.to_string(), method_name.to_string());
        }
        return CowGrpcMethod::new(method.service(), method_name.to_string());
    }
    CowGrpcMethod::new(method.service(), method.method())
}

fn resolve_method_name_from_http<B>(method: GrpcMethod, req: &http::Request<B>) -> CowGrpcMethod {
    resolve_method_name_impl(
        method,
        get_method_name_override_from_headers(req.headers()),
        get_service_name_override_from_headers(req.headers()),
    )
}

fn resolve_method_name_from_request<T>(method: GrpcMethod, request: &Request<T>) -> CowGrpcMethod {
    resolve_method_name_impl(
        method,
        get_method_name_override_from_metadata(request.metadata()),
        get_service_name_override_from_metadata(request.metadata()),
    )
}

impl<S: ModuleStack> Hooks for PolicyHooks<S> {
    type ServerContext = ServerContext<S>;
    type ChildContext = ChildContext<S>;
    type ParentContext = ParentContext<S>;
}

#[derive(Debug)]
pub struct ServerContext<S: ModuleStack> {
    modules: S::Server,
}

impl<S: ModuleStack> ServerContext<S> {
    /// Build the server state of the stack `S`, or report the first module
    /// whose required module or server state the stack does not provide before
    /// it. [`ServerHooks::new`] panics with the same message.
    ///
    /// Panics if two modules share a [`rpcstack::Module::NAME`].
    pub fn try_new(service_name: &'static str) -> Result<Self, MissingDependency> {
        Ok(Self {
            modules: build_server::<S>(service_name)?,
        })
    }
}

impl<S: ModuleStack> ServerHooks for ServerContext<S> {
    fn new(service_name: &'static str) -> Self {
        Self::try_new(service_name).unwrap_or_else(|err| panic!("{err}"))
    }
}

#[derive(Debug)]
pub struct ParentContext<S: ModuleStack> {
    modules: S,
    /// Hooks take `&self`, so the per-request state sits behind a lock;
    /// hooks run one at a time, so it is never contended. The lock is held
    /// for a whole stack call, so each child RPC is set up atomically.
    state: Mutex<RequestState>,
}

/// What the framework keeps for one request besides the modules' own state.
#[derive(Debug, Default)]
struct RequestState {
    ext: Extensions,
    /// The module that ended the request, and the status it gave, if it gave
    /// a status rather than a response. Reported to every module in `finalize`.
    ended_by: Option<(&'static str, Option<Status>)>,
}

impl RequestState {
    fn record<Ret>(&mut self, early: &Early<Ret>) {
        let status = early.reply.as_ref().err().cloned();
        self.ended_by.get_or_insert((early.by, status));
    }

    fn outcome(&self) -> Outcome<'_> {
        match &self.ended_by {
            None => Outcome::Handled,
            Some((by, Some(status))) => Outcome::Rejected { by, status },
            Some((by, None)) => Outcome::Replied { by },
        }
    }
}

impl<S: ModuleStack> ParentContext<S> {
    fn state(&self) -> MutexGuard<'_, RequestState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl<S: ModuleStack> ParentHooks<ChildContext<S>, ServerContext<S>> for ParentContext<S> {
    fn begin<B>(
        method: GrpcMethod,
        req: &http::Request<B>,
        server_ctx: Arc<ServerContext<S>>,
    ) -> Self {
        let resolved_method = resolve_method_name_from_http(method, req);
        let wire = WireIn::from_headers(req.headers()).unwrap_or_else(|err| panic!("{err}"));
        let mut ext = Extensions::new();
        let modules = S::new(&resolved_method, &server_ctx.modules, &wire, &mut ext);

        Self {
            modules,
            state: Mutex::new(RequestState {
                ext,
                ended_by: None,
            }),
        }
    }

    #[inline]
    fn before_poll<Ret>(&self) -> Result<(), Result<Response<Ret>, Status>> {
        // Debug builds walk the stack anyway, to check its `POLL_HOOKS`
        // declarations.
        if !S::POLL_HOOKS && !cfg!(debug_assertions) {
            return Ok(());
        }
        let mut state = self.state();
        match self.modules.before_poll(&mut state.ext) {
            Ok(()) => Ok(()),
            Err(early) => {
                state.record(&early);
                Err(early.reply)
            }
        }
    }

    fn before_child_rpc<T>(
        &self,
        child_method: GrpcMethod,
        request: &mut Request<T>,
        child_ctx: &mut ChildContext<S>,
    ) -> Result<(), Status> {
        let child_method_name = resolve_method_name_from_request(child_method, request);
        child_ctx.set_method_name(child_method_name.clone());

        let mut child_wire = WireOut::new();
        let mut state = self.state();
        self.modules
            .before_child_rpc(
                &child_method_name,
                &mut child_ctx.state,
                request,
                &mut child_wire,
                &mut state.ext,
            )
            .map_err(|rejection| rejection.status)?;
        if let Err(rejection) = self.modules.seal_child_rpc(
            &child_method_name,
            &mut child_ctx.state,
            request,
            &mut child_wire,
            &mut state.ext,
        ) {
            self.modules.reject_child_rpc(
                &child_method_name,
                rejection.by,
                &rejection.status,
                &child_ctx.state,
                &state.ext,
            );
            return Err(rejection.status);
        }
        drop(state);

        // The child request carries exactly what the modules put: nothing is
        // copied down from this request.
        child_wire.install(request.metadata_mut());

        Ok(())
    }

    fn after_child_rpc<T>(
        &self,
        _method: GrpcMethod,
        response: &mut Result<Response<T>, Status>,
        child_ctx: ChildContext<S>,
    ) -> Result<(), Status> {
        if let Some(child_method) = child_ctx.child_method_name.as_ref() {
            let metadata = match &*response {
                Ok(resp) => resp.metadata(),
                Err(status) => status.metadata(),
            };
            let response_wire =
                WireIn::from_metadata(metadata).unwrap_or_else(|err| panic!("{err}"));
            self.modules.after_child_rpc(
                child_method,
                ChildOutcome::Sent(&*response),
                &response_wire,
                &child_ctx.state,
                &self.state().ext,
            )?;
        }
        Ok(())
    }

    #[inline]
    fn after_poll<Ret>(
        &self,
        poll: &Poll<Result<Response<Ret>, Status>>,
    ) -> Result<(), Result<Response<Ret>, Status>> {
        if !S::POLL_HOOKS && !cfg!(debug_assertions) {
            return Ok(());
        }
        let mut state = self.state();
        match self.modules.after_poll(poll, &state.ext) {
            Ok(()) => Ok(()),
            Err(early) => {
                state.record(&early);
                Err(early.reply)
            }
        }
    }

    fn finalize_before_serialization<Ret>(&self, result: &mut Result<Response<Ret>, Status>) {
        let mut wire = WireOut::new();
        let state = self.state();
        self.modules
            .finalize(result, state.outcome(), &mut wire, &state.ext);
        let metadata = match result {
            Ok(resp) => resp.metadata_mut(),
            Err(status) => status.metadata_mut(),
        };
        wire.install(metadata);
    }
}

#[derive(Debug)]
pub struct ChildContext<S: ModuleStack> {
    pub child_method_name: Option<CowGrpcMethod>,
    /// What the modules keep for this one child RPC.
    state: ChildState,
    root_method: CowGrpcMethod,
    root_sent: bool,
    stack: PhantomData<fn() -> S>,
}

impl<S: ModuleStack> ClientHooks for ChildContext<S> {
    fn new<T>(method: GrpcMethod, request: &Request<T>) -> Self {
        Self {
            child_method_name: None,
            state: ChildState::new(),
            root_method: resolve_method_name_from_request(method, request),
            root_sent: false,
            stack: PhantomData,
        }
    }
    fn try_before_send<T>(&mut self, request: &mut Request<T>) -> Result<(), Status> {
        if self.child_method_name.is_some() {
            return Ok(());
        }
        let mut wire = WireOut::from_metadata(request.metadata())
            .map_err(|err| Status::invalid_argument(format!("invalid root metadata: {err}")))?;
        S::before_root_rpc(&self.root_method, request, &mut wire, &mut self.state)
            .map_err(|rejection| rejection.status)?;
        // Empty root stacks preserve caller-supplied metadata for compatibility.
        if !wire.header_value().is_empty() {
            wire.install(request.metadata_mut());
        }
        self.root_sent = true;
        Ok(())
    }

    fn after_recv<T>(&mut self, response: &mut Result<Response<T>, Status>) {
        if !self.root_sent {
            return;
        }
        self.root_sent = false;
        let metadata = match &*response {
            Ok(r) => r.metadata(),
            Err(s) => s.metadata(),
        };
        let result = match WireIn::from_metadata(metadata) {
            Ok(wire) => S::after_root_rpc(
                &self.root_method,
                ChildOutcome::Sent(response),
                &wire,
                &self.state,
            ),
            Err(err) => Err(Status::internal(format!(
                "invalid root response metadata: {err}"
            ))),
        };
        if let Err(status) = result {
            *response = Err(status);
        }
    }
}

impl<S: ModuleStack> ChildContext<S> {
    /// What the modules keep for this one child RPC.
    pub fn state(&self) -> &ChildState {
        &self.state
    }

    pub fn set_method_name(&mut self, name: CowGrpcMethod) {
        self.child_method_name = Some(name);
    }
}

#[cfg(test)]
mod tests {
    use super::{resolve_method_name_from_http, resolve_method_name_from_request};
    use crate::metadata::{
        set_method_name_override_in_headers, set_service_name_override_in_headers, RequestExt,
    };
    use tonic::{GrpcMethod, Request};

    #[test]
    fn resolve_method_name_from_http_with_overrides() {
        let method = GrpcMethod::new("TestService", "TestMethod");
        let mut req = http::Request::new(());

        let resolved = resolve_method_name_from_http(method, &req);
        assert_eq!(resolved.service(), "TestService");
        assert_eq!(resolved.method(), "TestMethod");

        set_method_name_override_in_headers(req.headers_mut(), "OverriddenMethod").unwrap();
        let resolved = resolve_method_name_from_http(method, &req);
        assert_eq!(resolved.service(), "TestService");
        assert_eq!(resolved.method(), "OverriddenMethod");

        set_service_name_override_in_headers(req.headers_mut(), "OverriddenService").unwrap();
        let resolved = resolve_method_name_from_http(method, &req);
        assert_eq!(resolved.service(), "OverriddenService");
        assert_eq!(resolved.method(), "OverriddenMethod");
    }

    #[test]
    fn resolve_method_name_from_request_with_overrides() {
        let method = GrpcMethod::new("TestService", "TestMethod");
        let mut req = Request::new(());

        let resolved = resolve_method_name_from_request(method, &req);
        assert_eq!(resolved.service(), "TestService");
        assert_eq!(resolved.method(), "TestMethod");

        req.set_method_name_override("OverriddenMethod").unwrap();
        let resolved = resolve_method_name_from_request(method, &req);
        assert_eq!(resolved.service(), "TestService");
        assert_eq!(resolved.method(), "OverriddenMethod");

        req.set_service_name_override("OverriddenService").unwrap();
        let resolved = resolve_method_name_from_request(method, &req);
        assert_eq!(resolved.service(), "OverriddenService");
        assert_eq!(resolved.method(), "OverriddenMethod");
    }
}
