use axum::{routing, Router};
use maud::{html, Markup};

use crate::{auth::Auth, base_tempalte, AppState};

mod friends;
mod profile;

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/",
            routing::get(|Auth { id: user_id }: Auth| async move {
                base_tempalte(html!(h1 { "hello "(user_id)"!" }))
            }),
        )
        .nest("/profile", profile::router())
        .nest("/friends", friends::router())
}

#[derive(PartialEq)]
enum UserTab {
    Profile,
    Friends,
}
fn render_user_nav(active: UserTab) -> Markup {
    use UserTab::*;
    html!(
        div class="tabs-boxed tabs" {
            button.tab.tab-active[active == Profile] hx-get={"/users/profile"} { "Profile" }
            button.tab.tab-active[active == Friends] hx-get={"/users/friends"} { "Friends" }
        }
    )
}
