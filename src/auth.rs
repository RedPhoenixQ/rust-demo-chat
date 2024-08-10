use axum::{async_trait, extract::FromRequestParts, http::request::Parts, response::Redirect};
use axum_extra::extract::CookieJar;
use uuid::Uuid;

#[derive(Debug)]
pub struct Auth {
    pub id: Uuid,
}

#[async_trait]
impl<S> FromRequestParts<S> for Auth {
    type Rejection = Redirect;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let cookies = CookieJar::from_request_parts(parts, &())
            .await
            .or(Err(Redirect::temporary("/login")))?;
        let auth_id = cookies
            .get("auth_id")
            .ok_or(Redirect::temporary("/login"))?;
        let id = Uuid::try_parse(auth_id.value_trimmed()).or(Err(Redirect::temporary("/login")))?;
        Ok(Auth { id })
    }
}
