use clap::{Parser, Subcommand};
use rustlog::tools::{
    duplicates::CleanupDuplicateIdsOptions, fill_missing::FillMissingOptions,
    migrate::MigrateOptions, mirror::MirrorOptions,
};
use std::path::PathBuf;

#[derive(Parser)]
#[clap(author, version, about, long_about = None)]
pub struct Args {
    /// Path to the config file
    #[clap(default_value = "config.json", long = "config")]
    pub config_path: PathBuf,
    #[clap(subcommand)]
    pub subcommand: Option<Command>,
}

#[derive(Subcommand)]
pub enum Command {
    /// Write the OpenAPI documents of the HTTP API; needs neither a config nor ClickHouse
    Openapi {
        /// Directory to write `legacy.json` and `v2.json` into
        #[clap(default_value = "docs/openapi")]
        out_dir: PathBuf,
    },
    /// Migrate existing justlog logs
    Migrate(MigrateOptions),
    /// Mirror the JSON API of a remote rustlog or justlog instance into ClickHouse
    Mirror(MirrorOptions),
    /// Fill missing local days from the mirrors listed by logs.zonian.dev/api/<channel>
    FillMissing(FillMissingOptions),
    /// Find or remove duplicate rows by Twitch message id
    CleanupDuplicateIds(CleanupDuplicateIdsOptions),
}
