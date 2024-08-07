use fastwebsockets::handshake;
use fastwebsockets::WebSocket;
use http_body_util::Empty;
use hyper::{
    body::Bytes,
    header::{CONNECTION, UPGRADE},
    upgrade::Upgraded,
    Request,
};
use hyper_util::rt::TokioIo;
use std::error::Error;
use std::future::Future;
use tokio::net::TcpStream;

use crate::constants::{PUMP_WS_HOST, PUMP_WS_URL};

pub async fn connect_to_pump_websocket(
) -> Result<WebSocket<TokioIo<Upgraded>>, Box<dyn Error>> {
    let stream = TcpStream::connect(PUMP_WS_HOST).await?;

    // Convert the TCP stream to a TLS stream
    let tls_connector =
        tokio_native_tls::native_tls::TlsConnector::new().unwrap();
    let tls_connector = tokio_native_tls::TlsConnector::from(tls_connector);
    let tls_stream = tls_connector
        .connect("frontend-api.pump.fun", stream)
        .await?;

    let req = Request::builder()
        .method("GET")
        .uri(PUMP_WS_URL)
        .header("Host", "frontend-api.pump.fun")
        .header(UPGRADE, "websocket")
        .header(CONNECTION, "upgrade")
        .header(
            "Sec-WebSocket-Key",
            fastwebsockets::handshake::generate_key(),
        )
        .header("Sec-WebSocket-Version", "13")
        .body(Empty::<Bytes>::new())?;

    let (ws, _) = handshake::client(&SpawnExecutor, req, tls_stream).await?;
    Ok(ws)
}

// Tie hyper's executor to tokio runtime
struct SpawnExecutor;

impl<Fut> hyper::rt::Executor<Fut> for SpawnExecutor
where
    Fut: Future + Send + 'static,
    Fut::Output: Send + 'static,
{
    fn execute(&self, fut: Fut) {
        tokio::task::spawn(fut);
    }
}

#[cfg(test)]
mod tests {
    use fastwebsockets::OpCode;

    use super::*;

    #[tokio::test]
    async fn connect_works() {
        let mut ws = connect_to_pump_websocket().await.expect("connect");
        let frame = ws.read_frame().await.expect("read frame");
        let mut pass = false;
        match frame.opcode {
            OpCode::Close => {}
            OpCode::Text | OpCode::Binary => {
                pass = true;
            }
            _ => {}
        }
        assert!(pass);
    }

    #[tokio::test]
    async fn listen_pump_works() {
        // match listen_and_snipe().await {
        //     Ok(_) => println!("Listening completed successfully"),
        //     Err(e) => eprintln!("Listening failed: {}", e),
        // }
    }
}
