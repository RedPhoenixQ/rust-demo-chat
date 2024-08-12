use chrono::{DateTime, Utc};
use uuid::Uuid;

pub trait MyUuidExt {
    fn get_datetime(&self) -> Option<DateTime<Utc>>;
}

impl MyUuidExt for Uuid {
    fn get_datetime(&self) -> Option<DateTime<Utc>> {
        self.get_timestamp()
            .map(|ts| ts.to_unix())
            .map(|(secs, nanos)| secs as i64 * 1_000_000_000 + nanos as i64)
            .map(DateTime::from_timestamp_nanos)
    }
}
