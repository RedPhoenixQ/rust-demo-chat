use axum::{
    extract::{Path, Request, State}, http::StatusCode, middleware::{from_fn_with_state, Next}, response::IntoResponse, routing, Router
};
use maud::Markup;
use serde::Deserialize;
use sqlx::query;
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

use crate::{base_tempalte, utils::MyUuidExt, AppState};

#[derive(Deserialize)]
struct ChannelId {
    channel_id: Uuid,
}

#[derive(Deserialize)]
struct ServerId {
    server_id: Uuid,
}

pub fn router(state: AppState) -> Router<AppState> {
    Router::new().nest(
        "/channels/:server_id",
        Router::<AppState>::new()
            .route("/:channel_id", routing::get(get_messages))
            .route_layer(from_fn_with_state(state.clone(), is_user_member_of_server)),
    )
}

const USER_ID: Result<Uuid, uuid::Error> = Uuid::try_parse("01912d47-1aa9-7c51-8537-3c751e5af344");
async fn is_user_member_of_server(
    State(state): State<AppState>,
    Path(ServerId { server_id }): Path<ServerId>,
    request: Request,
    next: Next,
) -> impl IntoResponse {
    match sqlx::query!(
        r#"SELECT EXISTS(SELECT * FROM users_member_of_servers WHERE "user" = $1 AND server = $2) as is_member"#,
        USER_ID.unwrap(),
        server_id,
    )
    .fetch_one(&state.db).await.unwrap().is_member {
        Some(true) => Ok(next.run(request).await),
        _ => Err(StatusCode::UNAUTHORIZED) 
    }
}

async fn get_messages(
    State(state): State<AppState>,
    Path(ChannelId { channel_id }): Path<ChannelId>,
    Path(ServerId { server_id }): Path<ServerId>,
) -> Markup {
    let servers = query!(
        r#"SELECT s.id, s.name
    FROM servers AS s
    WHERE EXISTS (
        SELECT * FROM users_member_of_servers WHERE "user" = $1
    )"#,
        USER_ID.unwrap(),
    )
    .fetch_all(&state.db)
    .await
    .unwrap();

    let channels = query!(
        r#"SELECT c.id, c.name
    FROM channels AS c
    WHERE c.server = $1"#,
        server_id,
    )
    .fetch_all(&state.db)
    .await
    .unwrap();

    let messages = query!(
        r#"SELECT m.id, m.content, m.updated, m.author, u.name as author_name 
    FROM messages AS m
    JOIN chat_users AS u ON u.id = m.author
    WHERE m.channel = $1"#,
        channel_id,
    )
    .fetch_all(&state.db)
    .await
    .unwrap();

    base_tempalte(maud::html!(
        div class="grid h-screen max-h-screen max-h-dvh px-4 py-2" style="grid-template-columns: auto auto 1fr;" {
            ul.menu.bg-base-200.rounded-box {
                @for server in servers {
                    li { a href={"/channels/"(server.id)} { (server.name) } }
                }
            }
            ul.menu.bg-base-200.rounded-box {
                li { 
                    details open {
                        summary { "Group" }
                        ul {
                            @for channel in channels {
                                li { a href={"/channels/"(server_id)"/"(channel.id)} { (channel.name) } }
                            }
                        }   
                    }
                }
            }
            .grid style="grid-template-rows: 1fr auto" {
                ol.flex.flex-col-reverse {
                    @for msg in messages.into_iter().rev() {
                        li.chat
                            .chat-start[msg.author == USER_ID.unwrap()]
                            .chat-end[msg.author != USER_ID.unwrap()] 
                        {
                            .chat-header {
                                (msg.author_name) " "
                                @let time = msg.id.get_datetime().unwrap();
                                time.text-xs.opacity-50 datetime=(time.format(&Rfc3339).unwrap()) {
                                    // TODO: Make this a human readable relative time (one minute ago, ...)
                                    (time.to_string())
                                }
                            }
                            .chat-bubble {
                                (msg.content)
                            }
                            .chat-footer {
                                (msg.updated.to_string())
                            }
                        }
                    }
                }
                form.flex.items-end.gap-2 {
                    input.input.input-bordered.grow name="content" placeholder="Type here...";
                    button.btn.btn-primary { "Send" }
                }
            }
        }
    ))
}
