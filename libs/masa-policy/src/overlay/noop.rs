// No-op overlay — used when neither `est` nor `ac_rajomon` is enabled.
//
// All methods use the default no-op implementations from the trait, so
// they compile away entirely.

use masa_core::Context;
use tonic_core::CowGrpcMethod;

use super::{Overlay, OverlayChild, OverlayServer};

#[derive(Debug)]
pub(crate) struct NoopOverlayServer;

impl OverlayServer for NoopOverlayServer {
    fn new() -> Self {
        Self
    }
}

#[derive(Debug)]
pub(crate) struct NoopOverlay;

impl Overlay for NoopOverlay {
    type Server = NoopOverlayServer;
    type Child = NoopOverlayChild;

    fn new(_method: &CowGrpcMethod, _server: &NoopOverlayServer, _ctx: &mut Context) -> Self {
        Self
    }

    // All other methods use default no-op impls from the trait.
}

#[derive(Debug, Clone)]
pub(crate) struct NoopOverlayChild;

impl OverlayChild for NoopOverlayChild {
    fn new() -> Self {
        Self
    }
}
