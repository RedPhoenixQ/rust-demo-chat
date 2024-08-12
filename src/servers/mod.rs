use axum::{
    extract::{Path, Query, Request, State},
    http::StatusCode,
    middleware::{from_fn_with_state, Next},
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
    chat::get_chat_page,
    error::{Error, Result},
    AppState,
};

pub mod channels;
mod settings;

#[derive(Deserialize)]
pub struct ServerId {
    pub server_id: Uuid,
}
#[derive(Deserialize)]
pub struct MaybeServerId {
    pub server_id: Option<Uuid>,
}

pub fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .nest("/:server_id/channels", channels::router())
        .route(
            "/:server_id",
            routing::get(get_chat_page).delete(delete_server),
        )
        .layer(from_fn_with_state(state.clone(), is_user_member_of_server))
        .nest(
            "/:server_id/settings",
            // NOTE: Does not need member check because it check edit rights
            settings::router(state.clone()),
        )
        .route("/", routing::get(get_servers).post(create_server))
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

#[derive(Deserialize)]
struct NewServer {
    name: String,
}
async fn create_server(
    State(state): State<AppState>,
    Auth { id: user_id }: Auth,
    new_server: Option<Form<NewServer>>,
) -> Result<impl IntoResponse> {
    fn render_new_server_form_inners() -> Markup {
        base_modal(html!(
            form method="post" hx-post="/servers" {
                label class="form-control m-auto w-full max-w-xs" {
                    .label { .label-text { "Channel name" } }
                    input type="text" name="name" class="input input-bordered w-full max-w-xs";
                }
                .modal-action {
                    button type="submit" class="btn btn-primary" { "Create" }
                }
            }
        ))
    }

    let Some(Form(new_server)) = new_server else {
        return Ok((
            HxResponseTrigger::normal(["open-main-modal"]),
            render_new_server_form_inners(),
        ));
    };

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

async fn get_servers(
    State(state): State<AppState>,
    Auth { id: user_id }: Auth,
    Query(MaybeServerId { server_id }): Query<MaybeServerId>,
) -> Result<impl IntoResponse> {
    fetch_render_server_list(&state.db, user_id, server_id).await
}
pub async fn fetch_render_server_list(
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
            hx-get={"/servers?server_id="(active_server.unwrap_or_default())}
            hx-trigger="get-server-list from:body"
            hx-swap="outerHTML"
        {
            li.menu-title {
                button class="btn btn-ghost btn-sm" hx-post="/servers" hx-target="#modalInner" { "New" }
            }
            @for server in servers {
                li #{"server-"(server.id)} {
                    div.active[active_server.is_some_and(|id| id == server.id)].flex {
                        a.grow href={"/servers/"(server.id)} {
                            (server.name)
                        }
                        button class="btn btn-circle btn-ghost btn-sm" hx-get={"/servers/"(server.id)"/settings"} hx-target="#modalInner" { "..." }
                    }
                }
            }
        }
    ))
}
