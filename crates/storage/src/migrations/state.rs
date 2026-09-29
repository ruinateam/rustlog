use super::migratable::Migratable;
use crate::state::{LegacyConfig, migrate_legacy_config};

pub struct StateMigration<'a> {
    pub legacy: &'a LegacyConfig,
}

impl<'a> Migratable<'a> for StateMigration<'a> {
    async fn run(&self, db: &'a clickhouse::Client) -> anyhow::Result<()> {
        migrate_legacy_config(db, self.legacy).await
    }
}
