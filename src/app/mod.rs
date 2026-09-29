use crate::{
    config::Config,
    domain::opt_out::OptedOut,
    services::{sully::SullyGnome, tiers::Tiers},
    state::OperationalState,
    storage::{logs::delete_user_logs, message::StructuredMessage, writer::FlushBuffer},
    twitch::Twitch,
};
use anyhow::Context;
use dashmap::DashSet;
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::info;

/// Everything the HTTP handlers and the bot share.
#[derive(Clone)]
pub struct App {
    pub twitch: Twitch,
    pub sully: SullyGnome,
    pub tiers: Tiers,
    pub optout_codes: Arc<DashSet<String>>,
    pub db: Arc<clickhouse::Client>,
    pub config: Arc<Config>,
    pub state: OperationalState,
    pub flush_buffer: FlushBuffer,
    pub firehose_tx: broadcast::Sender<StructuredMessage<'static>>,
}

impl App {
    pub async fn optout_user(&self, user_id: &str) -> anyhow::Result<()> {
        self.state.optout_user(user_id).await?;
        self.flush_buffer.remove_user(user_id).await;
        delete_user_logs(&self.db, user_id)
            .await
            .context("could not delete logs")?;

        info!(user_id, "user opted out");

        Ok(())
    }

    pub fn check_opted_out(&self, channel_id: &str, user_id: Option<&str>) -> Result<(), OptedOut> {
        if self.state.is_channel_opted_out(channel_id) {
            return Err(OptedOut::Channel);
        }

        if let Some(user_id) = user_id
            && self.state.is_user_opted_out(user_id)
        {
            return Err(OptedOut::User);
        }

        Ok(())
    }

    pub fn check_user_opted_out(&self, user_id: &str) -> Result<(), OptedOut> {
        if self.state.is_user_opted_out(user_id) {
            return Err(OptedOut::User);
        }

        Ok(())
    }
}
