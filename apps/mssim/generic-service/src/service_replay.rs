use crate::busy_spin;
use crate::core::ServiceCore;
use crate::service_stubs::local_span::SpanType;
use crate::service_stubs::span::Kind;
use crate::service_stubs::{ChildSpans, LocalSpan, ReplayRequest};
use crate::RpcClient;
use masa::Context as MasaContext;
use sim_config::svc::ServiceName;
use std::collections::HashMap;
use tokio::sync::RwLockReadGuard;
use tokio::time::{sleep, Duration};
use tonic::{Request, Status};
use tracing::warn;

pub(crate) struct ReplaySpanExecutor<'a> {
    request: &'a ReplayRequest,
    clients: RwLockReadGuard<'a, HashMap<ServiceName, RpcClient>>,
}

impl<'a> ReplaySpanExecutor<'a> {
    pub(crate) async fn new(
        state: &'a ServiceCore,
        request: &'a ReplayRequest,
    ) -> Result<Self, Status> {
        let clients = state.read_clients().await;
        Ok(Self { request, clients })
    }

    pub(crate) async fn run(self) -> Result<(), Status> {
        for span in &self.request.spans {
            if let Some(kind) = span.kind.as_ref() {
                match kind {
                    Kind::LocalSpan(local) => self.execute_local_span(local).await?,
                    Kind::ChildSpans(children) => self.execute_child_call(children).await?,
                }
            }
        }
        Ok(())
    }

    async fn execute_local_span(&self, local: &LocalSpan) -> Result<(), Status> {
        match SpanType::try_from(local.r#type) {
            Ok(SpanType::Compute) => {
                busy_spin(std::time::Duration::from_micros(local.val));
            }
            Ok(SpanType::Block) => {
                sleep(Duration::from_micros(local.val)).await;
            }
            Ok(SpanType::Unknown) => {
                warn!("Unknown span type, skipping");
            }
            Err(_) => {
                warn!("Invalid span type, skipping");
            }
        }
        Ok(())
    }

    async fn execute_child_call(&self, children: &ChildSpans) -> Result<(), Status> {
        let child_service = ServiceName::from_string(children.name.clone());
        let client = self.clients.get(&child_service).ok_or_else(|| {
            Status::not_found(format!("Child service {} not found", children.name))
        })?;

        let mut request = Request::new(ReplayRequest {
            req_id: self.request.req_id,
            exclude_queue_latency: self.request.exclude_queue_latency,
            slo: self.request.slo,
            start_at: self.request.start_at,
            deadline: self.request.deadline,
            spans: children.spans.clone(),
        });

        let ctx = MasaContext::new(
            "replay".to_string(),
            self.request.req_id,
            self.request.slo,
            self.request.start_at,
            self.request.deadline,
        );
        request.metadata_mut().insert_ctx("ctx", &ctx);

        let mut client = client.clone();
        client.replay(request).await.map_err(|e| {
            Status::internal(format!(
                "RPC to child service {} failed: {:?}",
                children.name, e
            ))
        })?;

        Ok(())
    }
}
