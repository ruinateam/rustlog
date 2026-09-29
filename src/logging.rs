//! Log output: readable text or JSON lines on stdout, and optionally the
//! same events in daily rotated files.

use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::{
    env,
    io::{self, IsTerminal},
    path::PathBuf,
};
use tracing_appender::{
    non_blocking::WorkerGuard,
    rolling::{RollingFileAppender, Rotation},
};
use tracing_subscriber::{
    EnvFilter, Layer, Registry, layer::SubscriberExt, util::SubscriberInitExt,
};

/// The `logging` section of the config file.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoggingConfig {
    /// Which events to log, in `RUST_LOG` syntax such as `info` or
    /// `info,rustlog=debug`. `RUST_LOG` takes precedence when set.
    #[serde(default = "default_filter")]
    pub filter: String,
    #[serde(default)]
    pub format: LogFormat,
    /// Also write the logs to daily rotated files.
    #[serde(default)]
    pub file: Option<LogFileConfig>,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            filter: default_filter(),
            format: LogFormat::default(),
            file: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogFormat {
    /// Human-readable lines with `key=value` fields.
    #[default]
    Text,
    /// One JSON object per event.
    Json,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogFileConfig {
    /// Directory for the `rustlog.YYYY-MM-DD.log` files.
    pub directory: PathBuf,
    /// How many daily files to keep; older ones are deleted.
    #[serde(default = "default_max_files")]
    pub max_files: usize,
}

fn default_filter() -> String {
    "info".to_owned()
}

fn default_max_files() -> usize {
    14
}

/// Installs the global logger and a panic hook that logs panics.
///
/// The returned guard flushes the log file on drop, so keep it alive until
/// the program exits.
pub fn init(config: &LoggingConfig) -> anyhow::Result<Option<WorkerGuard>> {
    let filter = match env::var("RUST_LOG") {
        Ok(filter) => EnvFilter::try_new(&filter).context("Invalid RUST_LOG filter")?,
        Err(_) => EnvFilter::try_new(&config.filter).context("Invalid logging filter")?,
    };

    let mut outputs = vec![format_layer(config.format, io::stdout, use_colors())];

    let guard = match &config.file {
        Some(file) => {
            // The appender lists the directory to prune old files before it
            // creates it, and complains on stderr if it is missing.
            std::fs::create_dir_all(&file.directory).with_context(|| {
                format!(
                    "Could not create log directory {}",
                    file.directory.display()
                )
            })?;
            let appender = RollingFileAppender::builder()
                .rotation(Rotation::DAILY)
                .filename_prefix("rustlog")
                .filename_suffix("log")
                .max_log_files(file.max_files)
                .build(&file.directory)
                .with_context(|| {
                    format!("Could not open log files in {}", file.directory.display())
                })?;
            let (writer, guard) = tracing_appender::non_blocking(appender);
            outputs.push(format_layer(config.format, writer, false));
            Some(guard)
        }
        None => None,
    };

    tracing_subscriber::registry()
        .with(outputs)
        .with(filter)
        .try_init()
        .context("Could not install the logger")?;

    std::panic::set_hook(Box::new(tracing_panic::panic_hook));

    Ok(guard)
}

type BoxedLayer = Box<dyn Layer<Registry> + Send + Sync>;

fn format_layer<W>(format: LogFormat, writer: W, colors: bool) -> BoxedLayer
where
    W: for<'writer> tracing_subscriber::fmt::MakeWriter<'writer> + Send + Sync + 'static,
{
    let layer = tracing_subscriber::fmt::layer().with_writer(writer);
    match format {
        LogFormat::Text => layer.with_ansi(colors).boxed(),
        LogFormat::Json => layer
            .json()
            .with_current_span(true)
            .with_span_list(false)
            .boxed(),
    }
}

/// Colors when stdout is a terminal, unless `NO_COLOR` is set.
/// `RUST_LOG_ANSI=true|false` overrides both.
fn use_colors() -> bool {
    if let Some(ansi) = env::var("RUST_LOG_ANSI")
        .ok()
        .and_then(|ansi| ansi.parse().ok())
    {
        return ansi;
    }
    env::var_os("NO_COLOR").is_none() && io::stdout().is_terminal()
}
