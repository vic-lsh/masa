use masa_core::Context;
use tonic_core::CowGrpcMethod;

use super::super::{Layer, LayerChild, LayerServer};

#[derive(Debug)]
pub(crate) struct NoopServer;

impl NoopServer {
    pub(crate) fn new() -> Self {
        Self
    }
}

impl LayerServer for NoopServer {}

#[derive(Debug)]
pub(crate) struct NoopLayer;

impl Layer for NoopLayer {
    type Server = NoopServer;
    type Child = NoopChild;

    fn new(_method: &CowGrpcMethod, _server: &NoopServer, _ctx: &mut Context) -> Self {
        Self
    }
}

#[derive(Debug, Clone)]
pub(crate) struct NoopChild;

impl LayerChild for NoopChild {
    fn new() -> Self {
        Self
    }
}
