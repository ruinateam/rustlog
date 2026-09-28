pub mod cache;

use self::cache::UsersCache;
use crate::{
    config::Config,
    db::{delete_user_logs, writer::FlushBuffer},
    error::Error,
    state::OperationalState,
    Result,
};
use anyhow::Context;
use dashmap::DashSet;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::{broadcast, RwLock};
use tracing::{debug, info};
use twitch_api::{
    helix::{
        chat::{BadgeSet, GetChannelChatBadgesRequest, GetGlobalChatBadgesRequest},
        users::GetUsersRequest,
    },
    twitch_oauth2::AppAccessToken,
    HelixClient,
};

#[derive(Clone)]
pub struct App {
    pub helix_client: HelixClient<'static, reqwest::Client>,
    pub token: Arc<RwLock<Option<AppAccessToken>>>,
    pub users: UsersCache,
    pub optout_codes: Arc<DashSet<String>>,
    pub db: Arc<clickhouse::Client>,
    pub config: Arc<Config>,
    pub state: OperationalState,
    pub flush_buffer: FlushBuffer,
    pub firehose_tx: broadcast::Sender<crate::db::schema::StructuredMessage<'static>>,
}

impl App {
    async fn token(&self) -> Result<AppAccessToken> {
        self.token
            .read()
            .await
            .clone()
            .ok_or(Error::TwitchTokenUnavailable)
    }

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
            debug!("Requesting user info for ids {chunk:?}");

            let request = GetUsersRequest::ids(chunk);
            let response = self
                .helix_client
                .req_get(request, token.as_ref().expect("token exists"))
                .await?;
            new_users.extend(response.data);
        }

        for chunk in names_to_request.chunks(100) {
            debug!("Requesting user info for names {chunk:?}");

            let request = GetUsersRequest::logins(chunk);
            let response = self
                .helix_client
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
                let response = self.helix_client.req_get(request, &token).await?;
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

    pub async fn optout_user(&self, user_id: &str) -> anyhow::Result<()> {
        self.state.optout_user(user_id).await?;
        self.flush_buffer.remove_user(user_id).await;
        delete_user_logs(&self.db, user_id)
            .await
            .context("Could not delete logs")?;

        info!("User {user_id} opted out");

        Ok(())
    }

    pub fn check_opted_out(&self, channel_id: &str, user_id: Option<&str>) -> Result<()> {
        if self.state.is_channel_opted_out(channel_id) {
            return Err(Error::ChannelOptedOut);
        }

        if let Some(user_id) = user_id {
            if self.state.is_user_opted_out(user_id) {
                return Err(Error::UserOptedOut);
            }
        }

        Ok(())
    }

    pub async fn get_chat_badges(
        &self,
        channel_id: &str,
    ) -> Result<(Vec<BadgeSet>, Vec<BadgeSet>)> {
        let token = self.token().await?;
        let global = self
            .helix_client
            .req_get(GetGlobalChatBadgesRequest::new(), &token)
            .await?
            .data;
        let channel = self
            .helix_client
            .req_get(
                GetChannelChatBadgesRequest::broadcaster_id(channel_id),
                &token,
            )
            .await?
            .data;

        Ok((global, channel))
    }

    pub fn check_user_opted_out(&self, user_id: &str) -> Result<()> {
        if self.state.is_user_opted_out(user_id) {
            return Err(Error::UserOptedOut);
        }

        Ok(())
    }
}
