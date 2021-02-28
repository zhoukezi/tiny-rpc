use futures::{SinkExt, StreamExt};
use tiny_rpc_macros::rpc_define;

rpc_define! {
    #[async_trait::async_trait]
    pub trait Hello {
        fn hello(name: String) -> String;
        async fn delay(&self) -> String;
    }
}

pub async fn run_example() {
    struct HelloImpl(String);
    #[async_trait::async_trait]
    impl HelloServerImpl for HelloImpl {
        fn hello(name: String) -> String {
            format!("Hello, {}!", name)
        }

        async fn delay(&self) -> String {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            format!("Hello, client at {}!", self.0)
        }
    }

    let (ctx, srx) = futures::channel::mpsc::channel::<(crate::rpc::RequestId, HelloRequest)>(128);
    let (stx, crx) = futures::channel::mpsc::channel::<(crate::rpc::RequestId, HelloResponse)>(128);
    let ctx = ctx.sink_map_err(|_| panic!());
    let stx = stx.sink_map_err(|_| panic!());
    let srx = srx.map(|e| Ok(e));
    let crx = crx.map(|e| Ok(e));

    tokio::spawn(async move {
        info!("server task spawned");

        tiny_rpc::rpc::serve(
            HelloServer::new(HelloImpl(String::from("local pipe"))),
            (srx, stx),
        )
        .await
        .unwrap();
        info!("server drop");
    });

    // construct a client and call hello
    info!("create stub");
    let mut stub1 = HelloStub::new((crx, ctx));
    let mut stub2 = stub1.clone();

    let join1 = tokio::spawn(async move {
        for _ in 0..128 {
            info!("call hello ...");
            let res = stub1.hello("world".to_owned()).await.unwrap();
            info!("... with return value {}", res);
            assert_eq!(res, "Hello, world!");
        }
    });
    let join2 = tokio::spawn(async move {
        info!("call delay ...");
        let res = stub2.delay().await.unwrap();
        info!("... with return value {}", res);
        assert_eq!(res, "Hello, client at local pipe!");
    });
    join1.await.unwrap();
    join2.await.unwrap();
}

#[tokio::test]
async fn example_test() {
    use tracing::subscriber;
    use tracing_subscriber::{EnvFilter, FmtSubscriber};

    let _guard = subscriber::set_default(
        FmtSubscriber::builder()
            .with_test_writer()
            .with_env_filter(EnvFilter::from_default_env())
            .finish(),
    );

    run_example().await
}
