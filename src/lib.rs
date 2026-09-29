//! Twitch chat logger backed by ClickHouse, with a justlog-compatible HTTP API.

pub mod app;
pub mod bot;
pub mod config;
pub mod db;
pub mod domain;
pub mod error;
pub mod logs;
pub mod maintenance;
pub mod migrator;
pub mod mirror;
pub mod services;
pub mod state;
pub mod web;

pub type Result<T> = std::result::Result<T, error::Error>;
pub type ShutdownRx = tokio::sync::watch::Receiver<()>;
