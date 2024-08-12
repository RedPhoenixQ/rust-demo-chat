use axum::{extract::State, response::IntoResponse, routing, Router};
use axum_htmx::HxResponseTrigger;
use maud::{html, Markup};
use sqlx::{query, PgPool};
use uuid::Uuid;

use crate::{auth::Auth, base_modal, error::Result, AppState};

use super::{render_user_nav, UserTab};

pub fn router() -> Router<AppState> {
    Router::new().route("/", routing::get(open_user_profile))
}

async fn open_user_profile(
    State(state): State<AppState>,
    Auth { id: user_id }: Auth,
) -> Result<impl IntoResponse> {
    Ok((
        HxResponseTrigger::normal(["open-main-modal"]),
        fetch_and_render_user_profile(&state.db, user_id).await?,
    ))
}
async fn fetch_and_render_user_profile(pool: &PgPool, user_id: Uuid) -> Result<Markup> {
    let user = query!("SELECT id, name FROM chat_users WHERE id = $1", user_id)
        .fetch_one(pool)
        .await?;

    Ok(base_modal(html! {
        (render_user_nav(UserTab::Profile))
        form {
          label.form-control {
            .label { .label-text { "Username" } }
            input type="text" class="input input-bordered" value=(user.name);
          }
        }
        div class="flex items-center" {
          (user.id)
          button class="btn btn-circle btn-ghost btn-sm"
            onclick={"navigator.clipboard.writeText('"(user.id)"')"}
            title="Copy user id"
            { ">" }
        }
    }))
}
