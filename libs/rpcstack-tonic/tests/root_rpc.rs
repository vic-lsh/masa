use rpcstack::{policy_stack, ChildOutcome, ChildState, Extensions, Module, WireIn, WireOut};
use rpcstack_tonic::{ChildContext, ParentContext, ServerContext};
use std::sync::{Arc, Mutex};
use tonic::masa::{ClientHooks, ParentHooks, ServerHooks};
use tonic::{CowGrpcMethod, GrpcMethod, Request, Response, Status};

type Events = Arc<Mutex<Vec<&'static str>>>;
#[derive(Debug)]
struct First;
#[derive(Debug)]
struct Second;
macro_rules! module {
    ($ty:ident, $name:literal, $before:literal, $after:literal) => {
        impl Module for $ty {
            type Server = ();
            type Wire = u64;
            const NAME: &'static str = $name;
            fn new(_: &CowGrpcMethod, _: &(), _: &WireIn<'_>, _: &mut Extensions) -> Self {
                Self
            }
            fn before_root_rpc<T>(
                method: &CowGrpcMethod,
                request: &mut Request<T>,
                wire: &mut WireOut,
                state: &mut ChildState,
            ) -> Result<(), Status> {
                let events = request.extensions().get::<Events>().unwrap().clone();
                events.lock().unwrap().push($before);
                state.insert(events);
                wire.put::<Self>(&7).unwrap();
                if method.method() == $name {
                    return Err(Status::resource_exhausted("reject"));
                }
                Ok(())
            }
            fn after_root_rpc<T>(
                _: &CowGrpcMethod,
                outcome: ChildOutcome<'_, T>,
                wire: &WireIn<'_>,
                state: &ChildState,
            ) -> Result<(), Status> {
                state.get::<Events>().unwrap().lock().unwrap().push($after);
                if outcome.sent().is_some() {
                    assert_eq!(wire.get::<First>().unwrap(), Some(9));
                }
                Ok(())
            }
        }
    };
}
module!(First, "first", "before first", "after first");
module!(Second, "second", "before second", "after second");
type Stack = policy_stack![First, Second];

#[test]
fn root_rejection_unwinds_only_entered_modules() {
    for (method, expected) in [
        ("first", vec!["before first", "after first"]),
        (
            "second",
            vec![
                "before first",
                "before second",
                "after second",
                "after first",
            ],
        ),
    ] {
        let events = Events::default();
        let mut req = Request::new(());
        req.extensions_mut().insert(events.clone());
        let mut child = ChildContext::<Stack>::new(GrpcMethod::new("test", method), &req);
        assert!(child.try_before_send(&mut req).is_err());
        child.after_recv(&mut Err::<Response<()>, _>(Status::unknown("never sent")));
        assert_eq!(*events.lock().unwrap(), expected);
    }
}

#[test]
fn root_error_response_metadata_reaches_all_modules_in_reverse_order() {
    let events = Events::default();
    let mut req = Request::new(());
    req.extensions_mut().insert(events.clone());
    let mut child = ChildContext::<Stack>::new(GrpcMethod::new("test", "ok"), &req);
    child.try_before_send(&mut req).unwrap();
    let mut wire = WireOut::new();
    wire.put::<First>(&9).unwrap();
    let mut status = Status::resource_exhausted("overload");
    wire.install(status.metadata_mut());
    child.after_recv(&mut Err::<Response<()>, _>(status));
    assert_eq!(
        *events.lock().unwrap(),
        [
            "before first",
            "before second",
            "after second",
            "after first"
        ]
    );
}

#[test]
fn child_calls_do_not_run_root_hooks() {
    let method = GrpcMethod::new("test", "child");
    let parent = ParentContext::<Stack>::begin(
        method,
        &http::Request::new(()),
        Arc::new(ServerContext::<Stack>::new("test")),
    );
    let mut req = Request::new(());
    let mut child = ChildContext::<Stack>::new(method, &req);
    parent
        .before_child_rpc(method, &mut req, &mut child)
        .unwrap();
    child.try_before_send(&mut req).unwrap(); // would panic if root hooks ran
    child.after_recv(&mut Ok(Response::new(())));
}
