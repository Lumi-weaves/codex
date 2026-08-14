use codex_protocol::error::CodexErr;
use codex_protocol::error::Result as CodexResult;
use serde::Serialize;
use serde_json::Value;
use sha2::Digest;
use sha2::Sha256;

pub(crate) fn canonical_json_bytes<T: Serialize>(value: &T) -> CodexResult<Vec<u8>> {
    let value = serde_json::to_value(value)
        .map_err(|err| CodexErr::Fatal(format!("failed to serialize prompt receipt: {err}")))?;
    serde_json::to_vec(&canonicalize_json(value))
        .map_err(|err| CodexErr::Fatal(format!("failed to encode prompt receipt: {err}")))
}

fn canonicalize_json(value: Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.into_iter().map(canonicalize_json).collect()),
        Value::Object(values) => {
            let mut entries = values.into_iter().collect::<Vec<_>>();
            entries.sort_unstable_by(|(left, _), (right, _)| left.cmp(right));
            Value::Object(
                entries
                    .into_iter()
                    .map(|(key, value)| (key, canonicalize_json(value)))
                    .collect(),
            )
        }
        scalar => scalar,
    }
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
