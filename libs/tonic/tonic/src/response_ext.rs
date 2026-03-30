//! Codec-specific extension methods for Response.
//!
//! These methods moved from inherent methods on `Response` to an extension
//! trait because `Response` is now defined in `tonic-core` (which has no
//! codec dependencies).
//!
//! Import `tonic::ResponseExt` to use `disable_compression()` on `Response`.

#[cfg(feature = "gzip")]
use crate::Response;

/// Codec-specific extension methods for [`Response`](crate::Response).
///
/// Import this trait to access `disable_compression()` on any `Response<T>`.
#[cfg(feature = "gzip")]
pub trait ResponseExt {
    /// Disable compression of the response body.
    ///
    /// This disables compression of the body of this response, even if compression is enabled on
    /// the server.
    ///
    /// **Note**: This only has effect on responses to unary requests and responses to client to
    /// server streams. Response streams (server to client stream and bidirectional streams) will
    /// still be compressed according to the configuration of the server.
    fn disable_compression(&mut self);
}

#[cfg(feature = "gzip")]
impl<T> ResponseExt for Response<T> {
    fn disable_compression(&mut self) {
        self.extensions_mut()
            .insert(crate::codec::compression::SingleMessageCompressionOverride::Disable);
    }
}
