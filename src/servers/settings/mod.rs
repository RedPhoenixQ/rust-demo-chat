use axum::{
    extract::{Path, Request, State},
    http::StatusCode,
    middleware::{from_fn_with_state, Next},
    response::IntoResponse,
    routing, Router,
};
use maud::{html, Markup};
use sqlx::query;
use uuid::Uuid;

use crate::{auth::Auth, error::Result, AppState};

use super::ServerId;

mod general;
mod members;

pub fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .route(
            "/",
            routing::get(general::open_general_page).put(general::update_server),
        )
        .route(
            "/members",
            routing::get(members::open_member_page).post(members::add_member),
        )
        .route(
            "/members/:member_id",
            routing::delete(members::remove_member),
        )
        .route("/members/table", routing::get(members::get_member_table))
        .layer(from_fn_with_state(state.clone(), is_allowed_to_edit_server))
}

async fn is_allowed_to_edit_server(
    State(state): State<AppState>,
    Auth { id: user_id }: Auth,
    Path(ServerId { server_id }): Path<ServerId>,
    request: Request,
    next: Next,
) -> Result<impl IntoResponse> {
    // FIXME: Check for edit rights
    match query!(
        r#"SELECT EXISTS(SELECT * FROM users_member_of_servers WHERE "user" = $1 AND server = $2) as "is_member!""#,
        user_id,
        server_id,
    )
    .fetch_one(&state.db).await?.is_member {
        true => Ok(next.run(request).await),
        false => Ok(StatusCode::UNAUTHORIZED.into_response())
    }
}

#[derive(PartialEq)]
enum SettingsTab {
    General,
    Members,
}
fn render_settings_nav(server_id: Uuid, active: SettingsTab) -> Markup {
    use SettingsTab::*;
    html!(
        div class="tabs-boxed tabs" {
            button.tab.tab-active[active == General] hx-get={"/servers/"(server_id)"/settings"} { "General" }
            button.tab.tab-active[active == Members] hx-get={"/servers/"(server_id)"/settings/members"} { "Members" }
        }
    )
}
