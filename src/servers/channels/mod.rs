use axum::{
    extract::{Path, Query, State},
    response::IntoResponse,
    routing, Form, Router,
};
use axum_htmx::HxResponseTrigger;
use maud::{html, Markup};
use serde::Deserialize;
use sqlx::{query, PgPool};
use uuid::Uuid;

use crate::{
    base_modal,
    chat::get_chat_page,
    error::{Error, Result},
    AppState,
};

use super::ServerId;

pub mod messages;

#[derive(Deserialize)]
pub struct ChannelId {
    pub channel_id: Uuid,
}
#[derive(Deserialize)]
pub struct MaybeChannelId {
    pub channel_id: Option<Uuid>,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .nest("/:channel_id/messages", messages::router())
        .route(
            "/:channel_id",
            routing::get(get_chat_page).delete(delete_channel),
        )
        .route("/", routing::get(get_channels).post(create_channel))
}

#[derive(Deserialize)]
pub struct NewChannel {
    name: String,
}
pub async fn create_channel(
    State(state): State<AppState>,
    Path(ServerId { server_id }): Path<ServerId>,
    new_channel: Option<Form<NewChannel>>,
) -> Result<impl IntoResponse> {
    fn render_new_channel_form_inners(server_id: Uuid) -> Markup {
        base_modal(html!(
            form method="post" hx-post={"/servers/"(server_id)"/channels"} {
                label class="form-control m-auto w-full max-w-xs" {
                    .label { .label-text { "Channel name" } }
                    input type="text" name="name" class="input input-bordered w-full max-w-xs";
                }
                .modal-action {
                    button type="submit" class="btn btn-primary" { "Create" }
                }
            }
        ))
    }

    let Some(Form(new_channel)) = new_channel else {
        return Ok((
            HxResponseTrigger::normal(["open-main-modal"]),
            render_new_channel_form_inners(server_id),
        ));
    };

    let new_id = Uuid::now_v7();
    let rows_affected = query!(
        r#"INSERT INTO channels (id, name, server) VALUES ($1, $2, $3)"#,
        new_id,
        new_channel.name,
        server_id,
    )
    .execute(&state.db)
    .await?;

    if rows_affected.rows_affected() != 1 {
        return Err(Error::DatabaseActionFailed);
    }

    Ok((
        HxResponseTrigger::normal(["close-modal", "get-channel-list"]),
        render_new_channel_form_inners(server_id),
    ))
}

pub async fn delete_channel(
    State(state): State<AppState>,
    Path(ChannelId { channel_id }): Path<ChannelId>,
) -> Result<impl IntoResponse> {
    let rows_affected = query!(r#"DELETE FROM channels WHERE id = $1"#, channel_id)
        .execute(&state.db)
        .await?;

    if rows_affected.rows_affected() != 1 {
        return Err(Error::DatabaseActionFailed);
    }

    Ok(html!())
}

pub async fn get_channels(
    State(state): State<AppState>,
    Path(ServerId { server_id }): Path<ServerId>,
    Query(MaybeChannelId { channel_id }): Query<MaybeChannelId>,
) -> Result<impl IntoResponse> {
    fetch_render_channel_list(&state.db, server_id, channel_id).await
}
pub async fn fetch_render_channel_list(
    pool: &PgPool,
    server_id: Uuid,
    active_channel: Option<Uuid>,
) -> Result<Markup> {
    let channels = query!(
        r#"SELECT c.id, c.name
    FROM channels AS c
    WHERE c.server = $1"#,
        server_id,
    )
    .fetch_all(pool)
    .await?;

    Ok(html!(
        ul #channels-list
            class="menu rounded-box bg-base-200"
            hx-get={"/servers/"(server_id)"/channels?channel_id="(active_channel.unwrap_or_default())}
            hx-trigger="get-channel-list from:body"
            hx-swap="outerHTML"
        {
            li.menu-title {
                button class="btn btn-ghost btn-sm" hx-post={"/servers/"(server_id)"/channels"} hx-target="#modalInner" { "New" }
            }
            @for channel in channels {
                li #{"channel-"(channel.id)} {
                    div.active[active_channel.is_some_and(|id| id == channel.id)].flex {
                        a.grow href={"/servers/"(server_id)"/channels/"(channel.id)} {
                            (channel.name)
                        }
                        button
                            class="btn btn-circle btn-ghost btn-sm hover:btn-error"
                            hx-delete={"/servers/"(server_id)"/channels/"(channel.id)}
                            hx-confirm={"Are you sure you want to delete '"(channel.name)"'?"}
                            hx-target="closest li"
                            hx-swap="outerHTML"
                            { "✕" }
                    }
                }
            }
        }
    ))
}
