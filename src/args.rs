use clap::{Parser, Subcommand};

#[derive(Parser)]
#[clap(author, version, about, long_about = None)]
pub struct Args {
    /// Path to the config file
    #[clap(default_value = "config.json", long = "config")]
    pub config_path: std::path::PathBuf,
    #[clap(subcommand)]
    pub subcommand: Option<Command>,
}

#[derive(Subcommand)]
pub enum Command {
    /// Write the OpenAPI documents of the HTTP API; needs neither a config nor ClickHouse
    Openapi {
        /// Directory to write `legacy.json` and `v2.json` into
        #[clap(default_value = "docs/openapi")]
        out_dir: std::path::PathBuf,
    },
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
        #[clap(long, default_value_t = 25_000)]
        batch: usize,
        /// Number of days to fetch in parallel
        #[clap(long, default_value_t = 4)]
        http_concurrency: usize,
        /// HTTP(S) proxy for remote mirror requests. Repeatable for a proxy pool.
        #[clap(long)]
        proxy: Vec<String>,
        /// Max requests per second per proxy slot (direct counts as one slot)
        #[clap(long, default_value_t = 2.0)]
        rps: f64,
    },
    /// Fill missing local days from mirrors listed by logs.zonian.dev/api/<channel>
    FillMissing {
        /// Channel login to fill. Repeatable.
        #[clap(long, required = true)]
        channel: Vec<String>,
        /// Year to fill (YYYY)
        #[clap(long)]
        year: u32,
        /// Zonian API base URL
        #[clap(long, default_value = "https://logs.zonian.dev")]
        api_base: String,
        /// Batch size for ClickHouse inserts
        #[clap(long, default_value_t = 25_000)]
        batch: usize,
        /// Number of days to fetch/check in parallel
        #[clap(long, default_value_t = 4)]
        http_concurrency: usize,
        /// HTTP(S) proxy for remote mirror requests. Repeatable for a proxy pool.
        #[clap(long)]
        proxy: Vec<String>,
        /// Max requests per second per proxy slot (direct counts as one slot)
        #[clap(long, default_value_t = 2.0)]
        rps: f64,
        /// Mirror base URL to skip. Repeatable.
        #[clap(long)]
        exclude_instance: Vec<String>,
        /// Print missing days without importing
        #[clap(long)]
        dry_run: bool,
        /// Re-check days that already exist locally and re-mirror days where a mirror has more message ids
        #[clap(long)]
        repair_existing: bool,
        /// In repair mode, compare all mirrors instead of using the first working mirror
        #[clap(long)]
        deep: bool,
    },
    /// Find or remove duplicate rows by Twitch message id
    CleanupDuplicateIds {
        /// Limit to channel login. Repeatable.
        #[clap(long)]
        channel: Vec<String>,
        /// Limit by UTC year (YYYY)
        #[clap(long)]
        year: Option<u32>,
        /// Actually rewrite duplicate ids. Without this flag only prints a dry-run summary.
        #[clap(long)]
        execute: bool,
        /// Number of duplicate examples to print
        #[clap(long, default_value_t = 30)]
        sample_limit: usize,
        /// Seconds to wait for ClickHouse delete mutations
        #[clap(long, default_value_t = 600)]
        wait_timeout: u64,
    },
}
