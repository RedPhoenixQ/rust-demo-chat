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

use super::{render_settings_nav, ServerId, SettingsTab};

#[derive(Deserialize)]
struct MemberId {
    member_id: Uuid,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", routing::get(open_member_page).post(add_member))
        .route("/:member_id", routing::delete(remove_member))
        .route("/table", routing::get(get_member_table))
}

async fn open_member_page(
    State(state): State<AppState>,
    Auth { id: user_id }: Auth,
    Path(ServerId { server_id }): Path<ServerId>,
) -> Result<impl IntoResponse> {
    Ok((
        HxResponseTrigger::normal(["open-main-modal"]),
        fetch_render_members_page(&state.db, server_id, user_id).await?,
    ))
}
async fn fetch_render_members_page(
    pool: &PgPool,
    server_id: Uuid,
    user_id: Uuid,
) -> Result<Markup> {
    let member_table = fetch_render_member_table(pool, server_id, user_id).await?;

    Ok(base_modal(html! {
        (render_settings_nav(server_id, SettingsTab::Members))
        (render_add_member_form(server_id))
        (member_table)
    }))
}

#[derive(Deserialize)]
struct AddMember {
    id: Uuid,
}
async fn add_member(
    State(state): State<AppState>,
    Path(ServerId { server_id }): Path<ServerId>,
    add_member: Option<Form<AddMember>>,
) -> Result<impl IntoResponse> {
    if let Some(Form(add_member)) = add_member {
        let rows_affected = query!(
            r#"INSERT INTO users_member_of_servers ("user", server) VALUES ($1, $2)"#,
            add_member.id,
            server_id,
        )
        .execute(&state.db)
        .await?;
        if rows_affected.rows_affected() != 1 {
            return Err(Error::DatabaseActionFailed);
        }
    }
    Ok((
        HxResponseTrigger::normal(["update-member-table"]),
        render_add_member_form(server_id),
    ))
}

async fn remove_member(
    State(state): State<AppState>,
    Path(ServerId { server_id }): Path<ServerId>,
    Path(MemberId { member_id }): Path<MemberId>,
) -> Result<impl IntoResponse> {
    let mut transaction = state.db.begin().await?;
    let rows_affected = query!(
        r#"DELETE FROM users_member_of_servers WHERE "user" = $1 AND server = $2"#,
        member_id,
        server_id,
    )
    .execute(&mut *transaction)
    .await?;
    if rows_affected.rows_affected() != 1 {
        return Err(Error::DatabaseActionFailed);
    }
    transaction.commit().await?;

    Ok(html!())
}

fn render_add_member_form(server_id: Uuid) -> Markup {
    html!(
        form
            class="flex items-end"
            hx-post={"/servers/"(server_id)"/settings/members"}
            hx-swap="outerHTML"
            hx-target="this"
        {
            .form-control.grow {
                .label {
                    .label-text {
                        "Add user by id"
                    }
                }
                input type="text" name="id" class="input input-bordered w-full";
            }
            button type="submit" class="btn btn-primary" { "Add member" }
        }
    )
}

async fn get_member_table(
    State(state): State<AppState>,
    Auth { id: user_id }: Auth,
    Path(ServerId { server_id }): Path<ServerId>,
) -> impl IntoResponse {
    fetch_render_member_table(&state.db, server_id, user_id).await
}
async fn fetch_render_member_table(
    pool: &PgPool,
    server_id: Uuid,
    user_id: Uuid,
) -> Result<Markup> {
    let members = query!(
        r#"SELECT u.id, u.name 
    FROM chat_users as u
    JOIN users_member_of_servers AS m 
        ON u.id = m."user"
    WHERE m.server = $1 
    "#,
        server_id
    )
    .fetch_all(pool)
    .await?;

    Ok(html!(
        table class="table"
            hx-get={"/servers/"(server_id)"/settings/members/table"}
            hx-trigger="update-member-table from:body"
            hx-swap="outerHTML"
            hx-target="this"
        {
            thead {
                tr {
                    th { "name" }
                    th {}
                }
            }
            tbody {
                @for member in members {
                    tr {
                        td { (member.name) }
                        td {
                            @if member.id != user_id {
                                button class="link link-error"
                                    hx-delete={"/servers/"(server_id)"/settings/members/"(member.id)}
                                    hx-target="closest tr"
                                    { "Remove" }
                            } @else {
                                .italic.opacity-50 { "You" }
                            }
                        }
                    }
                }
            }
        }
    ))
}
