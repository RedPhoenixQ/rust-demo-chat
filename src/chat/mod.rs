use axum::{
    extract::{Path, Query, Request, State},
    http::StatusCode,
    middleware::{from_fn_with_state, Next},
    response::IntoResponse,
    routing, Form, Router,
};
use axum_htmx::{HxBoosted, HxRequest, HxResponseTrigger};
use maud::{html, Markup};
use serde::Deserialize;
use sqlx::{query, query_as, PgPool};
use time::{format_description::well_known::Rfc3339, PrimitiveDateTime};
use tokio::try_join;
use uuid::Uuid;

mod channels;
mod error;
mod messages;
mod servers;

pub use messages::{create_listener, MessageRegistry};

use crate::{auth::Auth, base_modal, base_tempalte, header, utils::MyUuidExt, AppState};
use error::{Error, Result};

#[derive(Deserialize)]
struct ChannelId {
    channel_id: Uuid,
}
#[derive(Deserialize)]
struct MaybeChannelId {
    channel_id: Option<Uuid>,
}

#[derive(Deserialize)]
struct ServerId {
    server_id: Uuid,
}
#[derive(Deserialize)]
struct MaybeServerId {
    server_id: Option<Uuid>,
}

pub fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .route(
            "/servers/:server_id/:channel_id/messages",
            routing::post(messages::send_message),
        )
        .route(
            "/servers/:server_id/:channel_id/more_messages",
            routing::get(messages::get_more_messages),
        )
        .route(
            "/servers/:server_id/:channel_id/events",
            routing::get(messages::message_event_stream),
        )
        .route(
            "/servers/:server_id/:channel_id",
            routing::get(get_chat_page).delete(channels::delete_channel),
        )
        .route(
            "/servers/:server_id/channels",
            routing::get(channels::get_channels).post(channels::create_channel),
        )
        .route(
            "/servers/:server_id",
            routing::get(get_chat_page).delete(servers::delete_server),
        )
        .layer(from_fn_with_state(state.clone(), is_user_member_of_server))
        .nest(
            "/servers/:server_id/settings",
            // NOTE: Does not need member check because it check edit rights
            servers::settings::router(state.clone()),
        )
        .route(
            "/servers",
            routing::get(servers::get_servers).post(servers::create_server),
        )
        .route("/", routing::get(get_chat_page))
}

async fn is_user_member_of_server(
    State(state): State<AppState>,
    Auth { id: user_id }: Auth,
    Path(ServerId { server_id }): Path<ServerId>,
    request: Request,
    next: Next,
) -> Result<impl IntoResponse> {
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

async fn get_chat_page(
    State(state): State<AppState>,
    Auth { id: user_id }: Auth,
    Path(MaybeChannelId { channel_id }): Path<MaybeChannelId>,
    Path(MaybeServerId { server_id }): Path<MaybeServerId>,
) -> Result<impl IntoResponse> {
    fetch_render_chat_page(&state.db, server_id, channel_id, user_id).await
}
async fn fetch_render_chat_page(
    pool: &PgPool,
    server_id: Option<Uuid>,
    channel_id: Option<Uuid>,
    user_id: Uuid,
) -> Result<impl IntoResponse> {
    let (server_list, channel_list, messages_list) = try_join!(
        servers::fetch_render_server_list(pool, user_id, server_id),
        async {
            Ok(if let Some(server_id) = server_id {
                Some(channels::fetch_render_channel_list(pool, server_id, channel_id).await?)
            } else {
                None
            })
        },
        async {
            Ok(
                if let (Some(server_id), Some(channel_id)) = (server_id, channel_id) {
                    Some((
                        messages::fetch_render_message_list(pool, server_id, channel_id, user_id)
                            .await?,
                        (server_id, channel_id),
                    ))
                } else {
                    None
                },
            )
        }
    )?;

    Ok(base_tempalte(html!(
        main class="grid max-h-screen min-h-screen px-4 py-2" style="grid-template-columns: auto auto 1fr; grid-template-rows: auto minmax(0,1fr)" {
            .col-span-full { (header()) }
            (server_list)
            (channel_list.unwrap_or(html!(ul #channels-list {})))
            #chat-wrapper.grid style="grid-template-rows: 1fr auto" {
                @if let Some((messages_list, (server_id, channel_id))) = messages_list {
                    (messages_list)
                    form #message-form.flex.items-end.gap-2
                        hx-post={"/servers/"(server_id)"/"(channel_id)"/messages"}
                        hx-swap="none"
                        "hx-on::after-request"="if (event.detail.successful) this.reset()"
                    {
                        input.input.input-bordered.grow name="content" placeholder="Type here...";
                        button.btn.btn-primary { "Send" }
                    }
                }
            }
        }
    )))
}
