use axum::{response::Redirect, Router};
use maud::{html, PreEscaped};
use sqlx::postgres::{PgListener, PgPool};
use tracing::{info, info_span};

mod auth;
mod chat;
mod utils;

const HTMX_SCRIPT: PreEscaped<&str> = PreEscaped(
    #[cfg(debug_assertions)]
    r#"<script src="https://unpkg.com/htmx.org@2.0.1/dist/htmx.js" integrity="sha384-gpIh5aLQ0qmX8kZdyhsd6jA24uKLkqIr1WAGtantR4KsS97l/NRBvh8/8OYGThAf" crossorigin="anonymous"></script>"#,
    #[cfg(not(debug_assertions))]
    r#"<script src="https://unpkg.com/htmx.org@2.0.1" integrity="sha384-QWGpdj554B4ETpJJC9z+ZHJcA/i59TyjxEPXiiUgN2WmTyV5OEZWCD6gQhgkdpB/" crossorigin="anonymous"></script>"#,
);
const HTMX_SSE_SCRIPT: PreEscaped<&str> =
    PreEscaped(r#"<script src="https://unpkg.com/htmx-ext-sse@2.2.1/sse.js"></script>"#);

fn base_tempalte(content: maud::Markup) -> maud::Markup {
    html!(
        (maud::DOCTYPE)
        html data-theme="dark" {
            head {
                (HTMX_SCRIPT)
                (HTMX_SSE_SCRIPT)
                link rel="stylesheet" href="/styles.css";
            }
            body class="min-h-screen" hx-boost="true" {
                (content)
            }
        }
    )
}

fn header() -> maud::Markup {
    html!(
        header class="navbar bg-base-100" {
            div class="flex-1" {
                a class="btn btn-ghost" href="/servers" {
                    "Servers"
                }
            }
            div class="flex-none" {
                ul class="menu menu-horizontal px-1" {
                    li {
                        details class="z-20" {
                            summary { "Auth" }
                            ul {
                                li { a href="/auth/test" { "Test" } }
                                li { a href="/auth/yeeter" { "Yeeter" } }
                            }
                        }
                    }
                }
            }
        }
    )
}

#[derive(Debug, Clone)]
struct AppState {
    db: PgPool,
    message_live: chat::live_messages::MessageRegistry,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    setup_tracing()?;

    let db = PgPool::connect_lazy(&std::env::var("DATABASE_URL")?)?;
    let message_live = chat::live_messages::create_listener(&db).await?;
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
        .route(
            "/hello",
            axum::routing::get(|| async {
                base_tempalte(maud::html!(span class="btn" {"Hello, World!"}))
            }),
        )
        // FIXME: Create propper auth login handlers
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
                    Redirect::temporary("/hello"),
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
                    Redirect::temporary("/hello"),
                )
            }),
        )
        .nest("/", chat::router(state.clone()))
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
