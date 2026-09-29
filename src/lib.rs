//! Twitch chat logger backed by ClickHouse, with a justlog-compatible HTTP API.

pub mod app;
pub mod bot;
pub mod config;
pub mod domain;
pub mod irc;
pub mod logging;
pub mod maintenance;
pub mod migrator;
pub mod mirror;
pub mod services;
pub mod state;
pub mod storage;
pub mod twitch;
pub mod web;

pub type ShutdownRx = tokio::sync::watch::Receiver<()>;
