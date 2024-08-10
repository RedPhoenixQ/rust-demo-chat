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

mod error;
pub mod live_messages;

use crate::{auth::Auth, base_tempalte, header, utils::MyUuidExt, AppState};
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
#[derive(Deserialize)]
struct MaybeServerId {
    server_id: Option<Uuid>,
}

pub fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .route(
            "/servers/:server_id/:channel_id/messages",
            routing::post(send_message),
        )
        .route(
            "/servers/:server_id/:channel_id/more_messages",
            routing::get(get_more_messages),
        )
        .route(
            "/servers/:server_id/:channel_id/events",
            routing::get(message_event_stream),
        )
        .route(
            "/servers/:server_id/:channel_id",
            routing::get(get_chat_page).delete(delete_channel),
        )
        .route(
            "/servers/:server_id/channels",
            routing::get(get_channels).post(create_channel),
        )
        .route(
            "/servers/:server_id",
            routing::get(get_chat_page).delete(delete_server),
        )
        .layer(from_fn_with_state(state.clone(), is_user_member_of_server))
        .route("/servers", routing::get(get_servers).post(create_server))
        .route("/", routing::get(get_chat_page))
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

async fn message_event_stream(
    State(state): State<AppState>,
    Auth { id: user_id }: Auth,
    Path(ChannelId { channel_id }): Path<ChannelId>,
) -> Result<
    axum::response::sse::Sse<
        impl tokio_stream::Stream<
            Item = std::result::Result<axum::response::sse::Event, std::convert::Infallible>,
        >,
    >,
> {
    let (tx, rx) = tokio::sync::oneshot::channel();

    state
        .message_live
        .register
        .send((channel_id, (user_id, tx)))
        .await
        .map_err(|_| Error::SSEChannelRegistrationChannelFailed)?;

    let stream = tokio_stream::wrappers::UnboundedReceiverStream::new(
        rx.await
            .map_err(|_| Error::SSERegistationDidNotRecvChannel)?,
    );

    Ok(axum::response::sse::Sse::new(stream).keep_alive(
        axum::response::sse::KeepAlive::new()
            .interval(std::time::Duration::from_secs(5))
            .text("heartbeat"),
    ))
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
        html!().into_response()
    } else {
        fetch_render_chat_page(&state.db, Some(server_id), Some(channel_id), user_id)
            .await
            .into_response()
    })
}

async fn get_chat_page(
    State(state): State<AppState>,
    Auth { id: user_id }: Auth,
    Path(MaybeChannelId { channel_id }): Path<MaybeChannelId>,
    Path(MaybeServerId { server_id }): Path<MaybeServerId>,
) -> Result<impl IntoResponse> {
    fetch_render_chat_page(&state.db, server_id, channel_id, user_id).await
}

#[derive(Deserialize)]
struct MoreOpts {
    before: Uuid,
}
async fn get_more_messages(
    State(state): State<AppState>,
    Auth { id: user_id }: Auth,
    Query(MoreOpts { before }): Query<MoreOpts>,
    Path(ChannelId { channel_id }): Path<ChannelId>,
    Path(ServerId { server_id }): Path<ServerId>,
) -> Result<impl IntoResponse> {
    let messages = query_as!(
        Message,
        r#"SELECT m.id, m.content, m.updated, m.author, u.name as author_name 
    FROM messages AS m
    JOIN chat_users AS u ON u.id = m.author
    WHERE m.channel = $1 AND m.id < $2
    ORDER BY m.id DESC
    LIMIT 25"#,
        channel_id,
        before
    )
    .fetch_all(&state.db)
    .await?;

    Ok(render_messages(&messages, server_id, channel_id, user_id)?)
}

async fn get_channels(
    State(state): State<AppState>,
    Path(ServerId { server_id }): Path<ServerId>,
    Query(MaybeChannelId { channel_id }): Query<MaybeChannelId>,
) -> Result<impl IntoResponse> {
    fetch_render_channel_list(&state.db, server_id, channel_id).await
}

async fn get_servers(
    State(state): State<AppState>,
    Auth { id: user_id }: Auth,
    Query(MaybeServerId { server_id }): Query<MaybeServerId>,
) -> Result<impl IntoResponse> {
    fetch_render_server_list(&state.db, user_id, server_id).await
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
        render_new_channel_form_inners(),
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

#[derive(Deserialize)]
struct NewServer {
    name: String,
}
async fn create_server(
    State(state): State<AppState>,
    Auth { id: user_id }: Auth,
    Form(new_server): Form<NewServer>,
) -> Result<impl IntoResponse> {
    let mut transaction = state.db.begin().await?;

    let new_id = Uuid::now_v7();
    let rows_affected = query!(
        r#"INSERT INTO servers (id, name) VALUES ($1, $2)"#,
        new_id,
        new_server.name,
    )
    .execute(&mut *transaction)
    .await?;
    if rows_affected.rows_affected() != 1 {
        return Err(Error::DatabaseActionFailed);
    }
    let rows_affected = query!(
        r#"INSERT INTO users_member_of_servers ("user", server) VALUES ($1, $2)"#,
        user_id,
        new_id,
    )
    .execute(&mut *transaction)
    .await?;
    if rows_affected.rows_affected() != 1 {
        return Err(Error::DatabaseActionFailed);
    }
    transaction.commit().await?;

    Ok((
        HxResponseTrigger::normal(["close-modal", "get-server-list"]),
        render_new_server_form_inners(),
    ))
}

async fn delete_server(
    State(state): State<AppState>,
    Path(ServerId { server_id }): Path<ServerId>,
) -> Result<impl IntoResponse> {
    let rows_affected = query!(r#"DELETE FROM servers WHERE id = $1"#, server_id)
        .execute(&state.db)
        .await?;

    if rows_affected.rows_affected() != 1 {
        return Err(Error::DatabaseActionFailed);
    }

    Ok(html!())
}

async fn fetch_render_chat_page(
    pool: &PgPool,
    server_id: Option<Uuid>,
    channel_id: Option<Uuid>,
    user_id: Uuid,
) -> Result<impl IntoResponse> {
    let (server_list, channel_list, messages_list) = try_join!(
        fetch_render_server_list(pool, user_id, server_id),
        async {
            Ok(if let Some(server_id) = server_id {
                Some(fetch_render_channel_list(pool, server_id, channel_id).await?)
            } else {
                None
            })
        },
        async {
            Ok(
                if let (Some(server_id), Some(channel_id)) = (server_id, channel_id) {
                    Some(fetch_render_message_list(pool, server_id, channel_id, user_id).await?)
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
                @if let Some(messages_list) = messages_list {
                    (messages_list)
                    form #message-form.flex.items-end.gap-2
                        method="POST"
                        action="messages"
                        hx-post="messages"
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

async fn fetch_render_server_list(
    pool: &PgPool,
    user_id: Uuid,
    active_server: Option<Uuid>,
) -> Result<Markup> {
    let servers = query!(
        r#"SELECT s.id, s.name
    FROM servers AS s
    WHERE EXISTS (
        SELECT * FROM users_member_of_servers 
        WHERE "user" = $1 AND server = s.id
    )"#,
        user_id,
    )
    .fetch_all(pool)
    .await?;

    Ok(html!(
        ul #server-list
            class="menu rounded-box bg-base-200"
            hx-get={"/servers/list?server_id="(active_server.unwrap_or_default())}
            hx-trigger="get-server-list from:body"
            hx-swap="outerHTML"
        {
            li.menu-title {
                button class="btn btn-ghost btn-sm" onclick="createServerDialog.showModal()" { "New" }
            }
            @for server in servers {
                li #{"server-"(server.id)} {
                    div.active[active_server.is_some_and(|id| id == server.id)].flex {
                        a.grow href={"/servers/"(server.id)} {
                            (server.name)
                        }
                        button
                            class="btn btn-circle btn-ghost btn-sm hover:btn-error"
                            hx-delete={"/servers/"(server.id)}
                            hx-confirm={"Are you sure you want to delete '"(server.name)"'?"}
                            hx-target="closest li"
                            hx-swap="outerHTML"
                            { "✕" }
                    }
                }
            }
        }
        dialog #createServerDialog.modal hx-on-close-modal="this.close()" {
            .modal-box {
                form method="dialog" hx-disable {
                    button class="btn btn-circle btn-ghost btn-sm absolute right-2 top-2"
                        type="submit"
                        aria-label="close"
                        { "✕" }
                }
                form method="post" hx-post="/servers" {
                    (render_new_server_form_inners())
                }
            }
            form.modal-backdrop method="dialog" hx-disable {
                button type="submit" { "Close" }
            }
        }
    ))
}

fn render_new_server_form_inners() -> Markup {
    html!(
        label class="form-control m-auto w-full max-w-xs" {
            .label { .label-text { "Channel name" } }
            input type="text" name="name" class="input input-bordered w-full max-w-xs";
        }
        .modal-action {
            button type="submit" class="btn btn-primary" { "Create" }
        }
    )
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
            hx-get={"/servers/"(server_id)"/list?channel_id="(active_channel.unwrap_or_default())}
            hx-trigger="get-channel-list from:body"
            hx-swap="outerHTML"
        {
            li.menu-title {
                button class="btn btn-ghost btn-sm" onclick="createChannelDialog.showModal()" { "New" }
            }
            @for channel in channels {
                li #{"channel-"(channel.id)} {
                    div.active[active_channel.is_some_and(|id| id == channel.id)].flex {
                        a.grow href={"/servers/"(server_id)"/"(channel.id)} {
                            (channel.name)
                        }
                        button
                            class="btn btn-circle btn-ghost btn-sm hover:btn-error"
                            hx-delete={"/servers/"(server_id)"/"(channel.id)}
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
                form method="dialog" hx-disable {
                    button class="btn btn-circle btn-ghost btn-sm absolute right-2 top-2"
                        type="submit"
                        aria-label="close"
                        { "✕" }
                }
                form method="post" hx-post={"/servers/"(server_id)"/channels"} {
                    (render_new_channel_form_inners())
                }
            }
            form.modal-backdrop method="dialog" hx-disable {
                button type="submit" { "Close" }
            }
        }
    ))
}

fn render_new_channel_form_inners() -> Markup {
    html!(
        label class="form-control m-auto w-full max-w-xs" {
            .label { .label-text { "Channel name" } }
            input type="text" name="name" class="input input-bordered w-full max-w-xs";
        }
        .modal-action {
            button type="submit" class="btn btn-primary" { "Create" }
        }
    )
}

async fn fetch_render_message_list(
    pool: &PgPool,
    server_id: Uuid,
    channel_id: Uuid,
    user_id: Uuid,
) -> Result<Markup> {
    let messages = query_as!(
        Message,
        r#"SELECT m.id, m.content, m.updated, m.author, u.name as author_name 
    FROM messages AS m
    JOIN chat_users AS u ON u.id = m.author
    WHERE m.channel = $1
    ORDER BY m.id DESC
    LIMIT 25"#,
        channel_id,
    )
    .fetch_all(pool)
    .await?;

    Ok(html!(
        ol #messages class="flex flex-col-reverse overflow-y-auto"
            hx-ext="sse"
            sse-connect={"/servers/"(server_id)"/"(channel_id)"/events"}
            sse-swap="message"
            hx-swap="afterbegin"
        {
            (render_messages(&messages,server_id, channel_id, user_id)?)
        }
    ))
}

fn render_messages(
    messages: &[Message],
    server_id: Uuid,
    channel_id: Uuid,
    user_id: Uuid,
) -> Result<Markup> {
    Ok(html!(
        @for msg in messages {
            (render_message(msg, &user_id, false)?)
        }
        @if let Some(last_msg) = messages.last() {
            div class="loading loading-dots mx-auto mt-auto pt-8"
                hx-trigger="intersect once"
                hx-swap="outerHTML"
                hx-get={"/servers/"(server_id)"/"(channel_id)"/more_messages?before="(last_msg.id)}
                {}
        }
    ))
}

fn render_message(msg: &Message, user_id: &Uuid, swap_oob: bool) -> Result<Markup> {
    let is_author = &msg.author == user_id;
    Ok(html!(
        li.chat
            .chat-end[is_author]
            .chat-start[!is_author]
            #{"msg-"(msg.id)}
            hx-swap-oob=[swap_oob.then_some("true")]
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
