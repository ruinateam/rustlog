use crate::{
    ShutdownRx,
    app::App,
    storage::message::StructuredMessage,
    web::responders::logs::message::{BasicMessage, ResponseMessage},
};
use axum::{
    Extension,
    extract::{
        Query, State,
        ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade},
    },
    http::StatusCode,
    response::Response,
};
use futures::{SinkExt, StreamExt};
use prometheus::{
    IntCounter, IntCounterVec, IntGauge, register_int_counter, register_int_counter_vec,
    register_int_gauge,
};
use schemars::JsonSchema;
use serde::Deserialize;
use std::sync::LazyLock;
use std::time::Duration;
use tokio::{sync::broadcast, time::timeout};
use tracing::warn;

const FRAME_SEND_TIMEOUT_SECONDS: u64 = 5;

static CLIENTS: LazyLock<IntGauge> = LazyLock::new(|| {
    register_int_gauge!(
        "rustlog_firehose_clients_count",
        "Currently connected firehose clients"
    )
    .unwrap()
});

static FRAMES_SENT: LazyLock<IntCounterVec> = LazyLock::new(|| {
    register_int_counter_vec!(
        "rustlog_firehose_frames_sent_total",
        "Firehose frames sent to clients",
        &["format"]
    )
    .unwrap()
});

static LAGGED_MESSAGES: LazyLock<IntCounter> = LazyLock::new(|| {
    register_int_counter!(
        "rustlog_firehose_lagged_messages_total",
        "Messages skipped for firehose clients that could not keep up"
    )
    .unwrap()
});

static DISCONNECTS: LazyLock<IntCounterVec> = LazyLock::new(|| {
    register_int_counter_vec!(
        "rustlog_firehose_disconnects_total",
        "Firehose disconnects by bounded reason",
        &["reason"]
    )
    .unwrap()
});

#[derive(Debug, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "kebab-case")]
pub enum FirehoseFormat {
    #[default]
    Raw,
    JsonBasic,
}

impl FirehoseFormat {
    fn metric_label(&self) -> &'static str {
        match self {
            Self::Raw => "raw",
            Self::JsonBasic => "json_basic",
        }
    }
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct FirehoseQuery {
    #[serde(default)]
    pub format: FirehoseFormat,
}

pub async fn firehose(
    websocket: WebSocketUpgrade,
    State(app): State<App>,
    Extension(shutdown_rx): Extension<ShutdownRx>,
    Query(query): Query<FirehoseQuery>,
) -> Result<Response, StatusCode> {
    let format = query.format;
    let receiver = app.firehose_tx.subscribe();

    Ok(websocket.on_upgrade(move |socket| run_session(socket, receiver, shutdown_rx, format)))
}

async fn run_session(
    socket: WebSocket,
    mut receiver: broadcast::Receiver<StructuredMessage<'static>>,
    mut shutdown_rx: ShutdownRx,
    format: FirehoseFormat,
) {
    CLIENTS.inc();
    let (mut sender, mut client_messages) = socket.split();
    let mut reason = "client_closed";

    loop {
        tokio::select! {
            message = receiver.recv() => match message {
                Ok(message) => {
                    let payload = match encode_message(&message, &format) {
                        Ok(payload) => payload,
                        Err(err) => {
                            warn!("Could not encode firehose message: {err:#}");
                            close(&mut sender, 1011, "serialization error").await;
                            reason = "serialization_error";
                            break;
                        }
                    };

                    match timeout(
                        Duration::from_secs(FRAME_SEND_TIMEOUT_SECONDS),
                        sender.send(Message::Text(payload.into())),
                    ).await {
                        Ok(Ok(())) => FRAMES_SENT.with_label_values(&[format.metric_label()]).inc(),
                        Ok(Err(_)) => {
                            reason = "send_error";
                            break;
                        }
                        Err(_) => {
                            close(&mut sender, 1013, "send timeout; reconnect").await;
                            reason = "send_timeout";
                            break;
                        }
                    }
                }
                Err(broadcast::error::RecvError::Lagged(count)) => {
                    LAGGED_MESSAGES.inc_by(count);
                    close(&mut sender, 1013, "lagged; reconnect and replay").await;
                    reason = "lagged";
                    break;
                }
                Err(broadcast::error::RecvError::Closed) => {
                    reason = "publisher_closed";
                    break;
                }
            },
            client_message = client_messages.next() => match client_message {
                Some(Ok(Message::Close(_))) | None => break,
                Some(Ok(_)) => (),
                Some(Err(_)) => {
                    reason = "client_error";
                    break;
                }
            },
            _ = shutdown_rx.changed() => {
                close(&mut sender, 1001, "server shutdown").await;
                reason = "shutdown";
                break;
            }
        }
    }

    DISCONNECTS.with_label_values(&[reason]).inc();
    CLIENTS.dec();
}

fn encode_message(
    message: &StructuredMessage<'static>,
    format: &FirehoseFormat,
) -> anyhow::Result<String> {
    match format {
        FirehoseFormat::Raw => Ok(message.to_raw_irc()),
        FirehoseFormat::JsonBasic => Ok(serde_json::to_string(&BasicMessage::from_structured(
            message,
        )?)?),
    }
}

async fn close<S>(sender: &mut S, code: u16, reason: &'static str)
where
    S: futures::Sink<Message> + Unpin,
{
    let _ = timeout(
        Duration::from_secs(FRAME_SEND_TIMEOUT_SECONDS),
        sender.send(Message::Close(Some(CloseFrame {
            code,
            reason: reason.into(),
        }))),
    )
    .await;
}

#[cfg(test)]
mod tests {
    use super::{FirehoseFormat, encode_message};
    use crate::storage::message::{StructuredMessage, UnstructuredMessage};

    fn sample_message() -> StructuredMessage<'static> {
        StructuredMessage::from_unstructured(&UnstructuredMessage {
            channel_id: "1",
            user_id: "2",
            timestamp: 1_686_947_117_960,
            raw: "@id=0a4b7b50-052e-473e-99ee-441f05ce52a7;login=user;display-name=User;user-id=2;room-id=1;tmi-sent-ts=1686947117960 :user!user@user.tmi.twitch.tv PRIVMSG #channel :hello",
        })
        .unwrap()
        .into_owned()
    }

    #[test]
    fn raw_frames_do_not_append_crlf() {
        let payload = encode_message(&sample_message(), &FirehoseFormat::Raw).unwrap();
        assert!(!payload.ends_with("\r\n"));
    }

    #[test]
    fn json_basic_frames_are_individual_json_values() {
        let payload = encode_message(&sample_message(), &FirehoseFormat::JsonBasic).unwrap();
        let value: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(value["text"], "hello");
        assert_eq!(value["displayName"], "User");
    }
}
