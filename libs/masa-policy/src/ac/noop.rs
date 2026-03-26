// No-op admission control handler.
//
// Used when neither `ac_rajomon` nor `ac_est` feature flags are enabled.
// All methods use the trait defaults (admit everything, no metadata).

use tonic_core::CowGrpcMethod;

use super::AcHandler;

#[derive(Debug)]
#[allow(dead_code)]
pub(crate) struct NoopAcHandler;

impl AcHandler for NoopAcHandler {
    fn new(_method: CowGrpcMethod) -> Self {
        Self
    }
}
