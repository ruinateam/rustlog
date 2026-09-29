//! A task that inserts the imported messages in batches.

use crate::storage::message::{MESSAGES_STRUCTURED_TABLE, StructuredMessage};
use anyhow::Context;
use clickhouse::Client;
use tokio::{sync::mpsc, task::JoinHandle};
use tracing::{debug, info};

/// Messages waiting for the writer; senders wait when it is full.
const QUEUE_SIZE: usize = 100_000;

pub struct Writer {
    pub sender: mpsc::Sender<StructuredMessage<'static>>,
    task: JoinHandle<anyhow::Result<u64>>,
}

impl Writer {
    /// Starts inserting whatever is sent, `batch_rows` rows per insert.
    pub fn spawn(db: Client, batch_rows: u64) -> Self {
        let (sender, receiver) = mpsc::channel(QUEUE_SIZE);
        let task = tokio::spawn(write(db, batch_rows, receiver));
        Self { sender, task }
    }

    /// Waits until everything sent is inserted; returns the row count.
    pub async fn finish(self) -> anyhow::Result<u64> {
        drop(self.sender);
        self.task.await.context("the writer task panicked")?
    }
}

async fn write(
    db: Client,
    batch_rows: u64,
    mut receiver: mpsc::Receiver<StructuredMessage<'static>>,
) -> anyhow::Result<u64> {
    let mut inserter = db
        .inserter::<StructuredMessage<'static>>(MESSAGES_STRUCTURED_TABLE)
        .with_max_rows(batch_rows.max(1));
    let mut written = 0_u64;

    while let Some(message) = receiver.recv().await {
        inserter
            .write(&message)
            .await
            .with_context(|| format!("could not write a message after {written} rows"))?;
        written += 1;

        let committed = inserter
            .commit()
            .await
            .with_context(|| format!("could not insert a batch after {written} rows"))?;
        if committed.rows > 0 {
            info!(rows = written, "inserted a batch");
        }
    }

    debug!("inserting the last batch");
    inserter
        .end()
        .await
        .with_context(|| format!("could not insert the last batch of {written} rows"))?;
    info!(rows = written, "writer finished");
    Ok(written)
}
