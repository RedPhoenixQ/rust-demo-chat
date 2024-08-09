use axum::{
    extract::{Path, Request, State},
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

mod error;

use crate::{auth::Auth, base_tempalte, utils::MyUuidExt, AppState};
use error::{Error, Result};

struct Message {
    id: Uuid,
    content: String,
    updated: PrimitiveDateTime,
    author: Uuid,
    author_name: String,
}

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

pub fn router(state: AppState) -> Router<AppState> {
    let is_member = from_fn_with_state(state.clone(), is_user_member_of_server);
    // FIXME: Implement admin check when database supports it
    let is_channel_admin = from_fn_with_state(state.clone(), is_user_member_of_server);
    Router::new()
        .route(
            "/servers/:server_id/channels/:channel_id",
            routing::delete(delete_channel)
                .layer(is_channel_admin.clone())
                .get(get_chat_page)
                .post(send_message),
        )
        .route(
            "/servers/:server_id/channels",
            routing::post(create_channel)
                .layer(is_channel_admin)
                .get(get_chat_page),
        )
        .route(
            "/servers/:server_id/channels_list",
            routing::get(get_channels),
        )
        .layer(is_member)
}

async fn is_user_member_of_server(
    State(state): State<AppState>,
    Auth { id: user_id }: Auth,
    Path(ServerId { server_id }): Path<ServerId>,
    request: Request,
    next: Next,
) -> Result<impl IntoResponse> {
    match sqlx::query!(
        r#"SELECT EXISTS(SELECT * FROM users_member_of_servers WHERE "user" = $1 AND server = $2) as "is_member!""#,
        user_id,
        server_id,
    )
    .fetch_one(&state.db).await?.is_member {
        true => Ok(next.run(request).await),
        false => Ok(StatusCode::UNAUTHORIZED.into_response())
    }
}

#[derive(Deserialize)]
struct SentMessage {
    content: String,
}
async fn send_message(
    State(state): State<AppState>,
    Auth { id: user_id }: Auth,
    HxRequest(hx_req): HxRequest,
    HxBoosted(hx_boosted): HxBoosted,
    Path(ChannelId { channel_id }): Path<ChannelId>,
    Path(ServerId { server_id }): Path<ServerId>,
    Form(sent_msg): Form<SentMessage>,
) -> Result<impl IntoResponse> {
    let new_id = Uuid::now_v7();
    let rows_affected = query!(
        r#"INSERT INTO messages (id, content, channel, author) VALUES ($1, $2, $3, $4)"#,
        new_id,
        sent_msg.content,
        channel_id,
        user_id
    )
    .execute(&state.db)
    .await?;

    if rows_affected.rows_affected() != 1 {
        return Err(Error::DatabaseActionFailed);
    }

    Ok(if hx_req && !hx_boosted {
        let msg = query_as!(
            Message,
            r#"SELECT m.id, m.content, m.updated, m.author, u.name as author_name 
        FROM messages AS m
        JOIN chat_users AS u ON u.id = m.author
        WHERE m.id = $1 LIMIT 1"#,
            new_id,
        )
        .fetch_one(&state.db)
        .await?;

        render_message(msg, user_id).into_response()
    } else {
        fetch_render_chat_page(&state.db, server_id, Some(channel_id), user_id)
            .await
            .into_response()
    })
}

async fn get_chat_page(
    State(state): State<AppState>,
    Auth { id: user_id }: Auth,
    Path(MaybeChannelId { channel_id }): Path<MaybeChannelId>,
    Path(ServerId { server_id }): Path<ServerId>,
) -> Result<impl IntoResponse> {
    fetch_render_chat_page(&state.db, server_id, channel_id, user_id).await
}

async fn get_channels(
    State(state): State<AppState>,
    Path(ServerId { server_id }): Path<ServerId>,
    Path(MaybeChannelId { channel_id }): Path<MaybeChannelId>,
) -> Result<impl IntoResponse> {
    fetch_render_channel_list(&state.db, server_id, channel_id).await
}

#[derive(Deserialize)]
struct NewChannel {
    name: String,
}
async fn create_channel(
    State(state): State<AppState>,
    Path(ServerId { server_id }): Path<ServerId>,
    Form(new_channel): Form<NewChannel>,
) -> Result<impl IntoResponse> {
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
        render_new_channel_dialog_form(server_id),
    ))
}

async fn delete_channel(
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

async fn fetch_render_chat_page(
    pool: &PgPool,
    server_id: Uuid,
    channel_id: Option<Uuid>,
    user_id: Uuid,
) -> Result<impl IntoResponse> {
    let (server_list, channel_list, messages_list) = try_join!(
        fetch_render_server_list(pool, user_id, server_id),
        fetch_render_channel_list(pool, server_id, channel_id),
        async {
            Ok(if let Some(channel_id) = channel_id {
                Some(fetch_render_message_list(pool, channel_id, user_id).await?)
            } else {
                None
            })
        }
    )?;

    Ok(base_tempalte(html!(
        main class="grid max-h-screen min-h-screen grid-rows-1 px-4 py-2" style="grid-template-columns: auto auto 1fr;" {
            (server_list)
            (channel_list)
            #chat-wrapper.grid style="grid-template-rows: 1fr auto" {
                @if let Some(messages_list) = messages_list {
                    (messages_list)
                    form #message-form.flex.items-end.gap-2
                        method="POST"
                        action=""
                        hx-post=""
                        hx-swap="afterbegin"
                        hx-target="#messages"
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

async fn fetch_render_server_list(
    pool: &PgPool,
    user_id: Uuid,
    active_server: Uuid,
) -> Result<Markup> {
    let servers = query!(
        r#"SELECT s.id, s.name
    FROM servers AS s
    WHERE EXISTS (
        SELECT * FROM users_member_of_servers WHERE "user" = $1
    )"#,
        user_id,
    )
    .fetch_all(pool)
    .await?;

    Ok(html!(
        ul.menu.bg-base-200.rounded-box #server-list {
            @for server in servers {
                li {
                    a.active[active_server == server.id]
                        href={"/servers/"(server.id)"/channels"} {
                        (server.name)
                    }
                }
            }
        }
    ))
}

async fn fetch_render_channel_list(
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
            hx-get={"/servers/"(server_id)"/channels_list"}
            hx-trigger="get-channel-list from:body"
            hx-swap="outerHTML"
        {
            li.menu-title {
                button class="btn btn-ghost btn-sm" onclick="createChannelDialog.showModal()" { "New" }
            }
            @for channel in channels {
                li {
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
        dialog #createChannelDialog.modal hx-on-close-modal="this.close()" {
            .modal-box {
                form method="dialog" {
                    button class="btn btn-circle btn-ghost btn-sm absolute right-2 top-2"
                        type="submit"
                        aria-label="close"
                        { "✕" }
                }
                (render_new_channel_dialog_form(server_id))
            }
            form.modal-backdrop method="dialog" {
                button type="submit" { "Close" }
            }
        }
    ))
}

fn render_new_channel_dialog_form(server_id: Uuid) -> Markup {
    html!(
        form method="post"
            hx-post={"/servers/"(server_id)"/channels"}
            hx-swap="outerHTML"
        {
            label class="form-control m-auto w-full max-w-xs" {
                .label { .label-text { "Channel name" } }
                input type="text" name="name" class="input input-bordered w-full max-w-xs";
            }
            .modal-action {
                button type="submit" class="btn btn-primary" { "Create" }
            }
        }
    )
}

async fn fetch_render_message_list(
    pool: &PgPool,
    channel_id: Uuid,
    user_id: Uuid,
) -> Result<Markup> {
    let messages = query_as!(
        Message,
        r#"SELECT m.id, m.content, m.updated, m.author, u.name as author_name 
    FROM messages AS m
    JOIN chat_users AS u ON u.id = m.author
    WHERE m.channel = $1
    ORDER BY m.id DESC"#,
        channel_id,
    )
    .fetch_all(pool)
    .await?;

    Ok(html!(
        ol #messages class="flex flex-col-reverse overflow-y-auto" {
            @for msg in messages {
                (render_message(msg, user_id)?)
            }
        }
    ))
}

fn render_message(msg: Message, user_id: Uuid) -> Result<Markup> {
    let is_author = msg.author == user_id;
    Ok(html!(
        li.chat
            .chat-end[is_author]
            .chat-start[!is_author]
            data-id=(msg.id)
        {
            .chat-header {
                (msg.author_name) " "
                @let time = msg.id.get_datetime().ok_or(Error::NoTimestampFromUuid { id: msg.id })?;
                time.text-xs.opacity-50 datetime=(time.format(&Rfc3339)?) {
                    // TODO: Make this a human readable relative time (one minute ago, ...)
                    (time.to_string())
                }
            }
            .chat-bubble.chat-bubble-primary[is_author] {
                (msg.content)
            }
            .chat-footer {
                (msg.updated.to_string())
            }
        }
    ))
}
