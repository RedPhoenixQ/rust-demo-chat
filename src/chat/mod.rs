use axum::{
    extract::{Path, Request, State}, http::StatusCode, middleware::{from_fn_with_state, Next}, response::IntoResponse, routing, Form, Router
};
use axum_htmx::{HxBoosted, HxRequest};
use maud::{html, Markup};
use serde::Deserialize;
use sqlx::{query, query_as, PgPool};
use time::{format_description::well_known::Rfc3339, PrimitiveDateTime};
use uuid::Uuid;

use crate::{auth::Auth, base_tempalte, utils::MyUuidExt, AppState};

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
struct ServerId {
    server_id: Uuid,
}

pub fn router(state: AppState) -> Router<AppState> {
    let is_member = from_fn_with_state(state.clone(), is_user_member_of_server);
    Router::new()
        .route(
            "/channels/:server_id/:channel_id", 
            routing::get(get_chat_page).post(send_message).layer(is_member)
        )
}

async fn is_user_member_of_server(
    State(state): State<AppState>,
    Auth{ id: user_id }: Auth,
    Path(ServerId { server_id }): Path<ServerId>,
    request: Request,
    next: Next,
) -> impl IntoResponse {
    match sqlx::query!(
        r#"SELECT EXISTS(SELECT * FROM users_member_of_servers WHERE "user" = $1 AND server = $2) as is_member"#,
        user_id,
        server_id,
    )
    .fetch_one(&state.db).await.unwrap().is_member {
        Some(true) => Ok(next.run(request).await),
        _ => Err(StatusCode::UNAUTHORIZED) 
    }
}

#[derive(Deserialize)]
struct SentMessage {
    content: String
}
async fn send_message(
    State(state): State<AppState>,
    Auth{ id: user_id }: Auth,
    HxRequest(hx_req): HxRequest,
    HxBoosted(hx_boosted): HxBoosted,
    Path(ChannelId { channel_id }): Path<ChannelId>,
    Path(ServerId { server_id }): Path<ServerId>,
    Form(sent_msg): Form<SentMessage>,
) -> Markup {
    let new_id = Uuid::now_v7();
    let rows_affected = query!(
        r#"INSERT INTO messages (id, content, channel, author) VALUES ($1, $2, $3, $4)"#,
        new_id,
        sent_msg.content,
        channel_id,
        user_id
    ).execute(&state.db).await.unwrap();

    // TODO: Return a propper error when failing to insert row
    assert_eq!(rows_affected.rows_affected(), 1);

    if hx_req && !hx_boosted {
        let msg = query_as!(
            Message,
            r#"SELECT m.id, m.content, m.updated, m.author, u.name as author_name 
        FROM messages AS m
        JOIN chat_users AS u ON u.id = m.author
        WHERE m.id = $1"#,
            new_id,
        )
        .fetch_one(&state.db)
        .await
        .unwrap();

        render_message(msg, user_id)
    } else {
        fetch_render_chat_page(&state.db, server_id, channel_id, user_id).await
    }
}

async fn get_chat_page(
    State(state): State<AppState>,
    Auth{ id: user_id }: Auth,
    Path(ChannelId { channel_id }): Path<ChannelId>,
    Path(ServerId { server_id }): Path<ServerId>,
) -> Markup {
    fetch_render_chat_page(&state.db, server_id, channel_id, user_id).await
}

async fn fetch_render_chat_page(pool: &PgPool, server_id: Uuid, channel_id: Uuid, user_id: Uuid) -> Markup {
    base_tempalte(html!(
        main class="grid max-h-screen grid-rows-1 px-4 py-2" style="grid-template-columns: auto auto 1fr;" {
            (fetch_render_server_list(pool, user_id).await)
            (fetch_render_channel_list(pool, server_id, channel_id).await)
            #chat-wrapper.grid style="grid-template-rows: 1fr auto" {
                (fetch_render_message_list(pool, channel_id, user_id).await)
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
    ))
}

async fn fetch_render_server_list(pool: &PgPool, user_id: Uuid) -> Markup {
    let servers = query!(
        r#"SELECT s.id, s.name
    FROM servers AS s
    WHERE EXISTS (
        SELECT * FROM users_member_of_servers WHERE "user" = $1
    )"#,
        user_id,
    )
    .fetch_all(pool)
    .await
    .unwrap();

    html!(
        ul.menu.bg-base-200.rounded-box #server-list {
            @for server in servers {
                li { a href={"/channels/"(server.id)} { (server.name) } }
            }
        }
    )
}

async fn fetch_render_channel_list(pool: &PgPool, server_id: Uuid, channel_id: Uuid) -> Markup {
    let channels = query!(
        r#"SELECT c.id, c.name
    FROM channels AS c
    WHERE c.server = $1"#,
        server_id,
    )
    .fetch_all(pool)
    .await
    .unwrap();

    html!(
        ul.menu.bg-base-200.rounded-box #channels-list {
            @for channel in channels {
                li { 
                    a.active[channel.id == channel_id] 
                        href={"/channels/"(server_id)"/"(channel.id)} 
                    { (channel.name) } 
                }
            }
        }
    )
}

async fn fetch_render_message_list(pool: &PgPool, channel_id: Uuid, user_id: Uuid) -> Markup {
    let messages = query_as!(
        Message,
        r#"SELECT m.id, m.content, m.updated, m.author, u.name as author_name 
    FROM messages AS m
    JOIN chat_users AS u ON u.id = m.author
    WHERE m.channel = $1"#,
        channel_id,
    )
    .fetch_all(pool)
    .await
    .unwrap();

    html!(
        ol #messages class="flex flex-col-reverse overflow-y-auto" {
            @for msg in messages {
                (render_message(msg, user_id))
            }
        }
    )
}

fn render_message(msg: Message, user_id: Uuid) -> Markup {
    let is_author = msg.author == user_id;
    html!(
        li.chat
            .chat-end[is_author]
            .chat-start[!is_author] 
        {
            .chat-header {
                (msg.author_name) " "
                @let time = msg.id.get_datetime().unwrap();
                time.text-xs.opacity-50 datetime=(time.format(&Rfc3339).unwrap()) {
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
    )
}