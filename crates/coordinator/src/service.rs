use crate::registry::Registry;
use gpucluster_proto::coordinator::{
    coordinator_service_server::CoordinatorService,
    Ack, GpuMetricsRequest, HeartbeatRequest, HeartbeatResponse, JobAssignment,
    JobStatusUpdate, RegisterRequest, RegisterResponse, StreamJobsRequest,
};
use tonic::{Request, Response, Status};

pub struct CoordSvc {
    pub registry: Registry,
}

#[tonic::async_trait]
impl CoordinatorService for CoordSvc {
    async fn register(
        &self,
        req: Request<RegisterRequest>,
    ) -> Result<Response<RegisterResponse>, Status> {
        let r = req.into_inner();
        let info = r.info.ok_or_else(|| Status::invalid_argument("info required"))?;
        let id = if r.node_id.is_empty() {
            uuid::Uuid::now_v7().to_string()
        } else {
            r.node_id
        };

        let mut info = info;
        info.node_id = id.clone();
        self.registry.upsert(info, None);

        Ok(Response::new(RegisterResponse {
            assigned_id: id,
            coordinator_endpoint: String::new(),
            heartbeat_interval_secs: 5,
        }))
    }

    type HeartbeatStream =
        std::pin::Pin<Box<dyn tokio_stream::Stream<Item = Result<HeartbeatResponse, Status>> + Send>>;

    async fn heartbeat(
        &self,
        _req: Request<tonic::Streaming<HeartbeatRequest>>,
    ) -> Result<Response<Self::HeartbeatStream>, Status> {
        Err(Status::unimplemented("phase 1+: heartbeat stream"))
    }

    async fn report_gpu_metrics(
        &self,
        _req: Request<GpuMetricsRequest>,
    ) -> Result<Response<Ack>, Status> {
        Ok(Response::new(Ack { ok: true, message: String::new() }))
    }

    type StreamJobsStream =
        std::pin::Pin<Box<dyn tokio_stream::Stream<Item = Result<JobAssignment, Status>> + Send>>;

    async fn stream_jobs(
        &self,
        _req: Request<StreamJobsRequest>,
    ) -> Result<Response<Self::StreamJobsStream>, Status> {
        Err(Status::unimplemented("phase 2+: job streaming"))
    }

    async fn report_job_status(
        &self,
        _req: Request<JobStatusUpdate>,
    ) -> Result<Response<Ack>, Status> {
        Ok(Response::new(Ack { ok: true, message: String::new() }))
    }
}
