use axum::{
    extract::{Path, State},
    response::IntoResponse,
    Form,
};
use axum_htmx::HxResponseTrigger;
use maud::{html, Markup};
use serde::Deserialize;
use sqlx::query;
use uuid::Uuid;

use crate::{
    base_modal,
    error::{Error, Result},
    AppState,
};

use super::{render_settings_nav, ServerId, SettingsTab};

fn render_form(server_id: Uuid) -> Markup {
    base_modal(html!(
        (render_settings_nav(server_id, SettingsTab::General))
        form hx-put={"/servers/"(server_id)"/settings"} {
            label class="form-control m-auto w-full max-w-xs" {
                .label { .label-text { "Server name" } }
                input type="text" name="name" class="input input-bordered w-full max-w-xs";
            }
            .modal-action {
                button
                  type="button"
                  class="btn btn-error"
                  hx-delete={"/servers/"(server_id)}
                  hx-confirm={"Are you sure you want to delete?"}
                  hx-swap="none"
                  { "Delete" }
                button type="submit" class="btn btn-primary" { "Update" }
            }
        }
    ))
}

pub async fn open_general_page(Path(ServerId { server_id }): Path<ServerId>) -> impl IntoResponse {
    (
        HxResponseTrigger::normal(["open-main-modal"]),
        render_form(server_id),
    )
}

#[derive(Deserialize)]
pub struct UpdatedServer {
    name: String,
}
pub async fn update_server(
    State(state): State<AppState>,
    Path(ServerId { server_id }): Path<ServerId>,
    Form(updated_server): Form<UpdatedServer>,
) -> Result<impl IntoResponse> {
    let mut transaction = state.db.begin().await?;
    let rows_affected = query!(
        r#"UPDATE servers SET name = $1 WHERE id = $2"#,
        updated_server.name,
        server_id,
    )
    .execute(&mut *transaction)
    .await?;
    if rows_affected.rows_affected() != 1 {
        return Err(Error::DatabaseActionFailed);
    }
    transaction.commit().await?;

    Ok((
        HxResponseTrigger::normal(["get-server-list"]),
        render_form(server_id),
    ))
}
