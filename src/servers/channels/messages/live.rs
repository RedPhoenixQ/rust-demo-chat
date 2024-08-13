use std::{collections::BTreeMap, convert::Infallible};

use axum::response::sse::Event;
use maud::html;
use sqlx::{postgres::PgListener, PgPool};
use tokio::sync::{mpsc, oneshot};
use tracing::{debug_span, error, trace, Instrument};
use uuid::Uuid;

use super::{render_message, Message};

type UserEvent = std::result::Result<Event, Infallible>;
type UserRegMsg = (Uuid, oneshot::Sender<mpsc::UnboundedReceiver<UserEvent>>);
type ChannelEventMsg = (Uuid, Kind);

#[derive(Debug, Clone)]
pub struct ChannelIds {
    pub channel_id: Uuid,
    pub server_id: Uuid,
}

#[derive(Debug, Clone)]
pub struct MessageRegistry {
    pub register: mpsc::Sender<(ChannelIds, UserRegMsg)>,
}

#[derive(Debug)]
enum Kind {
    Insert,
    Update,
    Delete,
}

pub async fn create_listener(pool: &PgPool) -> sqlx::Result<MessageRegistry> {
    let mut listener = PgListener::connect_with(pool).await?;
    listener
        .listen_all(["insert_message", "update_message", "delete_message"])
        .await?;

    let (register_tx, mut register_rx) = mpsc::channel::<(ChannelIds, UserRegMsg)>(4);

    let pool = pool.clone();
    tokio::spawn(async move {
        let mut channel_tasks =
            BTreeMap::<Uuid, (mpsc::Sender<UserRegMsg>, mpsc::Sender<ChannelEventMsg>)>::new();
        loop {
            tokio::select! {
                notif = listener.recv() => {
                    match notif {
                        Ok(notif) => {
                            let payload = notif.payload();
                            let channel = notif.channel();
                            let span = debug_span!("Message notification", %channel, %payload);
                            handle_notification(channel, payload, &channel_tasks).instrument(span).await;
                        }
                        Err(err) => error!(?err, "Error occured in db listener"),
                    }
                }
                Some((ids, user_reg_msg)) = register_rx.recv() => {
                    if let Some((user_tx, _)) = channel_tasks.get(&ids.channel_id) {
                        user_tx.send(user_reg_msg).await.expect("Registration to work");
                    } else {
                        let channel_id = ids.channel_id.clone();
                        let (user_tx, user_rx) = mpsc::channel(1);
                        let (event_tx, event_rx) = mpsc::channel(1);
                        spawn_channel_task(ids, user_rx, event_rx, pool.clone());
                        user_tx.send(user_reg_msg).await.expect("Registration to work");
                        channel_tasks.insert(channel_id, (user_tx, event_tx));
                    }
                }
            };
        }
    });

    Ok(MessageRegistry {
        register: register_tx,
    })
}

async fn handle_notification(
    channel: &str,
    payload: &str,
    channel_tasks: &BTreeMap<Uuid, (mpsc::Sender<UserRegMsg>, mpsc::Sender<ChannelEventMsg>)>,
) {
    const UUID_LEN: usize = 36;

    // Payload is exactly 2 Uuid's long
    if payload.len() != UUID_LEN * 2 {
        error!("Payload was not exactly 2 uuids");
        return;
    }

    let kind = match channel {
        "insert_message" => Kind::Insert,
        "update_message" => Kind::Update,
        "delete_message" => Kind::Delete,
        channel => {
            error!(%channel, "Unexpected channel recived");
            return;
        }
    };

    let (Ok(message_id), Ok(channel_id)) = (
        Uuid::try_parse(&payload[..UUID_LEN]),
        Uuid::try_parse(&payload[UUID_LEN..]),
    ) else {
        error!(message_id = %&payload[..UUID_LEN], channel_id = %&payload[UUID_LEN..], "An id failed to parse");
        return;
    };
    let Some((_, event_tx)) = channel_tasks.get(&channel_id) else {
        trace!(%channel_id, "No task exists for the channel");
        return;
    };
    trace!( %message_id, %channel_id,"Sending event to channel handler");
    if let Err(err) = event_tx.send((message_id, kind)).await {
        error!(
            ?err,
            "An error occured when sending message_id to channel task"
        );
    };
}

fn spawn_channel_task(
    ids: ChannelIds,
    mut register_rx: mpsc::Receiver<UserRegMsg>,
    mut event_rx: mpsc::Receiver<ChannelEventMsg>,
    pool: PgPool,
) {
    tokio::spawn(async move {
        let mut user_senders = BTreeMap::<Uuid, mpsc::UnboundedSender<UserEvent>>::new();
        loop {
            tokio::select! {
                Some((message_id, kind)) = event_rx.recv() => {
                    let span = debug_span!("Channel Event Task", %message_id, ?kind);
                    if let Err(err) = handle_message_event(&ids, message_id, kind, &mut user_senders, &pool).instrument(span).await {
                        error!(?err, "An error occured while sending events to users")
                    };
               }
                Some((user_id, sender)) = register_rx.recv() => {
                    let (tx, rx) = mpsc::unbounded_channel();
                    user_senders.insert(user_id, tx);
                    sender.send(rx).expect("Sending sse channel to work");
                }
            };
        }
    });
}

async fn handle_message_event(
    ChannelIds {
        channel_id,
        server_id,
    }: &ChannelIds,
    message_id: Uuid,
    kind: Kind,
    users: &mut BTreeMap<Uuid, mpsc::UnboundedSender<UserEvent>>,
    pool: &PgPool,
) -> Result<(), sqlx::Error> {
    let mut stale_sender = Vec::new();
    match kind {
        Kind::Insert | Kind::Update => {
            let msg = sqlx::query_as!(
                Message,
                r#"SELECT m.id, m.content, m.updated, m.author, u.name as author_name 
            FROM messages AS m
            JOIN chat_users AS u ON u.id = m.author
            WHERE m.id = $1
            LIMIT 1"#,
                message_id,
            )
            .fetch_one(pool)
            .await?;

            for (user_id, tx) in users.iter() {
                if let Ok(rendered_msg) = render_message(
                    &msg,
                    user_id,
                    channel_id,
                    server_id,
                    matches!(kind, Kind::Update),
                ) {
                    if tx
                        .send(Ok(Event::default().event("message").data(rendered_msg.0)))
                        .is_err()
                    {
                        stale_sender.push(user_id.to_owned());
                    };
                }
            }
        }
        Kind::Delete => {
            for (id, tx) in users.iter() {
                if tx
                    .send(Ok(Event::default()
                        .event("message")
                        .data(html!(#{"msg-"(message_id)} hx-swap-oob="delete" {}).0)))
                    .is_err()
                {
                    stale_sender.push(id.to_owned());
                };
            }
        }
    }
    for id in &stale_sender {
        trace!(user_id = %id, "Removing stale sender");
        users.remove(id);
    }
    Ok(())
}
