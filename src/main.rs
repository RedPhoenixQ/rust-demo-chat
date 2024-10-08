use axum::{response::Redirect, routing, Router};
use maud::{html, PreEscaped};
use sqlx::postgres::{PgListener, PgPool};
use tracing::{info, info_span};

mod auth;
mod chat;
mod error;
mod servers;
mod users;
mod utils;

use servers::channels::messages;

const HTMX_SCRIPT: PreEscaped<&str> = PreEscaped(
    #[cfg(debug_assertions)]
    r#"<script src="https://unpkg.com/htmx.org@2.0.1/dist/htmx.js" integrity="sha384-gpIh5aLQ0qmX8kZdyhsd6jA24uKLkqIr1WAGtantR4KsS97l/NRBvh8/8OYGThAf" crossorigin="anonymous"></script>"#,
    #[cfg(not(debug_assertions))]
    r#"<script src="https://unpkg.com/htmx.org@2.0.1" integrity="sha384-QWGpdj554B4ETpJJC9z+ZHJcA/i59TyjxEPXiiUgN2WmTyV5OEZWCD6gQhgkdpB/" crossorigin="anonymous"></script>"#,
);
const HTMX_SSE_SCRIPT: PreEscaped<&str> =
    PreEscaped(r#"<script src="https://unpkg.com/htmx-ext-sse@2.2.1/sse.js"></script>"#);
const RELATIVE_TIME_WEB_COMPONENT: PreEscaped<&str> = PreEscaped(
    r#"<script type="module" src="https://unpkg.com/@github/relative-time-element@4.4.2/dist/bundle.js"></script>"#,
);

fn base_tempalte(content: maud::Markup) -> maud::Markup {
    html!(
        (maud::DOCTYPE)
        html data-theme="dark" {
            head {
                (HTMX_SCRIPT)
                (HTMX_SSE_SCRIPT)
                (RELATIVE_TIME_WEB_COMPONENT)
                link rel="stylesheet" href="/styles.css";
            }
            body class="min-h-screen" hx-boost="true" hx-on-open-main-modal="mainModal.showModal()" {
                (content)
                dialog #mainModal class="modal"
                    hx-on-close-modal="this.close()"
                    hx-target="#modalInner"
                    hx-swap="outerHTML"
                {
                    (base_modal(Default::default()))
                    form.modal-backdrop method="dialog" hx-disable {
                        button type="submit" { "Close" }
                    }
                }
            }
        }
    )
}

fn header() -> maud::Markup {
    html!(
        header class="navbar bg-base-100" {
            div class="flex-1" {
                a class="btn btn-ghost" href="/" {
                    "Home"
                }
            }
            div class="flex-none" {
                ul class="menu menu-horizontal" {
                    li {
                        details class="z-20" {
                            summary { "Auth" }
                            ul {
                                li { a href="/auth/test" { "Test" } }
                                li { a href="/auth/yeeter" { "Yeeter" } }
                                li { a href="/logout" { "Logout" } }
                            }
                        }
                    }
                }
            }
            div class="flex-none" {
                button class="btn"  hx-get="/users/profile" hx-target="#modalInner" hx-swap="outerHTML" { "Profile" }
            }
        }
    )
}

fn base_modal(content: maud::Markup) -> maud::Markup {
    maud::html!(
        #modalInner .modal-box {
            form method="dialog" hx-disable {
                button class="btn btn-circle btn-ghost btn-sm absolute right-2 top-2"
                    type="submit"
                    aria-label="close"
                    { "✕" }
            }
            (content)
        }
    )
}

#[derive(Debug, Clone)]
struct AppState {
    db: PgPool,
    message_live: messages::live::MessageRegistry,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    setup_tracing()?;

    let db = PgPool::connect_lazy(&std::env::var("DATABASE_URL")?)?;
    let message_live = messages::live::create_listener(&db).await?;
    let state = AppState { db, message_live };

    let mut listener = PgListener::connect_with(&state.db).await?;
    tokio::spawn(async move {
        listener
            .listen_all(["insert_message", "update_message", "delete_message", "test"])
            .await
            .unwrap();
        while let Ok(notification) = listener.recv().await {
            info!(
                channel = notification.channel(),
                payload = notification.payload(),
                "Recived notification"
            );
        }
    });

    let router = Router::new()
        .route("/api/health", routing::any(|| async { "alive" }))
        // FIXME: Create propper auth login handlers
        .route(
            "/login",
            axum::routing::get(|| async {
                base_tempalte(maud::html!(
                    (header())
                    h1 { "Login" }
                ))
            }),
        )
        .route(
            "/logout",
            axum::routing::get(|cookies: axum_extra::extract::CookieJar| async {
                (
                    cookies.add(
                        axum_extra::extract::cookie::Cookie::build("auth_id")
                            .removal()
                            .path("/")
                            .http_only(true)
                            .secure(true),
                    ),
                    Redirect::temporary("/"),
                )
            }),
        )
        .route(
            "/auth/yeeter",
            axum::routing::get(|cookies: axum_extra::extract::CookieJar| async {
                (
                    cookies.add(
                        axum_extra::extract::cookie::Cookie::build((
                            "auth_id",
                            "01912d47-1aa9-7c51-8537-3c751e5af344",
                        ))
                        .path("/")
                        .http_only(true)
                        .secure(true),
                    ),
                    Redirect::temporary("/"),
                )
            }),
        )
        // FIXME: Create propper auth login handlers
        .route(
            "/auth/test",
            axum::routing::get(|cookies: axum_extra::extract::CookieJar| async {
                (
                    cookies.add(
                        axum_extra::extract::cookie::Cookie::build((
                            "auth_id",
                            "019132bf-fac6-7ccf-a673-302ec86fefd7",
                        ))
                        .path("/")
                        .http_only(true)
                        .secure(true),
                    ),
                    Redirect::temporary("/"),
                )
            }),
        )
        .nest("/servers", servers::router(state.clone()))
        .nest("/users", users::router())
        .route("/", routing::get(chat::get_chat_page))
        .fallback_service(tower_http::services::ServeDir::new("assets"))
        .layer(
            tower_http::trace::TraceLayer::new_for_http().make_span_with(
                |request: &axum::http::Request<_>| {
                    use axum::extract::MatchedPath;
                    // Log the matched route's path (with placeholders not filled in).
                    // Use request.uri() or OriginalUri if you want the real path.
                    let matched_path = request
                        .extensions()
                        .get::<MatchedPath>()
                        .map(MatchedPath::as_str);

                    info_span!(
                        "http_request",
                        method = ?request.method(),
                        matched_path,
                    )
                },
            ),
        )
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await?;
    info!(
        "Listening on http://localhost:{}",
        listener.local_addr()?.port()
    );

    axum::serve(listener, router).await?;
    info!("Server exited");

    Ok(())
}

fn setup_tracing() -> Result<(), tracing::subscriber::SetGlobalDefaultError> {
    let subscriber = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env());
    #[cfg(debug_assertions)]
    let subscriber = subscriber.pretty();

    tracing::subscriber::set_global_default(subscriber.finish())
}
