mod client;
mod server;
pub mod unique_id_service {
    tonic::include_proto!("uniqueidservice");
}

#[cfg(test)]
mod tests {
    use super::*;

    // [TODO] add integration test
}
