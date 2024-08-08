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
use log::info;
use std::error::Error;
use std::future::Future;
use tokio::net::TcpStream;

use crate::constants::PUMP_PORTAL_WS_HOST;
use crate::constants::PUMP_PORTAL_WS_URL;
use crate::constants::{PUMP_WS_HOST, PUMP_WS_URL};

pub async fn connect_to_pump_portal_websocket(
) -> Result<WebSocket<TokioIo<Upgraded>>, Box<dyn Error>> {
    _connect_to_websocket(
        PUMP_PORTAL_WS_HOST.to_string(),
        PUMP_PORTAL_WS_URL.to_string(),
    )
    .await
}

#[timed::timed(duration(printer = "info!"))]
pub async fn _connect_to_websocket(
    host: String,
    url: String,
) -> Result<WebSocket<TokioIo<Upgraded>>, Box<dyn Error>> {
    let stream = TcpStream::connect(format!("{}:443", host)).await?;

    // Convert the TCP stream to a TLS stream
    let tls_connector =
        tokio_native_tls::native_tls::TlsConnector::new().unwrap();
    let tls_connector = tokio_native_tls::TlsConnector::from(tls_connector);
    let tls_stream = tls_connector.connect(&host, stream).await?;

    let req = Request::builder()
        .method("GET")
        .uri(url)
        .header("Host", host)
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

pub async fn connect_to_pump_websocket(
) -> Result<WebSocket<TokioIo<Upgraded>>, Box<dyn Error>> {
    _connect_to_websocket(PUMP_WS_HOST.to_string(), PUMP_WS_URL.to_string())
        .await
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
    use fastwebsockets::{Frame, OpCode, Payload};

    use super::*;

    async fn assert_connection(ws: &mut WebSocket<TokioIo<Upgraded>>) {
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
    async fn connect_works() {
        let mut ws = connect_to_pump_websocket().await.expect("connect");
        assert_connection(&mut ws).await;
    }

    #[tokio::test]
    async fn connect_to_pump_portal_works() {
        let mut ws =
            connect_to_pump_portal_websocket().await.expect("connect");
        let payload = r#"{"method":"subscribeNewToken"}"#;
        ws.write_frame(Frame::text(Payload::Bytes(payload.into())))
            .await
            .expect("write frame");
        assert_connection(&mut ws).await;
    }
}
