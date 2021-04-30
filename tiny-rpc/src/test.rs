use futures::FutureExt;
use tokio::time::{timeout_at, Duration, Instant};
use tracing::subscriber;
use tracing_subscriber::{EnvFilter, FmtSubscriber};

#[tiny_rpc_macros::rpc_trait]
#[async_trait::async_trait]
pub trait Rpc {
    fn non_async_func(&self, magic: i32) -> bool;
    async fn delay(&self, delay_ms: u32) -> u32;
}

struct Impl;

#[async_trait::async_trait]
impl Rpc for Impl {
    fn non_async_func(&self, magic: i32) -> bool {
        magic == 42
    }

    async fn delay(&self, delay_ms: u32) -> u32 {
        tokio::time::sleep(Duration::from_millis(delay_ms as u64)).await;
        delay_ms
    }
}

#[tokio::test]
async fn basic_test() {
    let _guard = subscriber::set_default(
        FmtSubscriber::builder()
            .with_test_writer()
            .with_env_filter(EnvFilter::from_default_env())
            .finish(),
    );

    let now = Instant::now();
    let deadline = now.checked_add(Duration::from_secs(1)).unwrap();

    let (server_transport, client_transport) = crate::io::Transport::new_local();

    let server_driver_handle =
        tokio::spawn(RpcServer::serve(Impl, server_transport).map(|r| r.unwrap()));
    let (client, client_driver) = RpcClient::new(client_transport);
    let client_driver_handle = tokio::spawn(client_driver.map(|r| r.unwrap()));

    let client2 = client.clone();
    let client2_thread_handle = tokio::spawn(async move {
        client2.non_async_func(42).await.unwrap();
    });

    client.delay(5).await.unwrap();
    drop(client);

    timeout_at(deadline, client2_thread_handle)
        .await
        .unwrap()
        .unwrap();
    timeout_at(deadline, client_driver_handle)
        .await
        .unwrap()
        .unwrap();
    timeout_at(deadline, server_driver_handle)
        .await
        .unwrap()
        .unwrap();
}
