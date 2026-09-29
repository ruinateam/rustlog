//! `--repair-existing`: find stored days that a mirror has more messages of,
//! and import the missing messages.

use super::{
    ChannelPlan,
    local_days::{LocalDay, LocalDays},
};
use crate::mirror::{self, HttpPool, ImportSettings, LogSource};
use anyhow::{Context, bail};
use chrono::{Datelike, NaiveDate};
use clickhouse::Client;
use futures::{StreamExt, stream};
use serde::Deserialize;
use std::collections::{BTreeMap, HashSet};
use tracing::{info, warn};

/// A stored day that a mirror has more messages of.
pub struct Candidate {
    pub channel: String,
    pub date: NaiveDate,
    pub local_unique_ids: u64,
    pub remote_unique_ids: u64,
    pub mirror: String,
}

/// Which mirrors are compared with a stored day.
#[derive(Debug, Clone, Copy)]
pub enum Comparison {
    /// The first mirror that answers.
    FirstReadable,
    /// All mirrors, taking the one with the most messages.
    AllMirrors,
}

/// A stored day and the mirrors to compare it with.
struct DayCheck<'a> {
    channel: &'a str,
    date: NaiveDate,
    local: LocalDay,
    mirrors: &'a [String],
}

pub async fn find_candidates(
    http: &HttpPool,
    plans: &[ChannelPlan],
    local_days: &LocalDays,
    concurrency: usize,
    comparison: Comparison,
) -> Vec<Candidate> {
    let checks: Vec<DayCheck> = plans
        .iter()
        .flat_map(|plan| {
            let stored = local_days.get(&plan.channel);
            plan.expected_days.iter().filter_map(move |date| {
                Some(DayCheck {
                    channel: &plan.channel,
                    date: *date,
                    local: *stored?.get(date)?,
                    mirrors: &plan.mirrors,
                })
            })
        })
        .collect();
    if checks.is_empty() {
        return Vec::new();
    }

    // Comparing with all mirrors sends a request to each of them per day.
    let day_concurrency = match comparison {
        Comparison::FirstReadable => concurrency,
        Comparison::AllMirrors => {
            let most_mirrors = checks.iter().map(|check| check.mirrors.len()).max();
            (concurrency / most_mirrors.unwrap_or(1).max(1)).max(1)
        }
    };
    info!(
        days = checks.len(),
        day_concurrency,
        request_concurrency = concurrency,
        ?comparison,
        "checking stored days for missing messages"
    );

    let mut candidates: Vec<Candidate> = stream::iter(checks)
        .map(|check| check.find_candidate(http, comparison))
        .buffer_unordered(day_concurrency)
        .filter_map(|candidate| async move { candidate })
        .collect()
        .await;
    candidates.sort_by(|a, b| (&a.channel, a.date).cmp(&(&b.channel, b.date)));
    candidates
}

impl DayCheck<'_> {
    async fn find_candidate(self, http: &HttpPool, comparison: Comparison) -> Option<Candidate> {
        let mut failed_checks = 0_u32;
        let mut first_error = None;
        let mut best: Option<(&String, u64)> = None;

        let mut counts = stream::iter(self.mirrors)
            .map(|mirror| async move {
                (
                    mirror,
                    remote_unique_ids(http, mirror, self.channel, self.date).await,
                )
            })
            .buffered(match comparison {
                Comparison::FirstReadable => 1,
                Comparison::AllMirrors => self.mirrors.len().max(1),
            });
        while let Some((mirror, result)) = counts.next().await {
            match result {
                Ok(count) => {
                    if best.is_none_or(|(_, best_count)| count > best_count) {
                        best = Some((mirror, count));
                    }
                    if matches!(comparison, Comparison::FirstReadable) {
                        break;
                    }
                }
                Err(error) => {
                    failed_checks += 1;
                    first_error.get_or_insert_with(|| format!("{mirror}: {error:#}"));
                }
            }
        }

        let Some((mirror, remote_unique_ids)) = best else {
            warn!(
                channel = self.channel,
                date = %self.date,
                failed_checks,
                first_error = first_error.as_deref().unwrap_or("none"),
                "no readable mirror for day"
            );
            return None;
        };

        (remote_unique_ids > self.local.unique_ids).then(|| Candidate {
            channel: self.channel.to_owned(),
            date: self.date,
            local_unique_ids: self.local.unique_ids,
            remote_unique_ids,
            mirror: mirror.clone(),
        })
    }
}

/// A day of logs, reading only the message ids.
#[derive(Deserialize)]
struct DayOfIds<'a> {
    #[serde(default, borrow)]
    messages: Vec<MessageId<'a>>,
}

#[derive(Deserialize)]
struct MessageId<'a> {
    #[serde(default, borrow)]
    id: Option<&'a str>,
}

/// How many distinct message ids the mirror has of the day, or how many
/// messages when none has an id.
async fn remote_unique_ids(
    http: &HttpPool,
    mirror: &str,
    channel: &str,
    date: NaiveDate,
) -> anyhow::Result<u64> {
    let url = format!(
        "{mirror}/channel/{channel}/{:04}/{:02}/{:02}?jsonBasic=1",
        date.year(),
        date.month(),
        date.day()
    );
    let body = http.get_bytes(&url).await?;
    let day: DayOfIds = serde_json::from_slice(&body)
        .with_context(|| format!("could not decode a day of logs ({} bytes)", body.len()))?;

    let ids: HashSet<&str> = day
        .messages
        .iter()
        .filter_map(|message| message.id.filter(|id| !id.is_empty()))
        .collect();
    Ok(if ids.is_empty() {
        day.messages.len() as u64
    } else {
        ids.len() as u64
    })
}

pub fn log_candidates(candidates: &[Candidate]) {
    if candidates.is_empty() {
        info!("no incomplete days found");
        return;
    }

    info!(days = candidates.len(), "found incomplete days");
    for candidate in candidates {
        info!(
            channel = %candidate.channel,
            date = %candidate.date,
            local_unique_ids = candidate.local_unique_ids,
            remote_unique_ids = candidate.remote_unique_ids,
            mirror = %candidate.mirror,
            "incomplete day"
        );
    }
}

/// Imports the candidate days from the mirror that has the most of them.
pub async fn repair(
    db: &Client,
    http: &HttpPool,
    candidates: &[Candidate],
    settings: ImportSettings,
) -> anyhow::Result<()> {
    let mut days_by_source: BTreeMap<(&str, &str), Vec<NaiveDate>> = BTreeMap::new();
    for candidate in candidates {
        days_by_source
            .entry((&candidate.channel, &candidate.mirror))
            .or_default()
            .push(candidate.date);
    }

    let mut failed_groups = 0;
    for ((channel, mirror), days) in days_by_source {
        info!(
            channel,
            days = days.len(),
            mirror,
            "repairing days from mirror"
        );
        let source = LogSource::remote(http.clone(), mirror);
        if let Err(error) = mirror::import_days(db, &source, channel, &days, settings).await {
            warn!(
                channel,
                mirror,
                error = format!("{error:#}"),
                "could not repair days from mirror"
            );
            failed_groups += 1;
        }
    }

    if failed_groups > 0 {
        bail!("could not repair the days of {failed_groups} channel and mirror pairs");
    }
    Ok(())
}
