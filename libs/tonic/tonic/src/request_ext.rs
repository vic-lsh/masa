//! Transport-specific extension methods for Request.
//!
//! These methods moved from inherent methods on `Request` to an extension
//! trait because `Request` is now defined in `tonic-core` (which has no
//! transport dependencies).
//!
//! Import `tonic::RequestExt` to use `remote_addr()`, `local_addr()`, and
//! `peer_certs()` on `Request`.

use crate::Request;

#[cfg(feature = "transport")]
use crate::transport::server::TcpConnectInfo;
#[cfg(feature = "tls")]
use crate::transport::{server::TlsConnectInfo, Certificate};
#[cfg(feature = "transport")]
use std::net::SocketAddr;
#[cfg(feature = "tls")]
use std::sync::Arc;

/// Transport-specific extension methods for [`Request`].
///
/// Import this trait to access `remote_addr()`, `local_addr()`, and
/// `peer_certs()` on any `Request<T>`.
#[cfg(feature = "transport")]
pub trait RequestExt {
    /// Get the local address of this connection.
    ///
    /// This will return `None` if the `IO` type used
    /// does not implement `Connected` or when using a unix domain socket.
    /// This currently only works on the server side.
    fn local_addr(&self) -> Option<SocketAddr>;

    /// Get the remote address of this connection.
    ///
    /// This will return `None` if the `IO` type used
    /// does not implement `Connected` or when using a unix domain socket.
    /// This currently only works on the server side.
    fn remote_addr(&self) -> Option<SocketAddr>;

    /// Get the peer certificates of the connected client.
    ///
    /// This is used to fetch the certificates from the TLS session
    /// and is mostly used for mTLS. This currently only returns
    /// `Some` on the server side of the `transport` server with
    /// TLS enabled connections.
    #[cfg(feature = "tls")]
    fn peer_certs(&self) -> Option<Arc<Vec<Certificate>>>;
}

#[cfg(feature = "transport")]
impl<T> RequestExt for Request<T> {
    fn local_addr(&self) -> Option<SocketAddr> {
        let addr = self
            .extensions()
            .get::<TcpConnectInfo>()
            .and_then(|i| i.local_addr());

        #[cfg(feature = "tls")]
        let addr = addr.or_else(|| {
            self.extensions()
                .get::<TlsConnectInfo<TcpConnectInfo>>()
                .and_then(|i| i.get_ref().local_addr())
        });

        addr
    }

    fn remote_addr(&self) -> Option<SocketAddr> {
        let addr = self
            .extensions()
            .get::<TcpConnectInfo>()
            .and_then(|i| i.remote_addr());

        #[cfg(feature = "tls")]
        let addr = addr.or_else(|| {
            self.extensions()
                .get::<TlsConnectInfo<TcpConnectInfo>>()
                .and_then(|i| i.get_ref().remote_addr())
        });

        addr
    }

    #[cfg(feature = "tls")]
    fn peer_certs(&self) -> Option<Arc<Vec<Certificate>>> {
        self.extensions()
            .get::<TlsConnectInfo<TcpConnectInfo>>()
            .and_then(|i| i.peer_certs())
    }
}
