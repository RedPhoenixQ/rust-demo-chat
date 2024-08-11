use super::*;

mod general;

pub fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .route(
            "/",
            routing::get(general::open_general_page).put(general::update_server),
        )
        .route(
            "/members",
            routing::get(|Path(ServerId { server_id }): Path<ServerId>| async move {
                base_modal(render_settings_nav(server_id, SettingsTab::Members))
            }),
        )
        .layer(from_fn_with_state(state.clone(), is_allowed_to_edit_server))
}

async fn is_allowed_to_edit_server(
    State(state): State<AppState>,
    Auth { id: user_id }: Auth,
    Path(ServerId { server_id }): Path<ServerId>,
    request: Request,
    next: Next,
) -> Result<impl IntoResponse> {
    // FIXME: Check for edit rights
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

#[derive(PartialEq)]
enum SettingsTab {
    General,
    Members,
}
fn render_settings_nav(server_id: Uuid, active: SettingsTab) -> Markup {
    use SettingsTab::*;
    html!(
        div class="tabs-boxed tabs" {
            button.tab.tab-active[active == General] hx-get={"/servers/"(server_id)"/settings"} { "General" }
            button.tab.tab-active[active == Members] hx-get={"/servers/"(server_id)"/settings/members"} { "Members" }
        }
    )
}
