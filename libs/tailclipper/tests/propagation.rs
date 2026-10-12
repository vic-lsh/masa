use rpcstack::{policy_stack, Extensions, Module, WireIn, WireOut, HEADER_NAME};
use rpcstack_tonic::{ChildContext, ParentContext, RequestExt, ServerContext};
use std::sync::Arc;
use tailclipper::{TailClipperModule, TailClipperWire};
use tonic::masa::{ClientHooks, ParentHooks, ServerHooks};
use tonic::{CowGrpcMethod, GrpcMethod, Request};
type Stack = policy_stack![TailClipperModule];
#[test]
fn root_stamps_and_fanout_preserves_arrival_time() {
    let method = GrpcMethod::new("test", "Call");
    let mut root = Request::new(());
    let mut child = ChildContext::<Stack>::new(method, &root);
    child.try_before_send(&mut root).unwrap();
    let arrival = root.get_wire::<TailClipperModule>().unwrap();
    assert!(arrival.arrival_ns > 0);
    let mut incoming = http::Request::new(());
    incoming.headers_mut().insert(
        HEADER_NAME,
        root.metadata()
            .get(HEADER_NAME)
            .unwrap()
            .to_str()
            .unwrap()
            .parse()
            .unwrap(),
    );
    let parent = ParentContext::<Stack>::begin(
        method,
        &incoming,
        Arc::new(ServerContext::<Stack>::new("test")),
    );
    for _ in 0..3 {
        let mut request = Request::new(());
        let mut child = ChildContext::<Stack>::new(method, &request);
        parent
            .before_child_rpc(method, &mut request, &mut child)
            .unwrap();
        child.try_before_send(&mut request).unwrap();
        assert_eq!(request.get_wire::<TailClipperModule>(), Some(arrival));
        assert_eq!(
            request
                .metadata()
                .get(rpcstack_wire::SCHEDULING_HEADER)
                .unwrap()
                .to_str()
                .unwrap(),
            arrival.arrival_ns.to_string()
        );
    }
}
#[test]
fn zero_arrival_rejects_before_running_handler() {
    let mut out = WireOut::new();
    out.put::<TailClipperModule>(&TailClipperWire { arrival_ns: 0 })
        .unwrap();
    let mut metadata = tonic::metadata::MetadataMap::new();
    out.install(&mut metadata);
    let wire = WireIn::from_metadata(&metadata).unwrap();
    let module = TailClipperModule::new(
        &CowGrpcMethod::new("test", "Call"),
        &(),
        &wire,
        &mut Extensions::new(),
    );
    assert!(module.before_poll::<()>(&mut Extensions::new()).is_err());
}
