// A stack without poll hooks skips them, and a module cannot declare that it
// has none while overriding one.

use std::cell::Cell;
use std::sync::Arc;
use std::task::Poll;

use rpcstack::{policy_stack, Extensions, Module, ModuleStack, WireIn};
use rpcstack_tonic::PolicyHooks;
use tonic::masa::{Hooks, ParentHooks, ServerHooks};
use tonic::{CowGrpcMethod, GrpcMethod, Response, Status};

type Server<S> = <PolicyHooks<S> as Hooks>::ServerContext;
type Parent<S> = <PolicyHooks<S> as Hooks>::ParentContext;

// These hooks are driven synchronously on each test's thread. The
// should-panic test also runs Counts, so process-global counters race.
thread_local! {
    static BEFORE: Cell<u32> = const { Cell::new(0) };
    static AFTER: Cell<u32> = const { Cell::new(0) };
}

macro_rules! module {
    ($ty:ident, $name:literal, $flag:expr) => {
        #[derive(Debug)]
        struct $ty;

        impl Module for $ty {
            type Server = ();
            const NAME: &'static str = $name;
            type Wire = ();
            const POLL_HOOKS: bool = $flag;

            fn new(_: &CowGrpcMethod, _: &(), _: &WireIn<'_>, _: &mut Extensions) -> Self {
                Self
            }
        }
    };
}

module!(Quiet, "quiet", false);
module!(QuietToo, "quiet_too", false);

/// Overrides both poll hooks and keeps the default declaration.
#[derive(Debug)]
struct Counts;

impl Module for Counts {
    type Server = ();
    const NAME: &'static str = "counts";
    type Wire = ();

    fn new(_: &CowGrpcMethod, _: &(), _: &WireIn<'_>, _: &mut Extensions) -> Self {
        Self
    }

    fn before_poll<Ret>(&self, _: &mut Extensions) -> Result<(), Result<Response<Ret>, Status>> {
        BEFORE.set(BEFORE.get() + 1);
        Ok(())
    }

    fn after_poll<Ret>(
        &self,
        _: &Poll<Result<Response<Ret>, Status>>,
        _: &Extensions,
    ) -> Result<(), Result<Response<Ret>, Status>> {
        AFTER.set(AFTER.get() + 1);
        Ok(())
    }
}

/// Overrides `before_poll` while declaring it has no poll hooks.
#[derive(Debug)]
struct LiesBefore;

impl Module for LiesBefore {
    type Server = ();
    const NAME: &'static str = "lies_before";
    type Wire = ();
    const POLL_HOOKS: bool = false;

    fn new(_: &CowGrpcMethod, _: &(), _: &WireIn<'_>, _: &mut Extensions) -> Self {
        Self
    }

    fn before_poll<Ret>(&self, _: &mut Extensions) -> Result<(), Result<Response<Ret>, Status>> {
        Ok(())
    }
}

/// Overrides `after_poll` while declaring it has no poll hooks.
#[derive(Debug)]
struct LiesAfter;

impl Module for LiesAfter {
    type Server = ();
    const NAME: &'static str = "lies_after";
    type Wire = ();
    const POLL_HOOKS: bool = false;

    fn new(_: &CowGrpcMethod, _: &(), _: &WireIn<'_>, _: &mut Extensions) -> Self {
        Self
    }

    fn after_poll<Ret>(
        &self,
        _: &Poll<Result<Response<Ret>, Status>>,
        _: &Extensions,
    ) -> Result<(), Result<Response<Ret>, Status>> {
        Ok(())
    }
}

fn begin<S: ModuleStack>() -> Parent<S> {
    Parent::<S>::begin(
        GrpcMethod::new("poll_hooks", "Parent"),
        &http::Request::new(()),
        Arc::new(Server::<S>::new("poll_hooks")),
    )
}

fn poll<S: ModuleStack>(parent: &Parent<S>) {
    parent.before_poll::<()>().unwrap();
    parent.after_poll::<()>(&Poll::Pending).unwrap();
}

#[test]
fn a_stack_declares_poll_hooks_when_any_module_has_them() {
    assert!(!<policy_stack![] as ModuleStack>::POLL_HOOKS);
    assert!(!<policy_stack![Quiet, QuietToo] as ModuleStack>::POLL_HOOKS);
    assert!(<policy_stack![Quiet, Counts, QuietToo] as ModuleStack>::POLL_HOOKS);
}

#[test]
fn modules_with_poll_hooks_run_them_next_to_modules_without() {
    type S = policy_stack![Quiet, Counts, QuietToo];
    let parent = begin::<S>();
    let (before, after) = (BEFORE.get(), AFTER.get());
    poll::<S>(&parent);
    poll::<S>(&parent);
    assert_eq!(BEFORE.get() - before, 2);
    assert_eq!(AFTER.get() - after, 2);
}

#[test]
fn a_stack_without_poll_hooks_polls_without_effect() {
    type S = policy_stack![Quiet, QuietToo];
    let parent = begin::<S>();
    poll::<S>(&parent);
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "overrides `before_poll` but declares `Module::POLL_HOOKS = false`")]
fn overriding_before_poll_while_declaring_none_is_caught() {
    type S = policy_stack![Quiet, LiesBefore];
    poll::<S>(&begin::<S>());
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "overrides `after_poll` but declares `Module::POLL_HOOKS = false`")]
fn overriding_after_poll_while_declaring_none_is_caught() {
    type S = policy_stack![LiesAfter, Counts];
    poll::<S>(&begin::<S>());
}
