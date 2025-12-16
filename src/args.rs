use clap::{Parser, Subcommand};

#[derive(Parser)]
#[clap(author, version, about, long_about = None)]
pub struct Args {
    #[clap(subcommand)]
    pub subcommand: Option<Command>,
}

#[derive(Subcommand)]
pub enum Command {
    /// Migrate existing justlog logs
    Migrate {
        /// The justlog logs folder
        #[clap(short, long, value_parser)]
        source_dir: String,
        /// List of channel ids to migrate (None specified = migrate all)
        #[clap(short, long, value_parser)]
        channel_id: Vec<String>,
        /// Parallel migration jobs
        #[clap(short, long, default_value_t = 1)]
        jobs: usize,
    },
    /// Mirror remote rustlog/justlog JSON API into local ClickHouse
    Mirror {
        /// Remote base URL (e.g. https://logs.zonian.dev)
        #[clap(long, default_value = "https://logs.zonian.dev")]
        base_url: String,
        /// Use local cache instead of HTTP (path to cache root, e.g. /mnt/c/.../twitchlogs/cache)
        #[clap(long)]
        local_cache: Option<String>,
        /// Channel login to mirror
        #[clap(long)]
        channel: String,
        /// Optional year filter (YYYY)
        #[clap(long)]
        year: Option<u32>,
        /// Optional month filter (1-12)
        #[clap(long)]
        month: Option<u32>,
        /// Optional day filter (1-31)
        #[clap(long)]
        day: Option<u32>,
        /// Batch size for ClickHouse inserts
        #[clap(long, default_value_t = 1000)]
        batch: usize,
    },
}
