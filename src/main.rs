mod args;

use anyhow::{anyhow, Context};
use args::{Args, Command};
use clap::Parser;
use futures::future::try_join_all;
#[cfg(unix)]
use futures::{stream::FuturesUnordered, StreamExt};
use mimalloc::MiMalloc;
use rustlog::{
    app::App,
    bot,
    config::Config,
    maintenance,
    migrator::Migrator,
    mirror,
    services::{sully::SullyGnome, tiers::Tiers},
    state::OperationalState,
    storage::{setup_db, writer::create_writer},
    twitch::Twitch,
    web,
};
use std::{
    env, fs,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};
#[cfg(unix)]
use tokio::signal::unix::{signal, SignalKind};
use tokio::{
    sync::{broadcast, mpsc, watch},
    time::timeout,
};
use tracing::{debug, info};
use tracing_subscriber::EnvFilter;
use twitch_irc::login::StaticLoginCredentials;

const SHUTDOWN_TIMEOUT_SECONDS: u64 = 8;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let use_ansi = env::var("RUST_LOG_ANSI")
        .ok()
        .and_then(|ansi| ansi.parse().ok())
        .unwrap_or(true);
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_ansi(use_ansi)
        .init();

    let args = Args::parse();
    if let Some(Command::Openapi { out_dir }) = &args.subcommand {
        return write_openapi(out_dir);
    }

    let config = Config::load(&args.config_path)?;
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

    setup_db(&db, &config.clickhouse_db, &config)
        .await
        .context("Could not run DB migrations")?;

    match args.subcommand {
        None => run(config, db).await,
        Some(Command::Openapi { .. }) => unreachable!("handled before loading the config"),
        Some(Command::Migrate {
            source_dir,
            channel_id,
            jobs,
        }) => migrate(db, source_dir, channel_id, jobs).await,
        Some(Command::Mirror {
            base_url,
            local_cache,
            channel,
            year,
            month,
            day,
            batch,
            http_concurrency,
            proxy,
            rps,
        }) => {
            mirror::run(
                db,
                mirror::MirrorOptions {
                    base_url,
                    local_cache,
                    channel,
                    year,
                    month,
                    day,
                    batch,
                    http_concurrency,
                    proxies: proxy,
                    rps,
                },
            )
            .await
        }
        Some(Command::FillMissing {
            channel,
            year,
            api_base,
            batch,
            http_concurrency,
            proxy,
            rps,
            exclude_instance,
            dry_run,
            repair_existing,
            deep,
        }) => {
            maintenance::fill_missing(
                db,
                maintenance::FillMissingOptions {
                    channels: channel,
                    year,
                    api_base,
                    batch,
                    http_concurrency,
                    proxies: proxy,
                    rps,
                    exclude_instances: exclude_instance,
                    dry_run,
                    repair_existing,
                    deep,
                },
            )
            .await
        }
        Some(Command::CleanupDuplicateIds {
            channel,
            year,
            execute,
            sample_limit,
            wait_timeout,
        }) => {
            maintenance::cleanup_duplicate_ids(
                db,
                maintenance::CleanupDuplicateIdsOptions {
                    channels: channel,
                    year,
                    execute,
                    sample_limit,
                    wait_timeout,
                },
            )
            .await
        }
    }
}

async fn run(config: Config, db: clickhouse::Client) -> anyhow::Result<()> {
    let mut shutdown_rx = listen_shutdown().await;

    let config = Arc::new(config);

    let db = Arc::new(db);
    let state = OperationalState::load(db.clone())
        .await
        .context("Could not load operational state")?;

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
        .context("Could not create the SullyGnome client")?;
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
    let mut web_handle = tokio::spawn(web::run(app, shutdown_rx.clone(), bot_tx));

    tokio::select! {
        _ = shutdown_rx.changed() => {
            debug!("Waiting for tasks to shut down");

            let started_at = Instant::now();

            let shutdown_future = try_join_all([bot_handle, web_handle, writer_handle, token_handle]);
            match timeout(Duration::from_secs(SHUTDOWN_TIMEOUT_SECONDS), shutdown_future).await {
                Ok(Ok(_)) => {
                    debug!("Cleanup finished in {}ms", started_at.elapsed().as_millis());
                    Ok(())
                }
                Ok(Err(err)) => Err(anyhow!("Could not shut down properly: {err}")),
                Err(_) => {
                    Err(anyhow!("Tasks did not shut down after {} seconds", SHUTDOWN_TIMEOUT_SECONDS))
                }
            }

        }
        _ = &mut bot_handle => {
            Err(anyhow!("Bot task exited unexpectedly"))
        }
        _ = &mut web_handle => {
            Err(anyhow!("Web task exited unexpectedly"))
        }
        _ = &mut writer_handle => {
            Err(anyhow!("Writer task exited unexpectedly"))
        }
        _ = &mut token_handle => {
            Err(anyhow!("Token refresh task exited unexpectedly"))
        }
    }
}

fn write_openapi(out_dir: &Path) -> anyhow::Result<()> {
    let api = web::api();
    fs::create_dir_all(out_dir)
        .with_context(|| format!("Could not create {}", out_dir.display()))?;

    for (name, openapi) in [
        ("legacy.json", api.legacy_openapi),
        ("v2.json", api.v2_openapi),
    ] {
        let path = out_dir.join(name);
        let mut json = serde_json::to_string_pretty(&*openapi)?;
        json.push('\n');
        fs::write(&path, json).with_context(|| format!("Could not write {}", path.display()))?;
        info!("Wrote {}", path.display());
    }

    Ok(())
}

async fn migrate(
    db: clickhouse::Client,
    source_logs_path: String,
    channel_ids: Vec<String>,
    jobs: usize,
) -> anyhow::Result<()> {
    let migrator = Migrator::new(db, source_logs_path, channel_ids).await?;
    migrator.run(jobs).await
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
        info!("Received shutdown signal");
        tx.send(()).unwrap();
    });

    rx
}

#[cfg(not(unix))]
async fn listen_shutdown() -> watch::Receiver<()> {
    let (tx, rx) = watch::channel(());

    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            info!("Received shutdown signal");
            let _ = tx.send(());
        }
    });

    rx
}
