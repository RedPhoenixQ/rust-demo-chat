use axum::{
    extract::{Path, Query, State},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse,
    },
    routing, Form, Router,
};
use maud::{html, Markup};
use serde::Deserialize;
use sqlx::{query, query_as, PgPool};
use std::convert::Infallible;
use time::{format_description::well_known::Rfc3339, PrimitiveDateTime};
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

struct Message {
    id: Uuid,
    content: String,
    updated: PrimitiveDateTime,
    author: Uuid,
    author_name: String,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", routing::post(send_message))
        .route("/more", routing::get(get_more_messages))
        .route("/events", routing::get(message_event_stream))
}

async fn message_event_stream(
    State(state): State<AppState>,
    Auth { id: user_id }: Auth,
    Path(ChannelId { channel_id }): Path<ChannelId>,
) -> Result<Sse<impl tokio_stream::Stream<Item = std::result::Result<Event, Infallible>>>> {
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
            (render_message(msg, &user_id, false)?)
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
