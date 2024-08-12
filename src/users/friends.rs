use axum::{
    extract::{Path, State},
    response::IntoResponse,
    routing, Form, Router,
};
use axum_htmx::HxResponseTrigger;
use maud::{html, Markup};
use serde::Deserialize;
use sqlx::{query, PgPool};
use uuid::Uuid;

use crate::{
    auth::Auth,
    base_modal,
    error::{Error, Result},
    AppState,
};

use super::{render_user_nav, UserTab};

#[derive(Deserialize)]
struct FriendId {
    friend_id: Uuid,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", routing::get(open_user_friends).post(add_friends))
        .route("/:friend_id", routing::delete(remove_friend))
        .route("/table", routing::get(get_friends_table))
}

async fn open_user_friends(
    State(state): State<AppState>,
    Auth { id: user_id }: Auth,
) -> Result<impl IntoResponse> {
    Ok((
        HxResponseTrigger::normal(["open-main-modal"]),
        fetch_render_user_friends(&state.db, user_id).await?,
    ))
}
async fn fetch_render_user_friends(pool: &PgPool, user_id: Uuid) -> Result<Markup> {
    let friends_table = fetch_render_friends_table(pool, user_id).await?;

    Ok(base_modal(html! {
        (render_user_nav(UserTab::Friends))
        (render_add_friend_form())
        (friends_table)
    }))
}

#[derive(Deserialize)]
struct AddFriend {
    id: Uuid,
}
async fn add_friends(
    State(state): State<AppState>,
    Auth { id: user_id }: Auth,
    add_friend: Option<Form<AddFriend>>,
) -> Result<impl IntoResponse> {
    if let Some(Form(add_friend)) = add_friend {
        let mut transaction = state.db.begin().await?;
        let rows_affected = query!(
            r#"INSERT INTO users_friends ("user", friend) VALUES ($1, $2)"#,
            user_id,
            add_friend.id,
        )
        .execute(&mut *transaction)
        .await?;
        if rows_affected.rows_affected() != 1 {
            return Err(Error::DatabaseActionFailed);
        }
        let rows_affected = query!(
            r#"INSERT INTO users_friends ("user", friend) VALUES ($1, $2)"#,
            add_friend.id,
            user_id,
        )
        .execute(&mut *transaction)
        .await?;
        if rows_affected.rows_affected() != 1 {
            return Err(Error::DatabaseActionFailed);
        }
        transaction.commit().await?;
    }
    Ok((
        HxResponseTrigger::normal(["update-friends-table"]),
        render_add_friend_form(),
    ))
}

async fn remove_friend(
    State(state): State<AppState>,
    Auth { id: user_id }: Auth,
    Path(FriendId { friend_id }): Path<FriendId>,
) -> Result<impl IntoResponse> {
    let mut transaction = state.db.begin().await?;
    let rows_affected = query!(
        r#"DELETE FROM users_friends WHERE "user" = $1 AND friend = $2"#,
        user_id,
        friend_id,
    )
    .execute(&mut *transaction)
    .await?;
    if rows_affected.rows_affected() != 1 {
        return Err(Error::DatabaseActionFailed);
    }
    let rows_affected = query!(
        r#"DELETE FROM users_friends WHERE "user" = $1 AND friend = $2"#,
        friend_id,
        user_id,
    )
    .execute(&mut *transaction)
    .await?;
    if rows_affected.rows_affected() != 1 {
        return Err(Error::DatabaseActionFailed);
    }
    transaction.commit().await?;

    Ok(html!())
}

fn render_add_friend_form() -> Markup {
    html!(
        form
            class="flex items-end"
            hx-post={"/users/friends"}
            hx-swap="outerHTML"
            hx-target="this"
        {
            .form-control.grow {
                .label {
                    .label-text {
                        "Add friend by id"
                    }
                }
                input type="text" name="id" class="input input-bordered w-full";
            }
            button type="submit" class="btn btn-primary" { "Add friend" }
        }
    )
}

async fn get_friends_table(
    State(state): State<AppState>,
    Auth { id: user_id }: Auth,
) -> impl IntoResponse {
    fetch_render_friends_table(&state.db, user_id).await
}
async fn fetch_render_friends_table(pool: &PgPool, user_id: Uuid) -> Result<Markup> {
    let friends = query!(
        r#"SELECT u.id, u.name 
    FROM users_friends as f
    RIGHT JOIN chat_users AS u
        ON u.id = f.friend
    WHERE f."user" = $1 
    "#,
        user_id
    )
    .fetch_all(pool)
    .await?;

    Ok(html!(
        table class="table"
            hx-get={"/users/friends/table"}
            hx-trigger="update-friends-table from:body"
            hx-swap="outerHTML"
            hx-target="this"
        {
            thead {
                tr {
                    th {}
                    th { "name" }
                    th {}
                }
            }
            tbody {
                @for friend in friends {
                    tr {
                        td {
                            button class="btn btn-ghost btn-sm"
                                onclick={"navigator.clipboard.writeText('"(friend.id)"')"}
                                title="Copy user id"
                                { "ID" }
                        }
                        td { (friend.name) }
                        td {
                            button class="link link-error"
                                hx-delete={"/users/friends/"(friend.id)}
                                hx-target="closest tr"
                                { "Remove" }
                        }
                    }
                }
            }
        }
    ))
}
