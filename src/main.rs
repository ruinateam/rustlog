mod args;
mod bot;

use anyhow::{Context, anyhow};
use args::{Args, Command};
use clap::Parser;
use futures::future::try_join_all;
#[cfg(unix)]
use futures::{StreamExt, stream::FuturesUnordered};
use mimalloc::MiMalloc;
use rustlog_app::{
    App,
    config::Config,
    logging::{self, LoggingConfig},
    services::{sully::SullyGnome, tiers::Tiers},
};
use rustlog_storage::{setup_db, state::OperationalState, writer::create_writer};
use rustlog_twitch::Twitch;
use std::{
    fs,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};
#[cfg(unix)]
use tokio::signal::unix::{SignalKind, signal};
use tokio::{
    sync::{broadcast, mpsc, watch},
    time::timeout,
};
use tracing::info;
use twitch_irc::login::StaticLoginCredentials;

const SHUTDOWN_TIMEOUT_SECONDS: u64 = 8;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    if let Some(Command::Openapi { out_dir }) = &args.subcommand {
        let _log_guard = logging::init(&LoggingConfig::default())?;
        return write_openapi(out_dir);
    }

    let config = Config::load(&args.config_path)?;
    // Keeps the log file writer alive until `main` returns.
    let _log_guard = logging::init(&config.logging)?;
    let mut db = clickhouse::Client::default()
        .with_url(&config.clickhouse_url)
        .with_database(&config.clickhouse_db)
        .with_compression(clickhouse::Compression::None);

    if let Some(user) = &config.clickhouse_username {
        db = db.with_user(user);
    }

    if let Some(password) = &config.clickhouse_password {
        db = db.with_password(password);
    }

    setup_db(&db, &config.clickhouse_db, &config.legacy_state())
        .await
        .context("could not run DB migrations")?;

    match args.subcommand {
        None => run(config, db).await,
        Some(Command::Openapi { .. }) => unreachable!("handled before loading the config"),
        Some(Command::Migrate(options)) => rustlog_tools::migrate::run(db, options).await,
        Some(Command::Mirror(options)) => rustlog_tools::mirror::run(db, options).await,
        Some(Command::FillMissing(options)) => rustlog_tools::fill_missing::run(db, options).await,
        Some(Command::CleanupDuplicateIds(options)) => {
            rustlog_tools::duplicates::run(db, options).await
        }
    }
}

async fn run(config: Config, db: clickhouse::Client) -> anyhow::Result<()> {
    let mut shutdown_rx = listen_shutdown().await;

    let config = Arc::new(config);

    let db = Arc::new(db);
    let state = OperationalState::load(db.clone())
        .await
        .context("could not load operational state")?;

    let (writer_tx, flush_buffer, mut writer_handle) = create_writer(
        db.clone(),
        shutdown_rx.clone(),
        config.clickhouse_flush_interval,
        state.clone(),
    )
    .await?;
    let (firehose_tx, _) = broadcast::channel(1024);

    let twitch = Twitch::new();
    let sully = SullyGnome::new(SullyGnome::DEFAULT_URL, SullyGnome::DEFAULT_CACHE_DIR)
        .context("could not create the SullyGnome client")?;
    let app = App {
        twitch: twitch.clone(),
        sully: sully.clone(),
        tiers: Tiers::new(db.clone(), sully, twitch.clone()),
        config: config.clone(),
        db,
        state,
        optout_codes: Arc::default(),
        flush_buffer,
        firehose_tx,
    };

    let (bot_tx, bot_rx) = mpsc::channel(1);

    let mut token_handle = tokio::spawn(twitch.keep_token_fresh(
        config.client_id.clone(),
        config.client_secret.clone(),
        shutdown_rx.clone(),
    ));
    let login_credentials = StaticLoginCredentials::anonymous();
    let mut bot_handle = tokio::spawn(bot::run(
        login_credentials,
        app.clone(),
        writer_tx,
        shutdown_rx.clone(),
        bot_rx,
    ));
    let mut web_handle = tokio::spawn(rustlog_web::run(app, shutdown_rx.clone(), bot_tx));

    tokio::select! {
        // Tasks stop soon after a shutdown signal, so check the signal first:
        // otherwise a task that already stopped looks like it crashed.
        biased;

        _ = shutdown_rx.changed() => {
            info!("shutting down");

            let started_at = Instant::now();

            let shutdown_future = try_join_all([bot_handle, web_handle, writer_handle, token_handle]);
            match timeout(Duration::from_secs(SHUTDOWN_TIMEOUT_SECONDS), shutdown_future).await {
                Ok(Ok(_)) => {
                    info!(took_ms = started_at.elapsed().as_millis() as u64, "shut down");
                    Ok(())
                }
                Ok(Err(err)) => Err(anyhow!("could not shut down properly: {err}")),
                Err(_) => {
                    Err(anyhow!("tasks did not shut down after {} seconds", SHUTDOWN_TIMEOUT_SECONDS))
                }
            }

        }
        _ = &mut bot_handle => {
            Err(anyhow!("bot task exited unexpectedly"))
        }
        _ = &mut web_handle => {
            Err(anyhow!("web task exited unexpectedly"))
        }
        _ = &mut writer_handle => {
            Err(anyhow!("writer task exited unexpectedly"))
        }
        _ = &mut token_handle => {
            Err(anyhow!("token refresh task exited unexpectedly"))
        }
    }
}

fn write_openapi(out_dir: &Path) -> anyhow::Result<()> {
    let api = rustlog_web::api();
    fs::create_dir_all(out_dir)
        .with_context(|| format!("could not create {}", out_dir.display()))?;

    for (name, openapi) in [
        ("legacy.json", api.legacy_openapi),
        ("v2.json", api.v2_openapi),
    ] {
        let path = out_dir.join(name);
        let mut json = serde_json::to_string_pretty(&*openapi)?;
        json.push('\n');
        fs::write(&path, json).with_context(|| format!("could not write {}", path.display()))?;
        info!(path = %path.display(), "wrote OpenAPI document");
    }

    Ok(())
}

#[cfg(unix)]
async fn listen_shutdown() -> watch::Receiver<()> {
    let shutdown_signals = [SignalKind::interrupt(), SignalKind::terminate()];
    let mut futures = FuturesUnordered::new();

    for signal_kind in shutdown_signals {
        let mut listener = signal(signal_kind).unwrap();
        futures.push(async move {
            listener.recv().await;
            signal_kind
        });
    }

    let (tx, rx) = watch::channel(());

    tokio::spawn(async move {
        futures.next().await;
        info!("received shutdown signal");
        tx.send(()).unwrap();
    });

    rx
}

#[cfg(not(unix))]
async fn listen_shutdown() -> watch::Receiver<()> {
    let (tx, rx) = watch::channel(());

    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            info!("received shutdown signal");
            let _ = tx.send(());
        }
    });

    rx
}
