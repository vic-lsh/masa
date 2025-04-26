use std::collections::HashSet;

use super::{Attributes, Method, Service};
use crate::{
    format_method_name, format_method_path, format_service_name, generate_doc_comments,
    naive_snake_case,
};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};

pub(crate) fn generate_internal<T: Service>(
    service: &T,
    emit_package: bool,
    proto_path: &str,
    compile_well_known_types: bool,
    build_transport: bool,
    enable_parent_rpc_ctx: bool,
    attributes: &Attributes,
    disable_comments: &HashSet<String>,
) -> TokenStream {
    let service_ident = quote::format_ident!("{}Client", service.name());
    let client_mod = quote::format_ident!("{}_client", naive_snake_case(service.name()));
    let methods = generate_methods(
        service,
        emit_package,
        proto_path,
        compile_well_known_types,
        enable_parent_rpc_ctx,
        disable_comments,
    );

    let connect = generate_connect(&service_ident, build_transport);

    let package = if emit_package { service.package() } else { "" };
    let service_name = format_service_name(service, emit_package);

    let service_doc = if disable_comments.contains(&service_name) {
        TokenStream::new()
    } else {
        generate_doc_comments(service.comment())
    };

    let mod_attributes = attributes.for_mod(package);
    let struct_attributes = attributes.for_struct(&service_name);

    let get_parent_rpc_ctx = if enable_parent_rpc_ctx {
        generate_get_parent_rpc_ctx(service)
    } else {
        TokenStream::new()
    };

    quote! {
        /// Generated client implementations.
        #(#mod_attributes)*
        pub mod #client_mod {
            #![allow(
                unused_variables,
                dead_code,
                missing_docs,
                // will trigger if compression is disabled
                clippy::let_unit_value,
            )]
            use tonic::codegen::*;
            use tonic::codegen::http::Uri;

            // requried to call functions in the trait, referenced through PrioritySelector
            #[allow(unused_imports)]
            use tonic::masa::{ClientHooks, ParentHooks};

            #service_doc
            #(#struct_attributes)*
            #[derive(Debug)]
            pub struct #service_ident<
                T,
                P: tonic::masa::PrioritySelector = tonic::masa::DefaultPrioritySelector,
            > {
                inner: tonic::client::Grpc<T>,
                _ctx_ty: std::marker::PhantomData<P>,
            }

            impl<T, P> Clone for #service_ident<T, P>
            where
                T: Clone,
                P: tonic::masa::PrioritySelector
            {
                fn clone(&self) -> Self {
                    Self {
                        inner: self.inner.clone(),
                        _ctx_ty: std::marker::PhantomData
                    }
                }
            }

            #connect

            impl<T> #service_ident<T, tonic::masa::DefaultPrioritySelector>
            where
                T: tonic::client::GrpcService<tonic::body::BoxBody>,
                T::Error: Into<StdError>,
                T::ResponseBody: Body<Data = Bytes> + Send  + 'static,
                <T::ResponseBody as Body>::Error: Into<StdError> + Send,
            {
                pub fn new(inner: T) -> Self {
                    Self::new_impl(inner)
                }

                pub fn with_origin(inner: T, origin: Uri) -> Self {
                    let inner = tonic::client::Grpc::with_origin(inner, origin);
                    Self {
                        inner,
                        _ctx_ty: std::marker::PhantomData
                    }
                }

                pub fn with_interceptor<F>(inner: T, interceptor: F) -> #service_ident<InterceptedService<T, F>>
                where
                    F: tonic::service::Interceptor,
                    T::ResponseBody: Default,
                    T: tonic::codegen::Service<
                        http::Request<tonic::body::BoxBody>,
                        Response = http::Response<<T as tonic::client::GrpcService<tonic::body::BoxBody>>::ResponseBody>
                    >,
                    <T as tonic::codegen::Service<http::Request<tonic::body::BoxBody>>>::Error: Into<StdError> + Send + Sync,
                {
                    #service_ident::new(InterceptedService::new(inner, interceptor))
                }

            }

            impl<T, P> #service_ident<T, P>
            where
                T: tonic::client::GrpcService<tonic::body::BoxBody>,
                T::Error: Into<StdError>,
                T::ResponseBody: Body<Data = Bytes> + Send  + 'static,
                <T::ResponseBody as Body>::Error: Into<StdError> + Send,
                P: tonic::masa::PrioritySelector
            {
                fn new_impl(inner: T) -> Self {
                    let inner = tonic::client::Grpc::new(inner);
                    Self {
                        inner,
                        _ctx_ty: std::marker::PhantomData
                    }
                }

                /// Compress requests with the given encoding.
                ///
                /// This requires the server to support it otherwise it might respond with an
                /// error.
                #[must_use]
                pub fn send_compressed(mut self, encoding: CompressionEncoding) -> Self {
                    self.inner = self.inner.send_compressed(encoding);
                    self
                }

                /// Enable decompressing responses.
                #[must_use]
                pub fn accept_compressed(mut self, encoding: CompressionEncoding) -> Self {
                    self.inner = self.inner.accept_compressed(encoding);
                    self
                }

                /// Limits the maximum size of a decoded message.
                ///
                /// Default: `4MB`
                #[must_use]
                pub fn max_decoding_message_size(mut self, limit: usize) -> Self {
                    self.inner = self.inner.max_decoding_message_size(limit);
                    self
                }

                /// Limits the maximum size of an encoded message.
                ///
                /// Default: `usize::MAX`
                #[must_use]
                pub fn max_encoding_message_size(mut self, limit: usize) -> Self {
                    self.inner = self.inner.max_encoding_message_size(limit);
                    self
                }

                #get_parent_rpc_ctx

                #methods
            }
        }
    }
}

fn generate_get_parent_rpc_ctx(_service: &impl Service) -> TokenStream {
    quote! {
        /// Internal. Obtain the parent RPC in which this RPC client stub operates.
        fn get_parent_ctx(&self) -> Option<&'_ P::ParentContext> {
            // SAFETY:
            // - same parent ctx type used in client and server
            //   - if the caller didn't configure P (the normal case where applications
            //     use clients and servers), code-gen ensures the same P type across
            //     client and server.
            //   - if custom P types are configured (e.g., in tests), the user have to
            //     ensure that. code-gen doesn't enforce this rule yet.
            unsafe { tonic::masa::context::client::get_parent_ctx::<P>() }
        }
    }
}

#[cfg(feature = "transport")]
fn generate_connect(service_ident: &syn::Ident, enabled: bool) -> TokenStream {
    let connect_impl = quote! {
        impl #service_ident<
            tonic::transport::Channel,
            tonic::masa::DefaultPrioritySelector,
        > {
            /// Attempt to create a new client by connecting to a given endpoint.
            pub async fn connect<D>(dst: D) -> Result<Self, tonic::transport::Error>
            where
                D: TryInto<tonic::transport::Endpoint>,
                D::Error: Into<StdError>,
            {
                let conn = tonic::transport::Endpoint::new(dst)?.connect().await?;
                Ok(Self::new_impl(conn))
            }
        }

        impl<P> #service_ident<tonic::transport::Channel, P>
        where
            P: tonic::masa::PrioritySelector
        {
            /// Attempt to create a new client by connecting to a given endpoint.
            pub async fn connect_with_custom_context<D>(dst: D) -> Result<Self, tonic::transport::Error>
            where
                D: TryInto<tonic::transport::Endpoint>,
                D::Error: Into<StdError>,
            {
                let conn = tonic::transport::Endpoint::new(dst)?.connect().await?;
                Ok(Self::new_impl(conn))
            }
        }
    };

    if enabled {
        connect_impl
    } else {
        TokenStream::new()
    }
}

#[cfg(not(feature = "transport"))]
fn generate_connect(_service_ident: &syn::Ident, _enabled: bool) -> TokenStream {
    TokenStream::new()
}

fn generate_methods<T: Service>(
    service: &T,
    emit_package: bool,
    proto_path: &str,
    compile_well_known_types: bool,
    enable_parent_rpc_ctx: bool,
    disable_comments: &HashSet<String>,
) -> TokenStream {
    let mut stream = TokenStream::new();

    for method in service.methods() {
        if !disable_comments.contains(&format_method_name(service, method, emit_package)) {
            stream.extend(generate_doc_comments(method.comment()));
        }

        let method = match (method.client_streaming(), method.server_streaming()) {
            (false, false) => generate_unary(
                service,
                method,
                emit_package,
                proto_path,
                compile_well_known_types,
                enable_parent_rpc_ctx,
            ),
            (false, true) => generate_server_streaming(
                service,
                method,
                emit_package,
                proto_path,
                compile_well_known_types,
            ),
            (true, false) => generate_client_streaming(
                service,
                method,
                emit_package,
                proto_path,
                compile_well_known_types,
            ),
            (true, true) => generate_streaming(
                service,
                method,
                emit_package,
                proto_path,
                compile_well_known_types,
            ),
        };

        stream.extend(method);
    }

    stream
}

fn generate_unary<T: Service>(
    service: &T,
    method: &T::Method,
    emit_package: bool,
    proto_path: &str,
    compile_well_known_types: bool,
    enable_parent_rpc_ctx: bool,
) -> TokenStream {
    let codec_name = syn::parse_str::<syn::Path>(method.codec_path()).unwrap();
    let ident = format_ident!("{}", method.name());
    let (request, response) = method.request_response_name(proto_path, compile_well_known_types);
    let service_name = format_service_name(service, emit_package);
    let path = format_method_path(service, method, emit_package);
    let method_name = method.identifier();

    let before_child_rpc = if enable_parent_rpc_ctx {
        quote! {
            if let Some(parent_ctx) = self.get_parent_ctx() {
                // log::info!("into parent ctx, before rpc, method: {:?}", grpc_method);
                if let Err(s) = parent_ctx.before_child_rpc(grpc_method, &mut req, &mut child_ctx) {
                    return Err(s);
                }
            }
        }
    } else {
        TokenStream::new()
    };

    let after_child_rpc = if enable_parent_rpc_ctx {
        quote! {
            if let Some(parent_ctx) = self.get_parent_ctx() {
                // log::info!("into parent ctx, after rpc, method: {:?}", grpc_method);
                if let Err(status) = parent_ctx.after_child_rpc(grpc_method, &mut resp, child_ctx) {
                    return Err(status);
                }
            }
        }
    } else {
        TokenStream::new()
    };

    quote! {
        pub async fn #ident(
            &mut self,
            request: impl tonic::IntoRequest<#request>,
        ) -> std::result::Result<tonic::Response<#response>, tonic::Status> {
           self.inner.ready().await.map_err(|e| {
               tonic::Status::new(tonic::Code::Unknown, format!("Service was not ready: {}", e.into()))
           })?;
           let codec = #codec_name::default();
           // [NOTE] Method path name on the client side.
           let path = http::uri::PathAndQuery::from_static(#path);
           let mut req = request.into_request();
           let grpc_method = GrpcMethod::new(#service_name, #method_name);
           req.extensions_mut().insert(grpc_method);

           let mut child_ctx = P::ChildContext::new(grpc_method, &req);

           #before_child_rpc

           child_ctx.before_send(&mut req);
           #[allow(unused_mut)]
           let mut resp = self.inner.unary(req, path, codec).await;
           child_ctx.after_recv(&mut resp);

           #after_child_rpc

           resp
        }
    }
}

fn generate_server_streaming<T: Service>(
    service: &T,
    method: &T::Method,
    emit_package: bool,
    proto_path: &str,
    compile_well_known_types: bool,
) -> TokenStream {
    let codec_name = syn::parse_str::<syn::Path>(method.codec_path()).unwrap();
    let ident = format_ident!("{}", method.name());
    let (request, response) = method.request_response_name(proto_path, compile_well_known_types);
    let service_name = format_service_name(service, emit_package);
    let path = format_method_path(service, method, emit_package);
    let method_name = method.identifier();

    quote! {
        pub async fn #ident(
            &mut self,
            request: impl tonic::IntoRequest<#request>,
        ) -> std::result::Result<tonic::Response<tonic::codec::Streaming<#response>>, tonic::Status> {
            self.inner.ready().await.map_err(|e| {
                        tonic::Status::new(tonic::Code::Unknown, format!("Service was not ready: {}", e.into()))
            })?;
            let codec = #codec_name::default();
            let path = http::uri::PathAndQuery::from_static(#path);
            let mut req = request.into_request();
            req.extensions_mut().insert(GrpcMethod::new(#service_name, #method_name));
            self.inner.server_streaming(req, path, codec).await
        }
    }
}

fn generate_client_streaming<T: Service>(
    service: &T,
    method: &T::Method,
    emit_package: bool,
    proto_path: &str,
    compile_well_known_types: bool,
) -> TokenStream {
    let codec_name = syn::parse_str::<syn::Path>(method.codec_path()).unwrap();
    let ident = format_ident!("{}", method.name());
    let (request, response) = method.request_response_name(proto_path, compile_well_known_types);
    let service_name = format_service_name(service, emit_package);
    let path = format_method_path(service, method, emit_package);
    let method_name = method.identifier();

    quote! {
        pub async fn #ident(
            &mut self,
            request: impl tonic::IntoStreamingRequest<Message = #request>
        ) -> std::result::Result<tonic::Response<#response>, tonic::Status> {
            self.inner.ready().await.map_err(|e| {
                        tonic::Status::new(tonic::Code::Unknown, format!("Service was not ready: {}", e.into()))
            })?;
            let codec = #codec_name::default();
            let path = http::uri::PathAndQuery::from_static(#path);
            let mut req = request.into_streaming_request();
            req.extensions_mut().insert(GrpcMethod::new(#service_name, #method_name));
            self.inner.client_streaming(req, path, codec).await
        }
    }
}

fn generate_streaming<T: Service>(
    service: &T,
    method: &T::Method,
    emit_package: bool,
    proto_path: &str,
    compile_well_known_types: bool,
) -> TokenStream {
    let codec_name = syn::parse_str::<syn::Path>(method.codec_path()).unwrap();
    let ident = format_ident!("{}", method.name());
    let (request, response) = method.request_response_name(proto_path, compile_well_known_types);
    let service_name = format_service_name(service, emit_package);
    let path = format_method_path(service, method, emit_package);
    let method_name = method.identifier();

    quote! {
        pub async fn #ident(
            &mut self,
            request: impl tonic::IntoStreamingRequest<Message = #request>
        ) -> std::result::Result<tonic::Response<tonic::codec::Streaming<#response>>, tonic::Status> {
            self.inner.ready().await.map_err(|e| {
                        tonic::Status::new(tonic::Code::Unknown, format!("Service was not ready: {}", e.into()))
            })?;
            let codec = #codec_name::default();
            let path = http::uri::PathAndQuery::from_static(#path);
            let mut req = request.into_streaming_request();
            req.extensions_mut().insert(GrpcMethod::new(#service_name,#method_name));
            self.inner.streaming(req, path, codec).await
        }
    }
}
