use std::{future::Future, time::Duration};
use tokio::time::sleep;

/// Retries the given async operation until it returns `Ok`,
/// with exponential backoff between retries.
///
/// # Arguments
/// * `op` - An async function or closure that returns a `Result<T, E>`.
/// * `base_delay` - Initial delay before first retry.
/// * `max_delay` - Maximum backoff delay to cap growth.
///
/// # Returns
/// * The successful value of type `T`.
pub async fn retry_until_ok<T, E, Fut, F>(mut op: F, base_delay: Duration, max_delay: Duration) -> T
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
{
    let mut delay = base_delay;

    loop {
        match op().await {
            Ok(val) => return val,
            Err(_) => {
                sleep(delay).await;
                delay = (delay * 2).min(max_delay);
            }
        }
    }
}
