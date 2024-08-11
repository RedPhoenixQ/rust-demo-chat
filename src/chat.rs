use axum::{
    extract::{Path, State},
    response::IntoResponse,
};
use maud::html;
use tokio::try_join;

use crate::{
    auth::Auth,
    base_tempalte,
    error::Result,
    header,
    servers::{
        channels::{
            fetch_render_channel_list, messages::fetch_render_message_list, MaybeChannelId,
        },
        fetch_render_server_list, MaybeServerId,
    },
    AppState,
};

pub async fn get_chat_page(
    State(state): State<AppState>,
    Auth { id: user_id }: Auth,
    Path(MaybeChannelId { channel_id }): Path<MaybeChannelId>,
    Path(MaybeServerId { server_id }): Path<MaybeServerId>,
) -> Result<impl IntoResponse> {
    let (server_list, channel_list, messages_list) = try_join!(
        fetch_render_server_list(&state.db, user_id, server_id),
        async {
            Ok(if let Some(server_id) = server_id {
                Some(fetch_render_channel_list(&state.db, server_id, channel_id).await?)
            } else {
                None
            })
        },
        async {
            Ok(
                if let (Some(server_id), Some(channel_id)) = (server_id, channel_id) {
                    Some((
                        fetch_render_message_list(&state.db, server_id, channel_id, user_id)
                            .await?,
                        (server_id, channel_id),
                    ))
                } else {
                    None
                },
            )
        }
    )?;

    Ok(base_tempalte(html!(
        main class="grid max-h-screen min-h-screen px-4 py-2" style="grid-template-columns: auto auto 1fr; grid-template-rows: auto minmax(0,1fr)" {
            .col-span-full { (header()) }
            (server_list)
            (channel_list.unwrap_or(html!(ul #channels-list {})))
            #chat-wrapper.grid style="grid-template-rows: 1fr auto" {
                @if let Some((messages_list, (server_id, channel_id))) = messages_list {
                    (messages_list)
                    form #message-form.flex.items-end.gap-2
                        hx-post={"/servers/"(server_id)"/channels/"(channel_id)"/messages"}
                        hx-swap="none"
                        "hx-on::after-request"="if (event.detail.successful) this.reset()"
                    {
                        input.input.input-bordered.grow name="content" placeholder="Type here...";
                        button.btn.btn-primary { "Send" }
                    }
                }
            }
        }
    )))
}
