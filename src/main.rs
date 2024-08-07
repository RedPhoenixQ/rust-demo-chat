use axum::Router;
use maud::{html, PreEscaped};
use sqlx::PgPool;
use tracing::{info, info_span};

mod chat;
mod utils;

const HTMX_SCRIPT: PreEscaped<&str> = PreEscaped(
    #[cfg(debug_assertions)]
    r#"<script src="https://unpkg.com/htmx.org@2.0.1/dist/htmx.js" integrity="sha384-gpIh5aLQ0qmX8kZdyhsd6jA24uKLkqIr1WAGtantR4KsS97l/NRBvh8/8OYGThAf" crossorigin="anonymous"></script>"#,
    #[cfg(not(debug_assertions))]
    r#"<script src="https://unpkg.com/htmx.org@2.0.1" integrity="sha384-QWGpdj554B4ETpJJC9z+ZHJcA/i59TyjxEPXiiUgN2WmTyV5OEZWCD6gQhgkdpB/" crossorigin="anonymous"></script>"#,
);

fn base_tempalte(content: maud::Markup) -> maud::Markup {
    html!(
        (maud::DOCTYPE)
        html data-theme="dark" {
            head {
                (HTMX_SCRIPT)
                link rel="stylesheet" href="/styles.css";
            }
            body.min-h-sreen.min-h-dvh hx-boost {
                (content)
            }
        }
    )
}

#[derive(Debug, Clone)]
struct AppState {
    db: PgPool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    setup_tracing()?;

    let state = AppState {
        db: PgPool::connect_lazy(&std::env::var("DATABASE_URL")?)?,
    };

    let router = Router::new()
        .route(
            "/hello",
            axum::routing::get(|| async {
                base_tempalte(maud::html!(span class="btn" {"Hello, World!"}))
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

    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000").await?;
    info!("Listening on http://{}", listener.local_addr()?);

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
