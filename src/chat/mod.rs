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

async fn is_user_member_of_server(
    State(state): State<AppState>,
    Path(ServerId { server_id }): Path<ServerId>,
    request: Request,
    next: Next,
) -> impl IntoResponse {
    const USER_ID: Result<Uuid, uuid::Error> = Uuid::try_parse("01912d47-1aa9-7c51-8537-3c751e5af344");
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
) -> Markup {
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
        div class="btn" {"Hello, World!"}
        div {
            @for msg in messages {
                .chat.chat-start {
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
    ))
}
