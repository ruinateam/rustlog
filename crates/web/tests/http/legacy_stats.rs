//! Legacy statistics and username history routes.

use insta::assert_snapshot;

use crate::support::get;

#[tokio::test]
async fn channel_stats() {
    assert_snapshot!(get("/channelid/11111/stats").send().await);
}

#[tokio::test]
async fn user_stats() {
    assert_snapshot!(get("/channelid/11111/userid/22222/stats").send().await);
}

#[tokio::test]
async fn name_history() {
    assert_snapshot!(get("/namehistory/22222").send().await);
}
