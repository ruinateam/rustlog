//! `fill-missing`: import the days of a year that logs.zonian.dev knows
//! about but are not stored locally, from the instances that log the
//! channel. With `--repair-existing`, stored days that a mirror has more
//! messages of are completed too.

mod local_days;
mod repair;
mod zonian;

use self::{local_days::LocalDays, repair::Comparison};
use crate::mirror::{self, HttpPool, ImportSettings, LogSource, TransferOptions};
use anyhow::bail;
use chrono::NaiveDate;
use clickhouse::Client;
use std::collections::{BTreeSet, HashSet};
use tracing::{info, warn};

/// Instances whose logs are not trusted.
const DEFAULT_EXCLUDED_INSTANCES: &[&str] = &["https://logs.twitchmetrics.xyz"];

#[derive(Debug, Clone, clap::Args)]
pub struct FillMissingOptions {
    /// Channel login to fill. Repeatable.
    #[arg(long = "channel", value_name = "CHANNEL", required = true)]
    pub channels: Vec<String>,
    /// Year to fill
    #[arg(long)]
    pub year: u32,
    /// Base URL of the logs.zonian.dev API
    #[arg(long, default_value = "https://logs.zonian.dev")]
    pub api_base: String,
    #[command(flatten)]
    pub transfer: TransferOptions,
    /// Base URL of an instance not to import from. Repeatable.
    #[arg(long = "exclude-instance", value_name = "BASE_URL")]
    pub exclude_instances: Vec<String>,
    /// Only report the missing (and with --repair-existing, the incomplete) days
    #[arg(long)]
    pub dry_run: bool,
    /// Also complete stored days that a mirror has more message ids of
    #[arg(long)]
    pub repair_existing: bool,
    /// With --repair-existing, compare every mirror instead of the first one that answers
    #[arg(long)]
    pub deep: bool,
}

/// What to fill for one channel.
pub struct ChannelPlan {
    pub channel: String,
    /// Days of the year that some instance has logs of.
    pub expected_days: BTreeSet<NaiveDate>,
    /// Base URLs of the instances to import from, in order of preference.
    pub mirrors: Vec<String>,
}

impl ChannelPlan {
    fn missing_days(&self, local_days: &LocalDays) -> Vec<NaiveDate> {
        let stored = local_days.get(&self.channel);
        self.expected_days
            .iter()
            .filter(|date| stored.is_none_or(|days| !days.contains_key(date)))
            .copied()
            .collect()
    }
}

pub async fn run(db: Client, options: FillMissingOptions) -> anyhow::Result<()> {
    let year = options.year as i32;
    let http = options.transfer.http_pool("rustlog-fill-missing/0.2")?;
    let settings = options.transfer.import_settings();
    let comparison = if options.deep {
        Comparison::AllMirrors
    } else {
        Comparison::FirstReadable
    };

    let plans = plan(&http, &options).await?;
    let local_days = local_days::read(&db, &options.channels, year).await?;
    for plan in &plans {
        let missing = plan.missing_days(&local_days);
        info!(
            channel = %plan.channel,
            local_days = local_days.get(&plan.channel).map_or(0, |days| days.len()),
            missing_days = missing.len(),
            "compared local days with mirrors"
        );
        if !missing.is_empty() {
            info!(channel = %plan.channel, days = join_dates(&missing), "missing days");
        }
    }

    if options.dry_run {
        if options.repair_existing {
            let candidates = repair::find_candidates(
                &http,
                &plans,
                &local_days,
                settings.concurrency,
                comparison,
            )
            .await;
            repair::log_candidates(&candidates);
        }
        return Ok(());
    }

    let mut still_missing = 0;
    for plan in &plans {
        let missing = plan.missing_days(&local_days);
        still_missing += fill_from_mirrors(&db, &http, plan, missing, year, settings).await?;
    }

    if options.repair_existing {
        let local_days = local_days::read(&db, &options.channels, year).await?;
        let candidates =
            repair::find_candidates(&http, &plans, &local_days, settings.concurrency, comparison)
                .await;
        repair::log_candidates(&candidates);
        repair::repair(&db, &http, &candidates, settings).await?;
    }

    log_summary(&db, &plans, &options.channels, year).await?;

    if still_missing > 0 {
        bail!("{still_missing} days are still missing");
    }
    Ok(())
}

/// Asks logs.zonian.dev which days and mirrors each channel has.
async fn plan(http: &HttpPool, options: &FillMissingOptions) -> anyhow::Result<Vec<ChannelPlan>> {
    let excluded: HashSet<String> = DEFAULT_EXCLUDED_INSTANCES
        .iter()
        .map(|instance| (*instance).to_owned())
        .chain(
            options
                .exclude_instances
                .iter()
                .map(|instance| instance.trim_end_matches('/').to_owned()),
        )
        .collect();

    let mut plans = Vec::with_capacity(options.channels.len());
    for channel in &options.channels {
        let overview =
            zonian::channel_overview(http, &options.api_base, channel, options.year as i32).await?;
        let mirrors: Vec<String> = overview
            .instances
            .into_iter()
            .filter(|instance| !excluded.contains(instance))
            .collect();
        info!(
            channel = %channel,
            expected_days = overview.days.len(),
            mirrors = ?mirrors,
            "found mirrors of channel"
        );
        plans.push(ChannelPlan {
            channel: channel.clone(),
            expected_days: overview.days,
            mirrors,
        });
    }
    Ok(plans)
}

/// Tries the mirrors in turn until every day is stored; returns how many
/// days are still missing.
async fn fill_from_mirrors(
    db: &Client,
    http: &HttpPool,
    plan: &ChannelPlan,
    mut missing: Vec<NaiveDate>,
    year: i32,
    settings: ImportSettings,
) -> anyhow::Result<usize> {
    let channel = &plan.channel;

    for mirror in &plan.mirrors {
        if missing.is_empty() {
            break;
        }
        info!(channel = %channel, days = missing.len(), mirror = %mirror, "filling days from mirror");

        let source = LogSource::remote(http.clone(), mirror);
        if let Err(error) = mirror::import_days(db, &source, channel, &missing, settings).await {
            warn!(
                channel = %channel,
                mirror = %mirror,
                error = format!("{error:#}"),
                "could not fill days from mirror"
            );
        }

        // A mirror can lack days that it listed, so check what is stored now.
        let local_days = local_days::read(db, std::slice::from_ref(channel), year).await?;
        let stored = local_days.get(channel);
        let before = missing.len();
        missing.retain(|date| stored.is_none_or(|days| !days.contains_key(date)));
        info!(
            channel = %channel,
            newly_filled = before - missing.len(),
            remaining = missing.len(),
            mirror = %mirror,
            "filled days from mirror"
        );
    }

    for date in &missing {
        warn!(channel = %channel, %date, "day is still missing");
    }
    Ok(missing.len())
}

async fn log_summary(
    db: &Client,
    plans: &[ChannelPlan],
    channels: &[String],
    year: i32,
) -> anyhow::Result<()> {
    let local_days = local_days::read(db, channels, year).await?;
    for plan in plans {
        let stored = local_days.get(&plan.channel);
        let missing = plan.missing_days(&local_days);
        info!(
            channel = %plan.channel,
            local_days = stored.map_or(0, |days| days.len()),
            expected_days = plan.expected_days.len(),
            rows = stored.map_or(0, |days| days.values().map(|day| day.rows).sum::<u64>()),
            missing_days = missing.len(),
            "channel summary"
        );
        if !missing.is_empty() {
            info!(channel = %plan.channel, days = join_dates(&missing), "days still missing");
        }
    }
    Ok(())
}

fn join_dates(dates: &[NaiveDate]) -> String {
    dates
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(" ")
}
