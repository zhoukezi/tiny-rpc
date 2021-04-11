use std::time::Duration;

use futures::StreamExt;

tiny_rpc_macros::rpc_define! {
    #[async_trait::async_trait]
    pub trait ProcMacroRpc {
        fn hello(&self,name: String) -> String;
        async fn delay(&self) -> String;
        fn p(&self,t:(u32,u32));
    }
}

#[tiny_rpc_macros::rpc_trait]
#[async_trait::async_trait]
pub trait AttrRpc {
    fn non_async_func(&self, magic: i32) -> bool;
    async fn delay(&self, delay_ms: u32) -> u32;
}

#[tokio::test]
async fn basic_test() {
    struct Impl;

    #[async_trait::async_trait]
    impl AttrRpc for Impl {
        fn non_async_func(&self, magic: i32) -> bool {
            magic == 42
        }

        async fn delay(&self, delay_ms: u32) -> u32 {
            tokio::time::sleep(Duration::from_millis(delay_ms as u64)).await;
            delay_ms
        }
    }

    use tracing::subscriber;
    use tracing_subscriber::{EnvFilter, FmtSubscriber};

    let _guard = subscriber::set_default(
        FmtSubscriber::builder()
            .with_test_writer()
            .with_env_filter(EnvFilter::from_default_env())
            .finish(),
    );

    let (server_transport, client_transport) = crate::io::Transport::new_local();

    let join = tokio::spawn(async move {
        let mut spawn_stream = AttrRpcServer::serve(Impl, server_transport);
        while let Some(fut) = spawn_stream.next().await {
            tokio::spawn(fut);
        }
    });
    let (client, driver) = AttrRpcClient::new(client_transport);
    let join2 = tokio::spawn(driver);

    let client2 = client.clone();
    tokio::spawn(async move {
        client2.non_async_func(42).await.unwrap();
    });

    client.delay(1000).await.unwrap();

    drop(client);

    join.await.unwrap();
    join2.await.unwrap();
}
