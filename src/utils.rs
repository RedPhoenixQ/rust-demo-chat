use time::{Duration, OffsetDateTime};
use uuid::Uuid;

pub trait MyUuidExt {
    fn get_datetime(&self) -> Option<OffsetDateTime>;
}

impl MyUuidExt for Uuid {
    fn get_datetime(&self) -> Option<OffsetDateTime> {
        let (timestamp, nanos) = self.get_timestamp()?.to_unix();
        OffsetDateTime::from_unix_timestamp(timestamp as i64)
            .map(|datetime| datetime + Duration::nanoseconds(nanos.into()))
            .ok()
    }
}
