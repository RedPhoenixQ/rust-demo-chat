use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use maud::html;
use tracing::debug;
use uuid::Uuid;

use crate::base_tempalte;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
    NoTimestampFromUuid { id: Uuid },

    // Database
    DatabaseActionFailed,
    DB(sqlx::Error),

    // time crate
    Time(time::Error),
}

impl std::error::Error for Error {}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let id = Uuid::now_v7().to_string();
        debug!(error = ?self, id = &id, "An error occured");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            base_tempalte(html!(
              main class="grid min-h-screen place-items-center" {
                div {
                  h1 class="text-center text-2xl" { "An error occured" }
                  p class="text-center" { "Bellow is an error id" }
                  p class="text-center" { (id) }
                }
              }
            )),
        )
            .into_response()
    }
}

impl From<sqlx::Error> for Error {
    fn from(value: sqlx::Error) -> Self {
        Error::DB(value)
    }
}

impl From<time::error::Format> for Error {
    fn from(err: time::error::Format) -> Self {
        Error::Time(err.into())
    }
}
