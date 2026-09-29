use super::message::StructuredMessage;
use crate::{ShutdownRx, state::OperationalState, storage::message::MESSAGES_STRUCTURED_TABLE};
use anyhow::{Context, anyhow};
use clickhouse::Client;
use prometheus::{IntGauge, register_int_gauge};
use std::sync::LazyLock;
use std::{ops::Range, sync::Arc, time::Duration};
use tokio::{
    sync::{
        RwLock,
        mpsc::{Sender, channel},
    },
    task::JoinHandle,
    time::{Instant, sleep},
};
use tracing::{debug, error, info, trace};

const RETRY_COUNT: usize = 20;
const RETRY_INTERVAL_SECONDS: u64 = 5;

static BATCH_MESSAGE_COUNT_GAUGE: LazyLock<IntGauge> = LazyLock::new(|| {
    register_int_gauge!(
        "rustlog_messages_written_per_batch",
        "How many messages are written to the database per batch"
    )
    .unwrap()
});

#[derive(Default, Clone)]
pub struct FlushBuffer {
    messages: Arc<RwLock<Vec<StructuredMessage<'static>>>>,
}

impl FlushBuffer {
    pub async fn messages_by_channel(
        &self,
        time_range: Range<u64>,
        channel_id: &str,
    ) -> Vec<StructuredMessage<'static>> {
        let msgs = self
            .messages
            .read()
            .await
            .iter()
            .filter(|msg| time_range.contains(&msg.timestamp))
            .filter(|msg| msg.channel_id == channel_id)
            .cloned()
            .collect::<Vec<_>>();
        trace!(messages = msgs.len(), "Read messages from the write buffer");
        msgs
    }

    pub async fn messages_by_channel_and_user(
        &self,
        time_range: Range<u64>,
        channel_id: &str,
        user_id: &str,
    ) -> Vec<StructuredMessage<'static>> {
        let msgs = self
            .messages
            .read()
            .await
            .iter()
            .filter(|msg| time_range.contains(&msg.timestamp))
            .filter(|msg| msg.channel_id == channel_id && msg.user_id == user_id)
            .cloned()
            .collect::<Vec<_>>();
        trace!(messages = msgs.len(), "Read messages from the write buffer");
        msgs
    }

    pub async fn remove_user(&self, user_id: &str) {
        let mut messages = self.messages.write().await;
        messages.retain(|message| message.user_id != user_id);
    }
}

pub async fn create_writer(
    db: Arc<Client>,
    mut shutdown_rx: ShutdownRx,
    flush_interval: u64,
    state: OperationalState,
) -> anyhow::Result<(
    Sender<StructuredMessage<'static>>,
    FlushBuffer,
    JoinHandle<()>,
)> {
    let (tx, mut rx) = channel::<StructuredMessage<'static>>(1000);

    let flush_buffer = FlushBuffer::default();
    let flush_buffer_clone = flush_buffer.clone();

    let handle = tokio::spawn(async move {
        let timeout = tokio::time::sleep(Duration::from_secs(flush_interval));
        tokio::pin!(timeout);

        loop {
            tokio::select! {
                _ = &mut timeout => {
                    timeout.as_mut().reset(Instant::now() + Duration::from_secs(flush_interval));
                    if let Err(err) = write_chunk_with_retry(&db, &flush_buffer, &state).await {
                        error!(error = format!("{err:#}"), "Could not write messages");
                    }
                }
                Some(msg) = rx.recv() => {
                    if state.is_loggable(&msg.channel_id, &msg.user_id) {
                        flush_buffer.messages.write().await.push(msg);
                    }
                }
                Ok(()) = shutdown_rx.changed() => {
                    info!("Flushing the write buffer");

                    if let Err(err) = write_chunk_with_retry(&db, &flush_buffer, &state).await {
                        error!(error = format!("{err:#}"), "Could not flush the write buffer");
                    }

                    break;
                }
            }
        }
    });

    Ok((tx, flush_buffer_clone, handle))
}

async fn write_chunk_with_retry(
    db: &Client,
    buffer: &FlushBuffer,
    state: &OperationalState,
) -> anyhow::Result<()> {
    for attempt in 1..=RETRY_COUNT {
        match write_chunk(db, buffer, state).await {
            Ok(()) => {
                if attempt > 1 {
                    debug!(attempt, "Insert succeeded after retrying");
                }
                return Ok(());
            }
            Err(err) => {
                error!(
                    attempt,
                    max_attempts = RETRY_COUNT,
                    retry_in_secs = RETRY_INTERVAL_SECONDS,
                    error = format!("{err:#}"),
                    "Could not insert messages"
                );
                sleep(Duration::from_secs(RETRY_INTERVAL_SECONDS)).await;
            }
        }
    }
    Err(anyhow!(
        "Inserting failed even after {RETRY_COUNT} attempts"
    ))
}

async fn write_chunk(
    db: &Client,
    buffer: &FlushBuffer,
    state: &OperationalState,
) -> anyhow::Result<()> {
    let messages_read_guard = buffer.messages.read().await;
    let messages = messages_read_guard
        .iter()
        .filter(|message| state.is_loggable(&message.channel_id, &message.user_id))
        .cloned()
        .collect::<Vec<_>>();
    drop(messages_read_guard);

    if messages.is_empty() {
        buffer.messages.write().await.clear();
        BATCH_MESSAGE_COUNT_GAUGE.set(0);
        return Ok(());
    }

    let started_at = Instant::now();

    let mut insert = db
        .insert::<StructuredMessage<'static>>(MESSAGES_STRUCTURED_TABLE)
        .await?;
    for message in &messages {
        insert.write(message).await.context("Could not write row")?;
    }

    let mut messages_write_guard = buffer.messages.write().await;
    insert.end().await.context("Could not end insert")?;

    debug!(
        messages = messages.len(),
        took_ms = started_at.elapsed().as_millis() as u64,
        "Inserted messages"
    );
    BATCH_MESSAGE_COUNT_GAUGE.set(messages.len().try_into().unwrap());
    messages_write_guard.clear();

    Ok(())
}
