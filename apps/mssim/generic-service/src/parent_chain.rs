use sim_config::svc::ServiceName;
use std::str::FromStr;
use tonic::metadata::{MetadataMap, MetadataValue};
use tonic::Status;

pub(crate) const PARENT_CHAIN_METADATA_KEY: &str = "parent-chain";
const PARENT_CHAIN_DELIMITER: char = '>';

pub(crate) fn encode_parent_chain(
    chain: &[ServiceName],
) -> Result<Option<MetadataValue<tonic::metadata::Ascii>>, Status> {
    if chain.is_empty() {
        return Ok(None);
    }

    let delimiter = PARENT_CHAIN_DELIMITER.to_string();
    let encoded = chain
        .iter()
        .map(|svc| svc.as_str())
        .collect::<Vec<_>>()
        .join(&delimiter);

    MetadataValue::from_str(&encoded)
        .map(Some)
        .map_err(|_| Status::internal("Failed to encode parent chain metadata"))
}

pub(crate) fn decode_parent_chain(metadata: &MetadataMap) -> Result<Vec<ServiceName>, Status> {
    let Some(value) = metadata.get(PARENT_CHAIN_METADATA_KEY) else {
        return Ok(Vec::new());
    };

    let parents_str = value
        .to_str()
        .map_err(|_| Status::invalid_argument("Parent chain metadata is not valid ASCII"))?;

    if parents_str.is_empty() {
        return Ok(Vec::new());
    }

    let parents = parents_str
        .split(PARENT_CHAIN_DELIMITER)
        .filter(|name| !name.is_empty())
        .map(|name| ServiceName::from_string(name.to_string()))
        .collect();

    Ok(parents)
}
