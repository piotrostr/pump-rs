use std::error::Error;

use fastwebsockets::{Frame, OpCode, Payload};
use log::{info, warn};

use crate::snipe::{get_message_type, MessageType, NewCoin};
use crate::snipe_portal::NewPumpPortalToken;
use crate::ws::{
    connect_to_pump_portal_websocket, connect_to_pump_websocket,
};

pub async fn bench_pump_connection() -> Result<(), Box<dyn Error>> {
    let mut ws = connect_to_pump_websocket().await?;
    ws.set_writev(true);

    ws.write_frame(Frame::text(Payload::Borrowed(b"40".as_ref())))
        .await?;
    loop {
        let frame = ws.read_frame().await?;
        match frame.opcode {
            OpCode::Close => {
                warn!("received close opcode");
                break;
            }
            OpCode::Text => {
                let data = String::from_utf8(frame.payload.to_vec())?;
                match data.as_str() {
                    "2" => {
                        ws.write_frame(Frame::text(Payload::Borrowed(
                            "3".as_ref(),
                        )))
                        .await?;
                        info!("Heartbeat sent");
                    }
                    _ => {
                        if let MessageType::NewCoinCreated =
                            get_message_type(&data)
                        {
                            let timestamp_now_ms =
                                chrono::Utc::now().timestamp_millis();
                            info!("{} got the msg", timestamp_now_ms);
                            let coin: NewCoin = serde_json::from_str(
                                data.trim_start_matches(
                                    r#"42["newCoinCreated","#,
                                )
                                .trim_end_matches(']'),
                            )?;
                            println!(
                                "{}: {}",
                                coin.mint,
                                timestamp_now_ms as u64
                                    - coin.created_timestamp
                            );
                        }
                    }
                }
            }
            _ => {}
        }
    }

    Ok(())
}

pub async fn bench_pump_portal_connection() -> Result<(), Box<dyn Error>> {
    let mut ws = connect_to_pump_portal_websocket().await?;
    ws.set_writev(true);

    let sub = r#"{"method":"subscribeNewToken"}"#;
    ws.write_frame(Frame::text(Payload::Bytes(sub.into())))
        .await
        .expect("write frame");

    loop {
        let frame = ws.read_frame().await?;
        match frame.opcode {
            OpCode::Close => {
                warn!("received close opcode");
                break;
            }
            OpCode::Text => {
                let data = String::from_utf8(frame.payload.to_vec())?;
                if data.contains("Successfully subscribed") {
                    continue;
                }
                let token =
                    serde_json::from_str::<NewPumpPortalToken>(&data)?;
                let timestamp_now_ms = chrono::Utc::now().timestamp_millis();
                println!("{}: {}", token.mint, timestamp_now_ms);
            }
            _ => {}
        }
    }

    Ok(())
}
