//! Generation of opt-out codes.

use axum::{Json, extract::State};
use rand::{RngExt, distr::Alphanumeric, rng};
use rustlog_app::App;
use std::time::Duration;
use tracing::debug;

pub async fn optout(app: State<App>) -> Json<String> {
    let mut rng = rng();
    let optout_code: String = (0..5).map(|_| rng.sample(Alphanumeric) as char).collect();

    app.optout_codes.insert(optout_code.clone());

    {
        let codes = app.optout_codes.clone();
        let optout_code = optout_code.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(60)).await;
            if codes.remove(&optout_code).is_some() {
                debug!(code = %optout_code, "opt-out code expired");
            }
        });
    }

    Json(optout_code)
}
