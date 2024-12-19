use rama::{
    graceful::Shutdown,
    layer::ConsumeErrLayer,
    net::forwarded::Forwarded,
    net::stream::{SocketInfo, Stream},
    proxy::haproxy::{
        client::HaProxyLayer as HaProxyClientLayer, server::HaProxyLayer as HaProxyServerLayer,
    },
    service::service_fn,
    tcp::{
        client::service::{Forwarder, TcpConnector},
        server::TcpListener,
    },
    tls::boring::server::{ TlsAcceptorData, TlsAcceptorLayer},
    Context, Layer,
    
};
use rama::net::tls::server::{SelfSignedData, ServerAuth, ServerConfig};

// everything else is provided by the standard library, community crates or tokio
use std::{convert::Infallible, time::Duration};
use tokio::io::AsyncWriteExt;
use tracing::metadata::LevelFilter;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

async fn internal_tcp_service_fn<S>(ctx: Context<()>, mut stream: S) -> Result<(), Infallible>
where
    S: Stream + Unpin,
    {

        let client_addr = ctx
            .get::<Forwarded>()
            .unwrap()
            .client_socket_addr()
            .unwrap();
        
        let proxy_addr = ctx.get::<SocketInfo>().unwrap().peer_addr();
        
        let payload = format!(
            "hello client {client_addr}, you were served by tls terminator proxy {proxy_addr}\r\n"
        );

        let response = format!(
            "HTTP/1.0 200 ok\r\n\
                                Connection: close\r\n\
                                Content-length: {}\r\n\
                                \r\n\
                                {}",
            payload.len(),
            payload
        );
    
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write to stream");
    
        Ok(())
        
    }

#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(
            EnvFilter::builder()
                .with_default_directive(LevelFilter::DEBUG.into())
                .from_env_lossy(),
            )
        .init();

    let tls_server_config = ServerConfig::new(ServerAuth::SelfSigned(SelfSignedData::default()));
    
    let acceptor_data = TlsAcceptorData::try_from(tls_server_config).expect("create acceptor data");

    let shutdown = Shutdown::default();

    shutdown.spawn_task_fn(|guard| async move {
        let tcp_service = TlsAcceptorLayer::new(acceptor_data).layer(
            Forwarder::new(([127,0,0,1], 62800)).connector(
                HaProxyClientLayer::tcp().layer(TcpConnector::new()),
            ),
        );

        TcpListener::bind("127.0.0.1:63800")
            .await
            .expect("bind TCP Listener: tls")
            .serve_graceful(guard, tcp_service)
            .await;
    });

    shutdown.spawn_task_fn(|guard| async {
        let tcp_service = (ConsumeErrLayer::default(), HaProxyServerLayer::new())
            .layer(service_fn(internal_tcp_service_fn));

        TcpListener::bind("127.0.0.1:62800")
            .await
            .expect("bind TCP Listerner: http")
            .serve_graceful(guard, tcp_service)
            .await;    
    });

    shutdown
        .shutdown_with_limit(Duration::from_secs(30))
        .await
        .expect("graceful shutdown");


    }