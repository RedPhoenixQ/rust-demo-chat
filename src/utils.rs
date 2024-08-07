use sqlx::types::{time::OffsetDateTime, Uuid};

pub fn uuid_to_date(uuid: Uuid) -> Option<OffsetDateTime> {
    let t = uuid.get_timestamp()?;
    OffsetDateTime::from_unix_timestamp(t.to_unix().0 as i64).ok()
}
