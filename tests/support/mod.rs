#![allow(dead_code)]
use serde::de::DeserializeOwned;
use serde_json::Value;
pub fn from_slice<T: DeserializeOwned>(bytes: &[u8]) -> serde_json::Result<T> {
    let envelope: Value = serde_json::from_slice(bytes)?;
    assert_eq!(
        envelope["schema_version"], 2,
        "expected v2 output envelope: {envelope}"
    );
    assert!(envelope["metadata"]["project"].is_string());
    serde_json::from_value(envelope["data"].clone())
}
pub fn from_str<T: DeserializeOwned>(text: &str) -> serde_json::Result<T> {
    from_slice(text.as_bytes())
}
