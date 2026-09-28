use super::migratable::Migratable;
use crate::{config::Config, state::migrate_legacy_config};

pub struct StateMigration<'a> {
    pub config: &'a Config,
}

impl<'a> Migratable<'a> for StateMigration<'a> {
    async fn run(&self, db: &'a clickhouse::Client) -> anyhow::Result<()> {
        migrate_legacy_config(db, self.config).await
    }
}
