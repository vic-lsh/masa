// The module trait, and the stack that composes modules.
//
// A policy module is a type implementing `Module`. Modules are composed into a
// stack with `policy_stack!`, and a hook adapter (such as `rpcstack-tonic`'s
// `PolicyHooks<S>`) drives every lifecycle hook through the stack. The framework owns the composition rules, so a module
// states only what it does at each hook:
//
// - Pre-hooks (`new`, `before_poll`, `before_child_rpc`) run in stack order and
//   the first `Err` short-circuits the rest.
// - `seal_child_rpc` runs once per child RPC after every `before_child_rpc`
//   succeeded, in reverse stack order; a module can reject the child there.
// - Several modules can contribute to one decision (a child's cost, say):
//   they `propose` typed values, the framework records them with the proposing
//   module's name, and the module that owns the decision `resolve`s them in its
//   `seal_child_rpc` by its own rule. The framework has none.
// - Post-hooks (`after_child_rpc`, `finalize`) run in reverse stack order, and
//   only for the modules whose pre-hook ran: every `before_child_rpc` is
//   followed by exactly one `after_child_rpc`, whether the child was sent or a
//   module rejected it (`ChildOutcome`), and `finalize` learns whether a module
//   ended the request (`Outcome`).
// - `after_poll` decides, like `before_poll`, so it runs in stack order and
//   short-circuits.
// - Modules name the modules that must precede them (`Module::requires`); a
//   stack that violates this fails at server construction.
//
// `()` is the empty module, so a disabled slot in a stack costs nothing.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::task::Poll;

use serde::de::DeserializeOwned;
use serde::Serialize;
use tonic::{CowGrpcMethod, Response, Status};

use crate::extensions::{ChildState, Extensions};
use crate::wire::{assert_valid_name, WireIn, WireOut};

// ── Server state and dependencies ───────────────────────────────────────

/// Server-level module state, created once per service and shared across all
/// requests.
pub trait ModuleServer: Send + Sync + std::fmt::Debug + Sized {
    /// Construct the server state.
    ///
    /// Modules earlier in the stack are constructed first, so a module can
    /// consume state that an earlier module published with
    /// [`ServerInit::provide`]. A module whose prerequisite is missing returns
    /// the [`MissingDependency`] from [`ServerInit::require`] instead of
    /// panicking, so a misordered stack is reported at construction.
    fn new(init: &mut ServerInit) -> Result<Self, MissingDependency>;
}

/// What a module needs that its stack does not provide.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Missing {
    /// Server state that no earlier module published.
    ServerState,
    /// A module that the stack does not contain.
    ModuleAbsent,
    /// A module that the stack contains, but after the one that needs it.
    ModuleLater,
}

/// A module needs another module, or server state, that the stack does not
/// provide before it. Returned when the server state of a stack is built, so a
/// misconfigured stack fails at construction, never at the first request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingDependency {
    resource: &'static str,
    module: &'static str,
    missing: Missing,
}

impl MissingDependency {
    /// Type name of what is missing: a module, or a value of server state.
    pub fn resource(&self) -> &'static str {
        self.resource
    }

    /// Type name of the module that needed it.
    pub fn module(&self) -> &'static str {
        self.module
    }
}

impl std::fmt::Display for MissingDependency {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.missing {
            Missing::ServerState => write!(
                f,
                "policy stack misconfigured: module server `{}` requires a `{}` published by a \
                 module earlier in the stack, but none did; add the providing module or move it \
                 before `{}`",
                self.module, self.resource, self.module
            ),
            Missing::ModuleAbsent => write!(
                f,
                "policy stack misconfigured: module `{}` requires module `{}` earlier in the \
                 stack, but the stack has none; add `{}` before `{}`",
                self.module, self.resource, self.resource, self.module
            ),
            Missing::ModuleLater => write!(
                f,
                "policy stack misconfigured: module `{}` requires module `{}` earlier in the \
                 stack, but it comes later; move `{}` before `{}`",
                self.module, self.resource, self.resource, self.module
            ),
        }
    }
}

impl std::error::Error for MissingDependency {}

/// Construction context passed to every [`ModuleServer::new`] in stack order.
///
/// Carries the service name and a typed store through which modules share
/// server-level state (e.g., the estimation module publishes its latency
/// estimators and predictive admission control reads them).
#[derive(Debug)]
pub struct ServerInit {
    service_name: &'static str,
    /// Type name of the module server being constructed, for error messages.
    module: &'static str,
    resources: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
}

impl ServerInit {
    pub fn new(service_name: &'static str) -> Self {
        Self {
            service_name,
            module: "",
            resources: HashMap::new(),
        }
    }

    /// Record which module server is about to be constructed.
    fn enter<M: ModuleServer>(&mut self) {
        self.module = std::any::type_name::<M>();
    }

    /// Name of the service whose hooks are being constructed.
    pub fn service_name(&self) -> &'static str {
        self.service_name
    }

    /// Publish a value for modules later in the stack. Replaces any value
    /// previously published under the same type.
    pub fn provide<T: Any + Send + Sync + Clone>(&mut self, value: T) {
        self.resources.insert(TypeId::of::<T>(), Box::new(value));
    }

    /// Read a value published by an earlier module.
    pub fn get<T: Any + Send + Sync + Clone>(&self) -> Option<T> {
        self.resources
            .get(&TypeId::of::<T>())
            .and_then(|value| value.downcast_ref::<T>())
            .cloned()
    }

    /// Read a value published by an earlier module, or fail with an error
    /// naming the missing type and the module that needed it.
    pub fn require<T: Any + Send + Sync + Clone>(&self) -> Result<T, MissingDependency> {
        self.get::<T>().ok_or(MissingDependency {
            resource: std::any::type_name::<T>(),
            module: self.module,
            missing: Missing::ServerState,
        })
    }
}

/// The modules a module declares must precede it in the stack; filled by
/// [`Module::requires`].
#[derive(Debug, Default)]
pub struct Requires {
    modules: Vec<(TypeId, &'static str)>,
}

impl Requires {
    /// `M` must appear earlier in the stack than the module declaring this.
    pub fn module<M: Module>(&mut self) -> &mut Self {
        self.modules
            .push((TypeId::of::<M>(), std::any::type_name::<M>()));
        self
    }
}

/// One module of a stack, as seen by construction-time validation.
#[doc(hidden)]
#[derive(Debug)]
pub struct ModuleDecl {
    id: TypeId,
    type_name: &'static str,
    name: &'static str,
    has_wire: bool,
    is_empty: bool,
    requires: Requires,
}

impl ModuleDecl {
    fn of<M: Module>() -> Self {
        let mut requires = Requires::default();
        M::requires(&mut requires);
        Self {
            id: TypeId::of::<M>(),
            type_name: std::any::type_name::<M>(),
            name: M::NAME,
            has_wire: TypeId::of::<M::Wire>() != TypeId::of::<()>(),
            is_empty: TypeId::of::<M>() == TypeId::of::<()>(),
            requires,
        }
    }
}

/// Build the server state of the stack `S`, or report the first module whose
/// required module or server state the stack does not provide before it.
///
/// Panics if two modules share a [`Module::NAME`], which would make their
/// outcomes, proposals and wire data indistinguishable, or if the name of a
/// module with wire data cannot be a section name.
pub fn build_server<S: ModuleStack>(
    service_name: &'static str,
) -> Result<S::Server, MissingDependency> {
    validate_stack::<S>()?;
    S::Server::new(&mut ServerInit::new(service_name))
}

/// Fails unless every module of the stack `S` is preceded by the modules it
/// requires.
fn validate_stack<S: ModuleStack>() -> Result<(), MissingDependency> {
    let mut modules = Vec::new();
    S::describe(&mut modules);

    for (at, module) in modules.iter().enumerate().filter(|(_, m)| !m.is_empty) {
        if module.has_wire {
            assert_valid_name(module.name);
        }
        if let Some(first) = modules[..at]
            .iter()
            .find(|earlier| !earlier.is_empty && earlier.name == module.name)
        {
            panic!(
                "modules `{}` and `{}` in the policy stack share the name `{}`; `Module::NAME` \
                 must be unique",
                first.type_name, module.type_name, module.name
            );
        }
    }

    for (at, module) in modules.iter().enumerate() {
        for &(id, type_name) in &module.requires.modules {
            if modules[..at].iter().any(|earlier| earlier.id == id) {
                continue;
            }
            let later = modules[at + 1..].iter().any(|later| later.id == id);
            return Err(MissingDependency {
                resource: type_name,
                module: module.type_name,
                missing: if later {
                    Missing::ModuleLater
                } else {
                    Missing::ModuleAbsent
                },
            });
        }
    }
    Ok(())
}

// ── The module trait ────────────────────────────────────────────────────

/// How the request ended, as `finalize` sees it.
///
/// Modules entered before a later module ended the request learn about it
/// here, so an observer placed early in the stack sees everything that
/// happens after it. The cause is structured data (the rejecting module's
/// [`Module::NAME`] and the status it gave), so an observer never parses a
/// status message. It is what the framework recorded when the module ended the
/// request and does not change if a module rewrites the result in `finalize`:
/// the cause (`outcome`) and the final result (`result`) are both visible.
#[derive(Debug, Clone, Copy)]
pub enum Outcome<'a> {
    /// No module ended the request: the handler produced the result, which
    /// may itself be an error status (including one the handler made from a
    /// rejected child RPC).
    Handled,
    /// A module's `before_poll` or `after_poll` ended the request with an
    /// error status.
    Rejected {
        /// [`Module::NAME`] of the module that rejected.
        by: &'static str,
        status: &'a Status,
    },
    /// A module's `before_poll` or `after_poll` ended the request with a
    /// response it supplied itself.
    Replied {
        /// [`Module::NAME`] of the module that replied.
        by: &'static str,
    },
}

impl Outcome<'_> {
    /// [`Module::NAME`] of the module that ended the request, if one did.
    pub fn ended_by(&self) -> Option<&'static str> {
        match *self {
            Self::Handled => None,
            Self::Rejected { by, .. } | Self::Replied { by } => Some(by),
        }
    }
}

/// How a child RPC ended, as `after_child_rpc` sees it.
#[derive(Debug)]
pub enum ChildOutcome<'a, T> {
    /// The child RPC was sent; this is its response, or the error status it
    /// failed with.
    Sent(&'a Result<Response<T>, Status>),
    /// A module rejected the child RPC, so it was not sent.
    Rejected {
        /// [`Module::NAME`] of the module that rejected.
        by: &'static str,
        status: &'a Status,
    },
}

impl<T> Clone for ChildOutcome<'_, T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for ChildOutcome<'_, T> {}

impl<'a, T> ChildOutcome<'a, T> {
    /// The response, or the status of a failed one, if the child RPC was sent;
    /// `None` if a module rejected it.
    pub fn sent(&self) -> Option<&'a Result<Response<T>, Status>> {
        match *self {
            Self::Sent(response) => Some(response),
            Self::Rejected { .. } => None,
        }
    }
}

/// A policy module: per-request state that hooks into the request lifecycle
/// at key points to implement policy-specific logic (estimation, admission
/// control, etc.).
///
/// All hooks have default no-op implementations so that modules only need
/// to override the hooks they care about. The framework decides the order in
/// which a stack's modules run each hook and which modules run it; see the
/// documentation of each hook.
pub trait Module: Send + Sync + std::fmt::Debug + 'static {
    type Server: ModuleServer;

    /// Name of this module. Identifies it in [`Outcome`] and in proposals, and,
    /// when `Wire` is not `()`, names its section in the wire envelope. It must
    /// be unique among the modules of a stack (checked when the server state is
    /// built; the empty module `()` is exempt). When `Wire` is not `()` it must
    /// also be ASCII letters, digits, `_` or `-`, and stable across the
    /// binaries of a deployment.
    const NAME: &'static str;

    /// The type of the data this module exchanges with other hops. The module
    /// reads it with [`WireIn::get`] and writes it with [`WireOut::put`]; the
    /// framework never inspects, defaults or forwards it. A module with no
    /// wire data uses `()`.
    ///
    /// Associated type defaults are unstable, so every module declares this
    /// explicitly. Wrap a field in `Option` when its absence must be told
    /// apart from any value, such as zero.
    type Wire: Serialize + DeserializeOwned + Send + Sync + 'static;

    /// Whether this module overrides [`Module::before_poll`] or
    /// [`Module::after_poll`]. A stack in which no module does skips the poll
    /// hooks entirely: they run on every poll of every request, and the
    /// framework takes a lock around the request's state to call them. Leave it
    /// at `true` unless the module keeps both defaults; a module that sets it
    /// to `false` and overrides a poll hook is a bug that debug builds report
    /// with a panic at the first poll, because release builds would never call
    /// the hook.
    const POLL_HOOKS: bool = true;

    /// Admit and annotate an RPC originating outside a server request.
    /// Runs head first. On rejection, entered modules receive `after_root_rpc`
    /// in reverse order with `ChildOutcome::Rejected`; the RPC is never sent.
    /// State is private to this call. Shared client state belongs to the module.
    fn before_root_rpc<T>(
        _method: &CowGrpcMethod,
        _request: &mut tonic::Request<T>,
        _wire: &mut WireOut,
        _state: &mut ChildState,
    ) -> Result<(), Status> {
        Ok(())
    }

    /// Observe a root RPC response (including error metadata), or local rejection.
    fn after_root_rpc<T>(
        _method: &CowGrpcMethod,
        _outcome: ChildOutcome<'_, T>,
        _wire: &WireIn<'_>,
        _state: &ChildState,
    ) -> Result<(), Status> {
        Ok(())
    }

    /// Declare the modules that must appear earlier in the stack than this
    /// one, with [`Requires::module`]. A stack that omits one or puts it later
    /// fails when the server state is built, with an error naming both
    /// modules. Declare what the module reads from [`Extensions`] or
    /// [`ChildState`]: a module that does so unconditionally would otherwise
    /// fail at its first request.
    fn requires(_requires: &mut Requires) {}

    /// Construct per-request module state.
    ///
    /// `wire` is the inbound request's wire sections; the module reads its own
    /// with `wire.get::<Self>()`, which is `None` if the sender attached
    /// none.
    ///
    /// `ext` is the request's [`Extensions`], empty when the first module
    /// runs; modules earlier in the stack may already have stored values.
    ///
    /// Runs for every module, in stack order; every module therefore gets
    /// [`Module::finalize`].
    fn new(
        method: &CowGrpcMethod,
        server: &Self::Server,
        wire: &WireIn<'_>,
        ext: &mut Extensions,
    ) -> Self;

    /// Called before each poll of the handler future.
    ///
    /// Returns `Err` to end the request. Runs in stack order; the first `Err`
    /// ends the request without polling the handler, and the modules after it
    /// do not run. No `after_poll` follows; every module still learns about
    /// the rejection in [`Module::finalize`].
    fn before_poll<Ret>(&self, _ext: &mut Extensions) -> Result<(), Result<Response<Ret>, Status>> {
        #[cfg(debug_assertions)]
        default_poll_hook_ran();
        Ok(())
    }

    /// Called before each outbound child RPC.
    ///
    /// The module may reject the child RPC (returning `Err`). Runs in stack
    /// order; the first `Err` rejects the child RPC and the modules after it
    /// do not run. The modules that ran, including the rejecting one, get
    /// [`Module::after_child_rpc`] in reverse order with
    /// [`ChildOutcome::Rejected`]; the others get nothing.
    ///
    /// `child_wire` starts empty and becomes the child request's wire
    /// sections: a module that does not `put` anything sends nothing, and
    /// nothing is carried over from this request's inbound wire. `child` holds
    /// this child's state, keyed by type, which no other child sees. A
    /// decision several modules share is made by proposing to it, with
    /// `child.propose(value)`, where the decision type is defined by the module
    /// that owns it. A module that needs the result of the modules after it
    /// completes its work in [`Module::seal_child_rpc`].
    fn before_child_rpc<T>(
        &self,
        _child_method: &CowGrpcMethod,
        _child: &mut ChildState,
        _request: &mut tonic::Request<T>,
        _child_wire: &mut WireOut,
        _ext: &mut Extensions,
    ) -> Result<(), Status> {
        Ok(())
    }

    /// Called once per child RPC after every module's `before_child_rpc`
    /// accepted it, in reverse stack order: the module nearest the head of the
    /// stack runs last and sees what every module after it decided.
    ///
    /// This is where the owner of a decision settles it: `child.resolve::<D>()`
    /// returns every module's proposal with the proposing module's name, and the
    /// owner applies its own rule and may write its own wire section for the
    /// child. Returns `Err` to reject the child RPC: it is then not sent, and every
    /// module gets [`Module::after_child_rpc`] with [`ChildOutcome::Rejected`]
    /// (the modules that had not yet sealed do not seal).
    fn seal_child_rpc<T>(
        &self,
        _child_method: &CowGrpcMethod,
        _child: &mut ChildState,
        _request: &mut tonic::Request<T>,
        _child_wire: &mut WireOut,
        _ext: &mut Extensions,
    ) -> Result<(), Status> {
        Ok(())
    }

    /// Called once for every child RPC this module's `before_child_rpc` ran
    /// for, when the child RPC is over: after its response arrived
    /// ([`ChildOutcome::Sent`]), or when a module rejected it
    /// ([`ChildOutcome::Rejected`], the child was not sent). A module that
    /// counts children in `before_child_rpc` undoes it here for both.
    ///
    /// `response_wire` is the wire sections the child's modules attached to
    /// the response (or to the error status); read this module's own with
    /// `response_wire.get::<Self>()`. It is empty for a rejected child. It
    /// borrows from the response, which is therefore read-only here. The
    /// framework carries none of it forward: whatever this module wants to
    /// report upstream it must `put` in [`Module::finalize`].
    ///
    /// Runs in reverse stack order, and an `Err` does not skip the others. For
    /// a sent child, the first `Err` in that order is returned to the handler
    /// as the child RPC's failure; for a rejected child the rejection stands and
    /// an `Err` is ignored.
    fn after_child_rpc<T>(
        &self,
        _child_method: &CowGrpcMethod,
        _outcome: ChildOutcome<'_, T>,
        _response_wire: &WireIn<'_>,
        _child: &ChildState,
        _ext: &Extensions,
    ) -> Result<(), Status> {
        Ok(())
    }

    /// Called after each poll of the handler future.
    ///
    /// Returns `Err` to end the request (e.g., a guard checking on `Pending`).
    /// A decision point like `before_poll`, so it runs in stack order and the
    /// first `Err` ends the request: the order in which modules may end it is
    /// the same before and after the poll, and a poll has no nesting to
    /// unwind.
    fn after_poll<Ret>(
        &self,
        _poll: &Poll<Result<Response<Ret>, Status>>,
        _ext: &Extensions,
    ) -> Result<(), Result<Response<Ret>, Status>> {
        #[cfg(debug_assertions)]
        default_poll_hook_ran();
        Ok(())
    }

    /// Called before the response is serialized and sent.
    ///
    /// `outcome` says whether a module ended the request. Modules report to
    /// the caller by `put`ting their own section into `wire`, which starts
    /// empty and becomes the response's wire sections; nothing from the
    /// request or from child responses is carried into it.
    ///
    /// Runs for every module, in reverse stack order: modules nearer the
    /// head of the stack finalize last and so wrap everything after them. The
    /// data modules publish in [`Extensions`] while handling the request
    /// (rather than in their own `finalize`) is therefore complete for
    /// every `finalize`, whatever the order.
    fn finalize<Ret>(
        &self,
        _result: &mut Result<Response<Ret>, Status>,
        _outcome: Outcome<'_>,
        _wire: &mut WireOut,
        _ext: &Extensions,
    ) {
    }
}

// ── Checking `Module::POLL_HOOKS` ───────────────────────────────────────

#[cfg(debug_assertions)]
thread_local! {
    /// How many default poll hooks have run on this thread. The defaults count
    /// themselves, so a module that declares `POLL_HOOKS = false` can be caught
    /// running its own override.
    static DEFAULT_POLL_HOOKS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

#[cfg(debug_assertions)]
fn default_poll_hook_ran() {
    DEFAULT_POLL_HOOKS.with(|count| count.set(count.get() + 1));
}

/// Runs a poll hook of a module that declares `POLL_HOOKS = false`, which a
/// release build skips, and panics unless it was the default.
#[cfg(debug_assertions)]
fn assert_default_poll_hook<M: Module>(hook: &str, run: impl FnOnce()) {
    let before = DEFAULT_POLL_HOOKS.with(std::cell::Cell::get);
    run();
    assert!(
        DEFAULT_POLL_HOOKS.with(std::cell::Cell::get) == before + 1,
        "module `{}` overrides `{hook}` but declares `Module::POLL_HOOKS = false`, so release \
         builds never call it; remove the declaration",
        M::NAME
    );
}

// ── The empty module and stacks ─────────────────────────────────────────

impl ModuleServer for () {
    fn new(_init: &mut ServerInit) -> Result<Self, MissingDependency> {
        Ok(())
    }
}

/// The empty module. Fills a disabled slot in a stack.
impl Module for () {
    type Server = ();
    const NAME: &'static str = "";
    type Wire = ();
    const POLL_HOOKS: bool = false;

    fn new(
        _method: &CowGrpcMethod,
        _server: &(),
        _wire: &WireIn<'_>,
        _ext: &mut Extensions,
    ) -> Self {
    }
}

impl<A: ModuleServer, B: ModuleServer> ModuleServer for (A, B) {
    fn new(init: &mut ServerInit) -> Result<Self, MissingDependency> {
        init.enter::<A>();
        let head = A::new(init)?;
        init.enter::<B>();
        Ok((head, B::new(init)?))
    }
}

/// A reply ending a request from `before_poll` or `after_poll`, and the
/// module that gave it.
#[derive(Debug)]
pub struct Early<Ret> {
    pub by: &'static str,
    pub reply: Result<Response<Ret>, Status>,
}

/// A child RPC rejected by a module.
#[derive(Debug)]
pub struct Rejection {
    pub by: &'static str,
    pub status: Status,
}

/// A module stack: the modules of a [`Stack`], or `()` for none. This is where
/// the framework's rules for running modules live; modules implement
/// [`Module`] and never this trait.
pub trait ModuleStack: Send + Sync + std::fmt::Debug + 'static {
    #[doc(hidden)]
    type Server: ModuleServer;

    /// Whether any module of the stack has poll hooks
    /// ([`Module::POLL_HOOKS`]); if none does, the adapter need not call
    /// `before_poll` or `after_poll`.
    #[doc(hidden)]
    const POLL_HOOKS: bool;

    #[doc(hidden)]
    fn before_root_rpc<R>(
        method: &CowGrpcMethod,
        request: &mut tonic::Request<R>,
        wire: &mut WireOut,
        state: &mut ChildState,
    ) -> Result<(), Rejection>;

    #[doc(hidden)]
    fn after_root_rpc<R>(
        method: &CowGrpcMethod,
        outcome: ChildOutcome<'_, R>,
        wire: &WireIn<'_>,
        state: &ChildState,
    ) -> Result<(), Status>;

    #[doc(hidden)]
    fn describe(modules: &mut Vec<ModuleDecl>);

    #[doc(hidden)]
    fn new(
        method: &CowGrpcMethod,
        server: &Self::Server,
        wire: &WireIn<'_>,
        ext: &mut Extensions,
    ) -> Self;

    #[doc(hidden)]
    fn before_poll<Ret>(&self, ext: &mut Extensions) -> Result<(), Early<Ret>>;

    #[doc(hidden)]
    fn after_poll<Ret>(
        &self,
        poll: &Poll<Result<Response<Ret>, Status>>,
        ext: &Extensions,
    ) -> Result<(), Early<Ret>>;

    #[doc(hidden)]
    fn before_child_rpc<T>(
        &self,
        child_method: &CowGrpcMethod,
        child: &mut ChildState,
        request: &mut tonic::Request<T>,
        child_wire: &mut WireOut,
        ext: &mut Extensions,
    ) -> Result<(), Rejection>;

    #[doc(hidden)]
    fn seal_child_rpc<T>(
        &self,
        child_method: &CowGrpcMethod,
        child: &mut ChildState,
        request: &mut tonic::Request<T>,
        child_wire: &mut WireOut,
        ext: &mut Extensions,
    ) -> Result<(), Rejection>;

    /// Tell every module that a module rejected the child RPC in
    /// `seal_child_rpc`, when all of them had accepted it.
    #[doc(hidden)]
    fn reject_child_rpc(
        &self,
        child_method: &CowGrpcMethod,
        by: &'static str,
        status: &Status,
        child: &ChildState,
        ext: &Extensions,
    );

    #[doc(hidden)]
    fn after_child_rpc<T>(
        &self,
        child_method: &CowGrpcMethod,
        outcome: ChildOutcome<'_, T>,
        response_wire: &WireIn<'_>,
        child: &ChildState,
        ext: &Extensions,
    ) -> Result<(), Status>;

    #[doc(hidden)]
    fn finalize<Ret>(
        &self,
        result: &mut Result<Response<Ret>, Status>,
        outcome: Outcome<'_>,
        wire: &mut WireOut,
        ext: &Extensions,
    );
}

impl ModuleStack for () {
    type Server = ();
    const POLL_HOOKS: bool = false;

    fn before_root_rpc<R>(
        _method: &CowGrpcMethod,
        _request: &mut tonic::Request<R>,
        _wire: &mut WireOut,
        _state: &mut ChildState,
    ) -> Result<(), Rejection> {
        Ok(())
    }
    fn after_root_rpc<R>(
        _method: &CowGrpcMethod,
        _outcome: ChildOutcome<'_, R>,
        _wire: &WireIn<'_>,
        _state: &ChildState,
    ) -> Result<(), Status> {
        Ok(())
    }

    fn describe(_modules: &mut Vec<ModuleDecl>) {}

    fn new(
        _method: &CowGrpcMethod,
        _server: &(),
        _wire: &WireIn<'_>,
        _ext: &mut Extensions,
    ) -> Self {
    }

    fn before_poll<Ret>(&self, _ext: &mut Extensions) -> Result<(), Early<Ret>> {
        Ok(())
    }

    fn after_poll<Ret>(
        &self,
        _poll: &Poll<Result<Response<Ret>, Status>>,
        _ext: &Extensions,
    ) -> Result<(), Early<Ret>> {
        Ok(())
    }

    fn before_child_rpc<T>(
        &self,
        _child_method: &CowGrpcMethod,
        _child: &mut ChildState,
        _request: &mut tonic::Request<T>,
        _child_wire: &mut WireOut,
        _ext: &mut Extensions,
    ) -> Result<(), Rejection> {
        Ok(())
    }

    fn seal_child_rpc<T>(
        &self,
        _child_method: &CowGrpcMethod,
        _child: &mut ChildState,
        _request: &mut tonic::Request<T>,
        _child_wire: &mut WireOut,
        _ext: &mut Extensions,
    ) -> Result<(), Rejection> {
        Ok(())
    }

    fn reject_child_rpc(
        &self,
        _child_method: &CowGrpcMethod,
        _by: &'static str,
        _status: &Status,
        _child: &ChildState,
        _ext: &Extensions,
    ) {
    }

    fn after_child_rpc<T>(
        &self,
        _child_method: &CowGrpcMethod,
        _outcome: ChildOutcome<'_, T>,
        _response_wire: &WireIn<'_>,
        _child: &ChildState,
        _ext: &Extensions,
    ) -> Result<(), Status> {
        Ok(())
    }

    fn finalize<Ret>(
        &self,
        _result: &mut Result<Response<Ret>, Status>,
        _outcome: Outcome<'_>,
        _wire: &mut WireOut,
        _ext: &Extensions,
    ) {
    }
}

/// A module stack: runs `head` before every module in `tail` in pre-hooks, and
/// after every module in `tail` in post-hooks.
///
/// Build stacks with [`policy_stack!`](crate::policy_stack) rather than by
/// hand.
#[derive(Debug)]
pub struct Stack<H, T> {
    head: H,
    tail: T,
}

impl<H: Module, T: ModuleStack> ModuleStack for Stack<H, T> {
    type Server = (H::Server, T::Server);
    const POLL_HOOKS: bool = H::POLL_HOOKS || T::POLL_HOOKS;

    fn before_root_rpc<R>(
        method: &CowGrpcMethod,
        request: &mut tonic::Request<R>,
        wire: &mut WireOut,
        state: &mut ChildState,
    ) -> Result<(), Rejection> {
        state.set_module(H::NAME);
        let rejection = match H::before_root_rpc(method, request, wire, state) {
            Err(status) => Rejection {
                by: H::NAME,
                status,
            },
            Ok(()) => match T::before_root_rpc(method, request, wire, state) {
                Ok(()) => return Ok(()),
                Err(rejection) => rejection,
            },
        };
        let outcome: ChildOutcome<'_, ()> = ChildOutcome::Rejected {
            by: rejection.by,
            status: &rejection.status,
        };
        let _ = H::after_root_rpc(method, outcome, &WireIn::default(), state);
        Err(rejection)
    }

    fn after_root_rpc<R>(
        method: &CowGrpcMethod,
        outcome: ChildOutcome<'_, R>,
        wire: &WireIn<'_>,
        state: &ChildState,
    ) -> Result<(), Status> {
        let tail = T::after_root_rpc(method, outcome, wire, state);
        let head = H::after_root_rpc(method, outcome, wire, state);
        tail.and(head)
    }

    fn describe(modules: &mut Vec<ModuleDecl>) {
        modules.push(ModuleDecl::of::<H>());
        T::describe(modules);
    }

    fn new(
        method: &CowGrpcMethod,
        server: &Self::Server,
        wire: &WireIn<'_>,
        ext: &mut Extensions,
    ) -> Self {
        let head = H::new(method, &server.0, wire, ext);
        Self {
            head,
            tail: T::new(method, &server.1, wire, ext),
        }
    }

    #[inline]
    fn before_poll<Ret>(&self, ext: &mut Extensions) -> Result<(), Early<Ret>> {
        if H::POLL_HOOKS {
            ext.set_module(H::NAME);
            self.head
                .before_poll(ext)
                .map_err(|reply| Early { by: H::NAME, reply })?;
        } else {
            #[cfg(debug_assertions)]
            assert_default_poll_hook::<H>("before_poll", || {
                let _ = self.head.before_poll::<Ret>(ext);
            });
        }
        self.tail.before_poll(ext)
    }

    #[inline]
    fn after_poll<Ret>(
        &self,
        poll: &Poll<Result<Response<Ret>, Status>>,
        ext: &Extensions,
    ) -> Result<(), Early<Ret>> {
        if H::POLL_HOOKS {
            self.head
                .after_poll(poll, ext)
                .map_err(|reply| Early { by: H::NAME, reply })?;
        } else {
            #[cfg(debug_assertions)]
            assert_default_poll_hook::<H>("after_poll", || {
                let _ = self.head.after_poll(poll, ext);
            });
        }
        self.tail.after_poll(poll, ext)
    }

    #[inline]
    fn before_child_rpc<R>(
        &self,
        child_method: &CowGrpcMethod,
        child: &mut ChildState,
        request: &mut tonic::Request<R>,
        child_wire: &mut WireOut,
        ext: &mut Extensions,
    ) -> Result<(), Rejection> {
        child.set_module(H::NAME);
        ext.set_module(H::NAME);
        let rejection =
            match self
                .head
                .before_child_rpc(child_method, child, request, child_wire, ext)
            {
                Err(status) => Rejection {
                    by: H::NAME,
                    status,
                },
                Ok(()) => {
                    match self
                        .tail
                        .before_child_rpc(child_method, child, request, child_wire, ext)
                    {
                        Ok(()) => return Ok(()),
                        Err(rejection) => rejection,
                    }
                }
            };
        // The head ran, so it learns of the rejection after the modules
        // behind it, which have already been told. An error from the hook
        // cannot undo the rejection.
        let rejected: ChildOutcome<'_, ()> = ChildOutcome::Rejected {
            by: rejection.by,
            status: &rejection.status,
        };
        let _ = self
            .head
            .after_child_rpc(child_method, rejected, &WireIn::default(), child, ext);
        Err(rejection)
    }

    #[inline]
    fn seal_child_rpc<R>(
        &self,
        child_method: &CowGrpcMethod,
        child: &mut ChildState,
        request: &mut tonic::Request<R>,
        child_wire: &mut WireOut,
        ext: &mut Extensions,
    ) -> Result<(), Rejection> {
        self.tail
            .seal_child_rpc(child_method, child, request, child_wire, ext)?;
        child.set_module(H::NAME);
        ext.set_module(H::NAME);
        self.head
            .seal_child_rpc(child_method, child, request, child_wire, ext)
            .map_err(|status| Rejection {
                by: H::NAME,
                status,
            })
    }

    #[inline]
    fn reject_child_rpc(
        &self,
        child_method: &CowGrpcMethod,
        by: &'static str,
        status: &Status,
        child: &ChildState,
        ext: &Extensions,
    ) {
        self.tail
            .reject_child_rpc(child_method, by, status, child, ext);
        let rejected: ChildOutcome<'_, ()> = ChildOutcome::Rejected { by, status };
        let _ = self
            .head
            .after_child_rpc(child_method, rejected, &WireIn::default(), child, ext);
    }

    #[inline]
    fn after_child_rpc<R>(
        &self,
        child_method: &CowGrpcMethod,
        outcome: ChildOutcome<'_, R>,
        response_wire: &WireIn<'_>,
        child: &ChildState,
        ext: &Extensions,
    ) -> Result<(), Status> {
        let tail = self
            .tail
            .after_child_rpc(child_method, outcome, response_wire, child, ext);
        let head = self
            .head
            .after_child_rpc(child_method, outcome, response_wire, child, ext);
        tail.and(head)
    }

    #[inline]
    fn finalize<Ret>(
        &self,
        result: &mut Result<Response<Ret>, Status>,
        outcome: Outcome<'_>,
        wire: &mut WireOut,
        ext: &Extensions,
    ) {
        self.tail.finalize(result, outcome, wire, ext);
        self.head.finalize(result, outcome, wire, ext);
    }
}

/// Compose policy modules into a stack type. Modules run in the listed order
/// in pre-hooks and in the reverse order in post-hooks.
///
/// ```ignore
/// type MyStack = rpcstack::policy_stack![MyGuard, MyAdmission];
/// type MyHooks = rpcstack_tonic::PolicyHooks<MyStack>;
/// ```
#[macro_export]
macro_rules! policy_stack {
    () => { () };
    ($head:ty $(, $tail:ty)* $(,)?) => {
        $crate::Stack<$head, $crate::policy_stack![$($tail),*]>
    };
}
