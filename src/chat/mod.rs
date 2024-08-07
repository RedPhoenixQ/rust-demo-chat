use axum::{extract::State, routing, Router};
use sqlx::query;

use crate::{base_tempalte, utils::uuid_to_date, AppState};

pub fn router() -> Router<AppState> {
    Router::new().route(
        "/",
        routing::get(|State(state): State<AppState>| async move {
            let messages = query!(
                "SELECT m.id, m.content, m.updated, m.author, u.name as author_name 
                FROM messages AS m
                JOIN chat_users AS u ON u.id = m.author"
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
                                (msg.author_name)
                                span.text-xs.opacity-50 {
                                    (uuid_to_date(msg.id).unwrap().to_string())
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
        }),
    )
}
