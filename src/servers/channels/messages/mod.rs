use axum::{
    extract::{Path, Query, State},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse,
    },
    routing, Form, Router,
};
use chrono::{NaiveDateTime, Utc};
use maud::{html, Markup};
use relativetime::RelativeTime;
use serde::Deserialize;
use sqlx::{query, query_as, PgPool};
use std::convert::Infallible;
use uuid::Uuid;

pub mod live;

use crate::{
    auth::Auth,
    error::{Error, Result},
    servers::ServerId,
    utils::MyUuidExt,
    AppState,
};

use super::ChannelId;

#[derive(Deserialize)]
struct MessageId {
    message_id: Uuid,
}

struct Message {
    id: Uuid,
    content: String,
    updated: NaiveDateTime,
    author: Uuid,
    author_name: String,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", routing::post(send_message))
        .route(
            "/:message_id",
            routing::get(get_message)
                .post(edit_message)
                .delete(delete_message),
        )
        .route("/:message_id/editable", routing::get(edit_message))
        .route("/more", routing::get(get_more_messages))
        .route("/events", routing::get(message_event_stream))
}

async fn message_event_stream(
    State(state): State<AppState>,
    Auth { id: user_id }: Auth,
    Path(ChannelId { channel_id }): Path<ChannelId>,
    Path(ServerId { server_id }): Path<ServerId>,
) -> Result<Sse<impl tokio_stream::Stream<Item = std::result::Result<Event, Infallible>>>> {
    let (tx, rx) = tokio::sync::oneshot::channel();

    state
        .message_live
        .register
        .send((
            live::ChannelIds {
                channel_id,
                server_id,
            },
            (user_id, tx),
        ))
        .await
        .map_err(|_| Error::SSEChannelRegistrationChannelFailed)?;

    let stream = tokio_stream::wrappers::UnboundedReceiverStream::new(
        rx.await
            .map_err(|_| Error::SSERegistationDidNotRecvChannel)?,
    );

    Ok(Sse::new(stream).keep_alive(
        KeepAlive::new()
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
    Path(ChannelId { channel_id }): Path<ChannelId>,
    Form(sent_msg): Form<SentMessage>,
) -> Result<impl IntoResponse> {
    // FIXME: Check if user has access to channel
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

    Ok(html!())
}

async fn get_message(
    State(state): State<AppState>,
    Auth { id: user_id }: Auth,
    Path(MessageId { message_id }): Path<MessageId>,
    Path(ChannelId { channel_id }): Path<ChannelId>,
    Path(ServerId { server_id }): Path<ServerId>,
) -> Result<impl IntoResponse> {
    // FIXME: Allow for getting any message user has access to, not just those they authored
    let msg = query_as!(
        Message,
        r#"SELECT m.id, m.content, m.updated, m.author, u.name as author_name 
      FROM messages AS m
      JOIN chat_users AS u ON u.id = m.author
      WHERE m.id = $1 AND m.author = $2"#,
        message_id,
        user_id
    )
    .fetch_one(&state.db)
    .await?;
    return Ok(render_message(
        &msg,
        &user_id,
        &channel_id,
        &server_id,
        false,
    )?);
}

#[derive(Deserialize)]
struct UpdatedMessage {
    content: String,
}
async fn edit_message(
    State(state): State<AppState>,
    Auth { id: user_id }: Auth,
    Path(MessageId { message_id }): Path<MessageId>,
    Path(ChannelId { channel_id }): Path<ChannelId>,
    Path(ServerId { server_id }): Path<ServerId>,
    updated_msg: Option<Form<UpdatedMessage>>,
) -> Result<impl IntoResponse> {
    // FIXME: Check if allowed to edit
    let Some(Form(updated_msg)) = updated_msg else {
        let msg = query_as!(
            Message,
            r#"SELECT m.id, m.content, m.updated, m.author, u.name as author_name 
          FROM messages AS m
          JOIN chat_users AS u ON u.id = m.author
          WHERE m.id = $1 AND m.author = $2"#,
            message_id,
            user_id
        )
        .fetch_one(&state.db)
        .await?;
        return Ok(render_message_for_edit(&msg, &server_id, &channel_id)?);
    };

    let rows_affected = query!(
        r#"UPDATE messages SET updated = NOW(), content = $1 WHERE id = $2 AND author = $3"#,
        updated_msg.content,
        message_id,
        user_id
    )
    .execute(&state.db)
    .await?;

    if rows_affected.rows_affected() != 1 {
        return Err(Error::DatabaseActionFailed);
    }

    Ok(html!())
}

async fn delete_message(
    State(state): State<AppState>,
    Path(MessageId { message_id }): Path<MessageId>,
) -> Result<impl IntoResponse> {
    // FIXME: Check if allowed to delete
    let rows_affected = query!(r#"DELETE FROM messages WHERE id = $1"#, message_id)
        .execute(&state.db)
        .await?;

    if rows_affected.rows_affected() != 1 {
        return Err(Error::DatabaseActionFailed);
    }

    Ok(html!())
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

    render_messages(
        &messages,
        server_id,
        channel_id,
        user_id,
        messages.len() >= 25,
    )
}

pub async fn fetch_render_message_list(
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
            sse-connect={"/servers/"(server_id)"/channels/"(channel_id)"/messages/events"}
            sse-swap="message"
            hx-swap="afterbegin"
        {
            (render_messages(&messages,server_id, channel_id, user_id, messages.len() >= 25)?)
        }
    ))
}

fn render_messages(
    messages: &[Message],
    server_id: Uuid,
    channel_id: Uuid,
    user_id: Uuid,
    should_load_more: bool,
) -> Result<Markup> {
    Ok(html!(
        @for msg in messages {
            (render_message(msg, &user_id, &channel_id, &server_id, false)?)
        }
        @if let Some(last_msg) = messages.last() {
            @if should_load_more {
                div class="loading loading-dots mx-auto mt-auto pt-8"
                    hx-trigger="intersect once"
                    hx-swap="outerHTML"
                    hx-get={"/servers/"(server_id)"/channels/"(channel_id)"/messages/more?before="(last_msg.id)}
                    {}
            }
        }
    ))
}

fn render_message(
    msg: &Message,
    user_id: &Uuid,
    channel_id: &Uuid,
    server_id: &Uuid,
    swap_oob: bool,
) -> Result<Markup> {
    let is_author = &msg.author == user_id;
    Ok(html!(
        li.group.chat
            .chat-end[is_author]
            .chat-start[!is_author]
            #{"msg-"(msg.id)}
            hx-swap-oob=[swap_oob.then_some("true")]
        {
            .chat-header {
                @let created_at = msg.id.get_datetime().ok_or(Error::NoTimestampFromUuid { id: msg.id })?;
                @if msg.updated.and_utc() > created_at {
                    span.italic.text-xs.opacity-50 {
                        "Edited "
                    }
                }
                (msg.author_name) " "
                time.text-xs.opacity-50 datetime=(created_at.to_rfc3339()) {
                    (created_at.signed_duration_since(Utc::now()).to_relative())
                }
            }
            .chat-bubble.chat-bubble-primary[is_author] {
                (msg.content)
            }
            .chat-footer.transition-opacity hx-target="closest li" hx-swap="outerHTML" {
                @if is_author {
                    button
                        class="link mr-2 opacity-0 group-hover:opacity-100"
                        hx-get={"/servers/"(server_id)"/channels/"(channel_id)"/messages/"(msg.id)"/editable"}
                        { "Edit" }
                }
                button
                    class="link link-error opacity-0 group-hover:opacity-100"
                    hx-delete={"/servers/"(server_id)"/channels/"(channel_id)"/messages/"(msg.id)}
                    hx-confirm="Are you sure?"
                    { "Delete" }
            }
        }
    ))
}

fn render_message_for_edit(msg: &Message, server_id: &Uuid, channel_id: &Uuid) -> Result<Markup> {
    Ok(html!(
        li.group.chat.chat-end
            #{"msg-"(msg.id)}
        {
            .chat-header {
                @let created_at = msg.id.get_datetime().ok_or(Error::NoTimestampFromUuid { id: msg.id })?;
                @if msg.updated.and_utc() > created_at {
                    span.italic.text-xs.opacity-50 {
                        "Edited "
                    }
                }
                (msg.author_name) " "
                time.text-xs.opacity-50 datetime=(created_at.to_rfc3339()) {
                    (created_at.signed_duration_since(Utc::now()).to_relative())
                }
            }
            form.chat-bubble.chat-bubble-primary
                hx-post={"/servers/"(server_id)"/channels/"(channel_id)"/messages/"(msg.id)}
            {
                input class="input text-base-content" name="content" value=(msg.content);
            }
            .chat-footer hx-target="closest li" hx-swap="outerHTML" {
                button
                    class="link mr-2"
                    hx-get={"/servers/"(server_id)"/channels/"(channel_id)"/messages/"(msg.id)}
                    { "Cancel" }
                button
                    class="link link-error"
                    hx-delete={"/servers/"(server_id)"/channels/"(channel_id)"/messages/"(msg.id)}
                    hx-confirm="Are you sure?"
                    { "Delete" }
            }
        }
    ))
}
