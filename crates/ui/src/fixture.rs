//! Recorded app-server data from `fixtures/` that the parity screens render, so every
//! screen draws from the same real session the reference captures show.

use serde::de::DeserializeOwned;
use serde_json::Value;
use std::{collections::HashMap, fs, path::Path};

/// Result of the `nth` (0-based) `method` request in `fixtures/<file>` (e.g. `readback.jsonl`,
/// `sessions/subtract.jsonl`), decoded as `T`. Panics on a missing fixture: parity screens
/// cannot render without it.
pub fn response<T: DeserializeOwned>(file: &str, method: &str, nth: usize) -> T {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(file);
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut ids = Vec::new();
    let mut results = HashMap::new();
    for line in text.lines() {
        let entry: Value = serde_json::from_str(line).expect("fixture line is JSON");
        let msg = &entry["msg"];
        match entry["dir"].as_str() {
            Some("send") if msg["method"] == method => ids.push(msg["id"].clone()),
            Some("recv") if msg.get("result").is_some() => {
                results.insert(msg["id"].to_string(), msg["result"].clone());
            }
            _ => {}
        }
    }
    let id = ids
        .get(nth)
        .unwrap_or_else(|| panic!("{file}: no request #{nth} for {method}"));
    let result = results
        .remove(&id.to_string())
        .unwrap_or_else(|| panic!("{file}: no result for {method} #{nth}"));
    serde_json::from_value(result).unwrap_or_else(|e| panic!("{file}: {method} #{nth}: {e}"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn reads_thread_list() {
        let list: serde_json::Value = super::response("readback.jsonl", "thread/list", 0);
        assert_eq!(list["data"].as_array().map(Vec::len), Some(3));
    }
}
