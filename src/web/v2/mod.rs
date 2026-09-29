//! The v2 API under `/api/v2`: numeric Twitch ids in paths, explicit ranges
//! and `application/problem+json` errors. See `docs/API_V2.md`.

mod admin;
mod docs;
mod extract;
mod logs;
mod params;
mod problem;
mod stats;
mod tiers;
mod users;

use self::{docs::V2Spec, problem::ApiProblem};
use crate::{app::App, web::openapi::enrich_openapi};
use aide::{
    axum::{
        ApiRouter,
        routing::{get, get_with, post_with, put_with},
    },
    openapi::{Info, OpenApi, Server, Tag},
    transform::TransformOperation,
};
use axum::{Extension, Router, routing::any};
use std::sync::Arc;

/// The v2 routes, to be nested under `/api/v2`, with their OpenAPI document.
pub struct V2Api {
    pub router: Router<App>,
    pub spec: Arc<OpenApi>,
}

pub fn api() -> V2Api {
    let mut spec = openapi();
    let router = routes()
        .route("/docs", get(crate::web::docs::redirect_to_page))
        .route("/openapi.json", get(docs::serve_openapi))
        // Unlike a nested service, a nested router neither matches its bare
        // prefix nor keeps its own default fallback: without these, `/api/v2`
        // would hit the legacy routes and unknown paths the frontend fallback.
        .route("/", any(not_found))
        .fallback(not_found)
        .finish_api(&mut spec)
        .method_not_allowed_fallback(method_not_allowed);
    enrich_openapi(&mut spec);
    let spec = Arc::new(spec);

    V2Api {
        router: router.layer(Extension(V2Spec(spec.clone()))),
        spec,
    }
}

fn openapi() -> OpenApi {
    let tag = |name: &str, description: &str| Tag {
        name: name.to_owned(),
        description: Some(description.to_owned()),
        ..Tag::default()
    };

    OpenApi {
        info: Info {
            title: "ChatTiers Rustlog API".to_owned(),
            summary: Some("Twitch chat logs, stats and chat tiers.".to_owned()),
            description: Some(include_str!("description.md").to_owned()),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            ..Info::default()
        },
        servers: vec![Server {
            url: "/api/v2".to_owned(),
            description: Some("This rustlog instance".to_owned()),
            ..Server::default()
        }],
        tags: vec![
            tag("Users", "Look up Twitch users and the logins they had."),
            tag(
                "Channels",
                "The logged channels, their chat badges and streams.",
            ),
            tag("Logs", "Chat messages of a channel or of one user in it."),
            tag("Stats", "Message counts."),
            tag("Tiers", "Users ranked by how steadily they chat."),
            tag("Opt-out", "Stop being logged."),
            tag(
                "Admin",
                "Manage the logged channels; needs the admin API key.",
            ),
        ],
        ..OpenApi::default()
    }
}

/// Describes an operation in the generated document.
fn describe<'a>(
    operation: TransformOperation<'a>,
    id: &str,
    tag: &str,
    summary: &str,
    description: &str,
) -> TransformOperation<'a> {
    operation
        .id(id)
        .tag(tag)
        .summary(summary)
        .description(description)
}

fn routes() -> ApiRouter<App> {
    ApiRouter::new()
        .api_route(
            "/users",
            get_with(users::users, |op| {
                describe(op, "getUsers", "Users", "Look up users",
                    "Look up users by login or id, up to 100 in total; repeat `login` or `id` for several. Users Twitch does not know are left out.")
            }),
        )
        .api_route(
            "/users/{userId}/name-history",
            get_with(stats::name_history, |op| {
                describe(op, "getNameHistory", "Users", "Get the logins of a user",
                    "Every login the user was seen with in a logged chat, with the first and last time.")
            }),
        )
        .api_route(
            "/channels",
            get_with(users::channels, |op| {
                describe(op, "getChannels", "Channels", "List the logged channels", "The channels being logged, ordered by login.")
            }),
        )
        .api_route(
            "/channels/{channelId}/badges",
            get_with(users::badges, |op| {
                describe(op, "getChatBadges", "Channels", "Get the chat badges of a channel",
                    "The global and the channel's Twitch chat badges, to render the `badges` tag of messages. Only for logged channels.")
            }),
        )
        .api_route(
            "/channels/{channelId}/streams",
            get_with(tiers::streams, |op| {
                describe(op, "getStreams", "Channels", "List the streams of a year",
                    "The channel's streams in a year as SullyGnome knows them, from its cache when SullyGnome is unreachable.")
            }),
        )
        .api_route(
            "/channels/{channelId}/log-dates",
            get_with(logs::channel_log_dates, |op| {
                describe(op, "getChannelLogDates", "Logs", "List the days with logs", "The UTC days with messages in the channel, newest first.")
            }),
        )
        .api_route(
            "/channels/{channelId}/logs",
            get_with(logs::channel_logs, |op| {
                describe(op, "getChannelLogs", "Logs", "Get the messages of a channel", "The channel's messages in `[from, to)`.")
            }),
        )
        .api_route(
            "/channels/{channelId}/logs/random",
            get_with(logs::random_channel_message, |op| {
                describe(op, "getRandomChannelMessage", "Logs", "Get a random message of a channel", "One message picked at random.")
            }),
        )
        .api_route(
            "/channels/{channelId}/stats",
            get_with(stats::channel_stats, |op| {
                describe(op, "getChannelStats", "Stats", "Count the messages of a channel",
                    "The number of messages and the users with the most, optionally in `[from, to)`.")
            }),
        )
        .api_route(
            "/channels/{channelId}/tiers/{period}",
            get_with(tiers::tier_table, |op| {
                describe(op, "getTiers", "Tiers", "Rank the chatters of a period",
                    "Ranks the users by the number of 1, 5, 15, 30 and 60 minute windows they chatted in, during a calendar day, month or year in Europe/Moscow time.")
            }),
        )
        .api_route(
            "/channels/{channelId}/users/{userId}/log-months",
            get_with(logs::user_log_months, |op| {
                describe(op, "getUserLogMonths", "Logs", "List the months with logs of a user",
                    "The UTC months in which the user wrote in the channel, newest first.")
            }),
        )
        .api_route(
            "/channels/{channelId}/users/{userId}/logs",
            get_with(logs::user_logs, |op| {
                describe(op, "getUserLogs", "Logs", "Get the messages of a user", "The user's messages in the channel in `[from, to)`.")
            }),
        )
        .api_route(
            "/channels/{channelId}/users/{userId}/logs/random",
            get_with(logs::random_user_message, |op| {
                describe(op, "getRandomUserMessage", "Logs", "Get a random message of a user",
                    "One of the user's messages in the channel, picked at random.")
            }),
        )
        .api_route(
            "/channels/{channelId}/users/{userId}/logs/search",
            get_with(logs::search_user_logs, |op| {
                describe(op, "searchUserLogs", "Logs", "Search the messages of a user",
                    "The user's messages in the channel that contain `q`, ignoring case.")
            }),
        )
        .api_route(
            "/channels/{channelId}/users/{userId}/stats",
            get_with(stats::user_stats, |op| {
                describe(op, "getUserStats", "Stats", "Count the messages of a user",
                    "The number of the user's messages in the channel, optionally in `[from, to)`.")
            }),
        )
        .api_route(
            "/opt-out-codes",
            post_with(admin::create_opt_out_code, |op| {
                describe(op, "createOptOutCode", "Opt-out", "Create an opt-out code",
                    "A one-time code for a chat command in a logged chat, valid for a minute: `!rustlog optout <code>` stops logging the sender and deletes their messages, `!rustlog optin <code>` logs them again. A broadcaster can hide their channel with `!rustlog optout-channel <code>` in its own chat, and show it again with `!rustlog optin-channel <code>`.")
            }),
        )
        .api_route(
            "/admin/channels/{channelId}/opt-out",
            put_with(admin::opt_out_channel, |op| {
                describe(op, "optOutChannel", "Admin", "Opt a channel out",
                    "Stops logging the channel and hides its logs, which are kept. The bot stays in the chat, so that the broadcaster can opt back in there.")
            })
            .delete_with(admin::opt_in_channel, |op| {
                describe(op, "optInChannel", "Admin", "Opt a channel back in",
                    "Logs the channel again and shows its logs, including those from before the opt-out.")
            }),
        )
        .api_route(
            "/admin/users/{userId}/opt-out",
            put_with(admin::opt_out_user, |op| {
                describe(op, "optOutUser", "Admin", "Opt a user out",
                    "Stops logging the user and deletes their messages and logins in every channel.")
            })
            .delete_with(admin::opt_in_user, |op| {
                describe(op, "optInUser", "Admin", "Opt a user back in",
                    "Logs the user again from now on; messages deleted by the opt-out stay deleted.")
            }),
        )
        .api_route(
            "/admin/channels/{channelId}",
            put_with(admin::join_channel, |op| {
                describe(op, "joinChannel", "Admin", "Start logging a channel", "The bot joins the channel's chat.")
            })
            .delete_with(admin::leave_channel, |op| {
                describe(op, "leaveChannel", "Admin", "Stop logging a channel",
                    "The bot leaves the channel's chat; its logs are kept.")
            }),
        )
}

async fn not_found() -> ApiProblem {
    ApiProblem::not_found("no such endpoint")
}

async fn method_not_allowed() -> ApiProblem {
    ApiProblem::method_not_allowed()
}
