//! API v2: users, channels and chat badges.

use crate::support::get;
use insta::assert_snapshot;

#[tokio::test]
async fn users_need_twitch() {
    assert_snapshot!(get("/api/v2/users?login=testchan&id=22222").send().await);
}

#[tokio::test]
async fn users_without_parameters() {
    assert_snapshot!(get("/api/v2/users").send().await);
}

#[tokio::test]
async fn users_with_invalid_id() {
    assert_snapshot!(get("/api/v2/users?id=alice").send().await);
}

#[tokio::test]
async fn too_many_users() {
    let logins: Vec<String> = (0..101).map(|number| format!("login={number}")).collect();
    assert_snapshot!(
        get(format!("/api/v2/users?{}", logins.join("&")))
            .without_body()
            .send()
            .await
    );
}

#[tokio::test]
async fn name_history() {
    assert_snapshot!(get("/api/v2/users/22222/name-history").send().await);
}

#[tokio::test]
async fn channels_need_twitch() {
    assert_snapshot!(get("/api/v2/channels").send().await);
}

#[tokio::test]
async fn badges_need_twitch() {
    assert_snapshot!(get("/api/v2/channels/11111/badges").send().await);
}

#[tokio::test]
async fn badges_of_unlogged_channel() {
    assert_snapshot!(get("/api/v2/channels/99999/badges").send().await);
}

#[tokio::test]
async fn login_instead_of_id() {
    assert_snapshot!(get("/api/v2/channels/testchan/badges").send().await);
}
