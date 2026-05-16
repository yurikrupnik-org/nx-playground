use rpc::tasks::tasks_service_client::TasksServiceClient;
use tonic::transport::Channel;
use tonic_health::pb::health_client::HealthClient;

/// Lazy gRPC channel + the clients that share it.
///
/// The channel is built lazily: no TCP/HTTP-2 handshake happens at construction,
/// so the api can boot independently of the tasks service. Tonic establishes
/// the connection on the first RPC and auto-reconnects with backoff if it
/// drops. Readiness is signalled separately via the `/ready` health endpoint
/// using `health_client`, which exercises the same channel.
pub struct TasksClients {
    pub tasks: TasksServiceClient<Channel>,
    pub health: HealthClient<Channel>,
}

pub fn create_optimized_tasks_clients(addr: String) -> eyre::Result<TasksClients> {
    let channel = grpc_client::create_channel_lazy(addr)?;

    let tasks = TasksServiceClient::new(channel.clone())
        .accept_compressed(tonic::codec::CompressionEncoding::Zstd)
        .send_compressed(tonic::codec::CompressionEncoding::Zstd)
        .max_decoding_message_size(8 * 1024 * 1024)
        .max_encoding_message_size(8 * 1024 * 1024);

    let health = HealthClient::new(channel);

    Ok(TasksClients { tasks, health })
}
