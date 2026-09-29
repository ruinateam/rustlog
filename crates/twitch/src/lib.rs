//! Twitch Helix API access: the app access token and cached user and badge
//! lookups.

pub mod cache;
mod error;

pub use self::error::{Error, HelixError, Result};

use self::cache::{BadgesCache, UsersCache};
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::sync::watch;
use tokio::{sync::RwLock, time::sleep};
use tracing::{debug, info, warn};
use twitch_api::{
    HelixClient,
    helix::{
        chat::{BadgeSet, GetChannelChatBadgesRequest, GetGlobalChatBadgesRequest},
        users::GetUsersRequest,
    },
    twitch_oauth2::{AppAccessToken, Scope},
};

const TOKEN_RETRY_INTERVAL: Duration = Duration::from_secs(5);
const TOKEN_REFRESH_INTERVAL: Duration = Duration::from_secs(3600);

/// Helix client with the app access token, which [`Twitch::keep_token_fresh`]
/// obtains and renews. Until then every request fails with
/// [`Error::TwitchTokenUnavailable`].
#[derive(Clone)]
pub struct Twitch {
    helix: HelixClient<'static, reqwest::Client>,
    token: Arc<RwLock<Option<AppAccessToken>>>,
    users: UsersCache,
    badges: BadgesCache,
}

impl Default for Twitch {
    fn default() -> Self {
        Self::new()
    }
}

impl Twitch {
    pub fn new() -> Self {
        Self {
            helix: HelixClient::default(),
            token: Arc::default(),
            users: UsersCache::default(),
            badges: BadgesCache::default(),
        }
    }

    /// Obtains the app access token and renews it every hour, retrying
    /// failures every few seconds, until shutdown.
    pub async fn keep_token_fresh(
        self,
        client_id: String,
        client_secret: String,
        mut shutdown_rx: watch::Receiver<()>,
    ) {
        loop {
            tokio::select! {
                result = self.generate_token(&client_id, &client_secret) => {
                    let wait = match result {
                        Ok(new_token) => {
                            *self.token.write().await = Some(new_token);
                            TOKEN_REFRESH_INTERVAL
                        }
                        Err(err) => {
                            warn!(
                                error = format!("{err:#}"),
                                retry_in_secs = TOKEN_RETRY_INTERVAL.as_secs(),
                                "could not get a Twitch app token"
                            );
                            TOKEN_RETRY_INTERVAL
                        }
                    };
                    tokio::select! {
                        _ = sleep(wait) => {}
                        _ = shutdown_rx.changed() => {
                            debug!("shutting down token refresh task");
                            break;
                        }
                    }
                }
                _ = shutdown_rx.changed() => {
                    debug!("shutting down token refresh task");
                    break;
                }
            }
        }
    }

    async fn generate_token(
        &self,
        client_id: &str,
        client_secret: &str,
    ) -> anyhow::Result<AppAccessToken> {
        let token = AppAccessToken::get_app_access_token(
            &self.helix,
            client_id.to_owned().into(),
            client_secret.to_owned().into(),
            Scope::all(),
        )
        .await?;
        info!("got a new Twitch app token");

        Ok(token)
    }

    async fn token(&self) -> Result<AppAccessToken> {
        self.token
            .read()
            .await
            .clone()
            .ok_or(Error::TokenUnavailable)
    }

    /// Resolves user ids and logins, returning a map from id to login. Users
    /// Twitch does not know (banned, deleted) are left out.
    pub async fn get_users(
        &self,
        ids: Vec<String>,
        names: Vec<String>,
        ignore_cache: bool,
    ) -> Result<HashMap<String, String>> {
        let mut users = HashMap::new();
        let mut ids_to_request = Vec::new();
        let mut names_to_request = Vec::new();

        if ignore_cache {
            ids_to_request.clone_from(&ids);
            names_to_request.clone_from(&names);
        } else {
            for id in ids {
                match self.users.get_login(&id) {
                    Some(Some(login)) => {
                        users.insert(id, login);
                    }
                    Some(None) => (),
                    None => ids_to_request.push(id),
                }
            }

            for name in names {
                match self.users.get_id(&name) {
                    Some(Some(id)) => {
                        users.insert(id, name);
                    }
                    Some(None) => (),
                    None => names_to_request.push(name),
                }
            }
        }

        let mut new_users = Vec::with_capacity(ids_to_request.len() + names_to_request.len());
        let token = if ids_to_request.is_empty() && names_to_request.is_empty() {
            None
        } else {
            Some(self.token().await?)
        };

        // There are no chunks if the vec is empty, so there is no empty request made
        for chunk in ids_to_request.chunks(100) {
            debug!(ids = ?chunk, "requesting Twitch users by id");

            let request = GetUsersRequest::ids(chunk);
            let response = self
                .helix
                .req_get(request, token.as_ref().expect("token exists"))
                .await?;
            new_users.extend(response.data);
        }

        for chunk in names_to_request.chunks(100) {
            debug!(logins = ?chunk, "requesting Twitch users by login");

            let request = GetUsersRequest::logins(chunk);
            let response = self
                .helix
                .req_get(request, token.as_ref().expect("token exists"))
                .await?;
            new_users.extend(response.data);
        }

        for user in new_users {
            let id = user.id.to_string();
            let login = user.login.to_string();

            self.users.insert(id.clone(), login.clone());

            users.insert(id, login);
        }

        // Banned users which were not returned by the api
        for id in ids_to_request {
            if !users.contains_key(id.as_str()) {
                self.users.insert_optional(Some(id), None);
            }
        }
        for name in names_to_request {
            if !users.values().any(|login| login == name.as_str()) {
                self.users.insert_optional(None, Some(name));
            }
        }

        Ok(users)
    }

    pub async fn get_user_id_by_name(&self, name: &str) -> Result<String> {
        match self.users.get_id(name) {
            Some(Some(id)) => Ok(id),
            Some(None) => Err(Error::NotFound),
            None => {
                let token = self.token().await?;
                let request = GetUsersRequest::logins(vec![name]);
                let response = self.helix.req_get(request, &token).await?;
                match response.data.into_iter().next() {
                    Some(user) => {
                        let user_id = user.id.to_string();
                        self.users.insert(user_id.clone(), user.login.to_string());
                        Ok(user_id)
                    }
                    None => {
                        self.users.insert_optional(None, Some(name.to_owned()));
                        Err(Error::NotFound)
                    }
                }
            }
        }
    }

    /// The global and the channel's chat badge sets.
    pub async fn get_chat_badges(
        &self,
        channel_id: &str,
    ) -> Result<(Vec<BadgeSet>, Vec<BadgeSet>)> {
        let global = match self.badges.get(None) {
            Some(global) => global,
            None => {
                let token = self.token().await?;
                let global = self
                    .helix
                    .req_get(GetGlobalChatBadgesRequest::new(), &token)
                    .await?
                    .data;
                self.badges.insert(None, global.clone());
                global
            }
        };

        let channel = match self.badges.get(Some(channel_id)) {
            Some(channel) => channel,
            None => {
                let token = self.token().await?;
                let channel = self
                    .helix
                    .req_get(
                        GetChannelChatBadgesRequest::broadcaster_id(channel_id),
                        &token,
                    )
                    .await?
                    .data;
                self.badges.insert(Some(channel_id), channel.clone());
                channel
            }
        };

        Ok((global, channel))
    }
}
