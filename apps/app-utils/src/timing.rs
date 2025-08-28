use std::time::{SystemTime, UNIX_EPOCH};

pub fn time_now() -> u64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_micros();
    now as u64
}

pub fn get_timestamp() -> u64 {
    // timestamps range from 0 to 9999 seconds (~166 mins)
    time_now() % 10_000_000_000
}
