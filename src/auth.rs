use axum::{
    async_trait,
    extract::FromRequestParts,
    http::{request::Parts, StatusCode},
};
use axum_extra::extract::CookieJar;
use uuid::Uuid;

#[derive(Debug)]
pub struct Auth {
    pub id: Uuid,
}

#[async_trait]
impl<S> FromRequestParts<S> for Auth {
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let cookies = CookieJar::from_request_parts(parts, &())
            .await
            .or(Err((StatusCode::UNAUTHORIZED, "Missing cookies")))?;
        let auth_id = cookies
            .get("auth_id")
            .ok_or((StatusCode::UNAUTHORIZED, "Missing auth_id cookie"))?;
        let id = Uuid::try_parse(auth_id.value_trimmed())
            .or(Err((StatusCode::UNAUTHORIZED, "Malformed auth_id cookie")))?;
        Ok(Auth { id })
    }
}
