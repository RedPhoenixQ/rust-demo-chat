use std::{collections::BTreeMap, convert::Infallible};

use axum::response::sse::Event;
use sqlx::{postgres::PgListener, PgPool};
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, debug_span, error, Instrument};
use uuid::Uuid;

type UserEvent = std::result::Result<Event, Infallible>;
type UserRegMsg = (Uuid, oneshot::Sender<mpsc::Receiver<UserEvent>>);
type ChannelEventMsg = (Uuid, Kind);

#[derive(Debug, Clone)]
pub struct MessageRegistry {
    pub register: mpsc::Sender<(Uuid, UserRegMsg)>,
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

    let (register_tx, mut register_rx) = mpsc::channel(4);

    let pool = pool.clone();
    tokio::spawn(async move {
        let mut channel_tasks =
            BTreeMap::<Uuid, (mpsc::Sender<UserRegMsg>, mpsc::Sender<ChannelEventMsg>)>::new();
        loop {
            const UUID_LEN: usize = 36;
            tokio::select! {
                notif = listener.recv() => {
                    match notif {
                        Ok(notif) => {
                            let channel = notif.channel();
                            let payload = notif.payload();
                            let _span = debug_span!("Message notification", %channel, %payload);
                            // Payload is exactly 2 Uuid's long
                            if payload.len() != UUID_LEN*2 {
                                error!("Payload was not exactly 2 uuids");
                                continue;
                            }

                            let kind = match notif.channel() {
                                "insert_message" => Kind::Insert,
                                "update_message" => Kind::Update,
                                "delete_message" => Kind::Delete,
                                _ => {
                                    continue;
                                }
                            };

                            let (Ok(message_id), Ok(channel_id)) = (Uuid::try_parse(&payload[..UUID_LEN]), Uuid::try_parse(&payload[UUID_LEN..])) else {
                                error!(message_id = %&payload[..UUID_LEN], channel_id = %&payload[UUID_LEN..], "An id failed to parse");
                                continue;
                            };
                            let _span = debug_span!("Message notification", %channel, %payload);
                            let Some((_, event_tx)) = channel_tasks.get(&channel_id) else {
                                debug!("No task exists for the channel");
                                continue;
                            };
                            if let Err(err) = event_tx.send((message_id, kind)).await {
                                error!(?err, "An error occured when sending message_id to channel task");
                            };
                        }
                        Err(err) => error!(?err, "Error occured in db listener"),
                    }
                }
                Some((channel_id, user_reg_msg)) = register_rx.recv() => {
                    if let Some((user_tx, _)) = channel_tasks.get(&channel_id) {
                        user_tx.send(user_reg_msg).await.expect("Registration to work");
                    } else {
                        let (user_tx, user_rx) = mpsc::channel(1);
                        let (event_tx, event_rx) = mpsc::channel(1);
                        spawn_channel_task(user_rx, event_rx, pool.clone());
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

fn spawn_channel_task(
    mut register_rx: mpsc::Receiver<UserRegMsg>,
    mut event_rx: mpsc::Receiver<ChannelEventMsg>,
    pool: PgPool,
) {
    tokio::spawn(async move {
        let mut user_senders = BTreeMap::<Uuid, mpsc::Sender<UserEvent>>::new();
        loop {
            tokio::select! {
                Some((message_id, kind)) = event_rx.recv() => {
                    let span = debug_span!("Channel Event Task", %message_id, ?kind);
                    if let Err(err) = handle_message_event(message_id, kind, user_senders.iter(), &pool).instrument(span).await {
                        error!(?err, "An error occured while sending events to users")
                    };
               }
                Some((user_id, sender)) = register_rx.recv() => {
                    let (tx, rx) = mpsc::channel(1);
                    user_senders.insert(user_id, tx);
                    sender.send(rx).expect("Sending sse channel to work");
                }
            };
        }
    });
}

async fn handle_message_event(
    message_id: Uuid,
    kind: Kind,
    users: impl Iterator<Item = (&Uuid, &mpsc::Sender<UserEvent>)>,
    pool: &PgPool,
) -> Result<(), sqlx::Error> {
    Ok(match kind {
        Kind::Insert => {
            let msg = sqlx::query_as!(
                super::Message,
                r#"SELECT m.id, m.content, m.updated, m.author, u.name as author_name 
            FROM messages AS m
            JOIN chat_users AS u ON u.id = m.author
            WHERE m.id = $1
            LIMIT 1"#,
                message_id,
            )
            .fetch_one(pool)
            .await?;

            for (id, tx) in users {
                if let Ok(rendered_msg) = super::render_message(&msg, id) {
                    tx.send(Ok(Event::default()
                        .event("insert_message")
                        .data(rendered_msg.0)))
                        .await
                        .unwrap();
                }
            }
        }
        Kind::Update => {
            error!("Unhandled event recived")
        }
        Kind::Delete => {
            error!("Unhandled event recived")
        }
    })
}
