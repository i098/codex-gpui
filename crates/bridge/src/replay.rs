//! Fake app-server that answers from a recorded transcript.
//!
//! Format: one JSON object per line, `{"t": ms, "dir": "send" | "recv", "msg": {...}}`, where
//! `send` is client to server. Each live client message is matched to the first unused recorded
//! `send`: requests by method + params (ignoring `id`), falling back to the first unused request
//! of the same method; notifications by method + params; replies to server requests by `id`.
//! A matched request is answered with its recorded response re-keyed to the live id. Recorded
//! notifications and server requests are emitted, in order, after the `send` or response that
//! precedes them in the recording. Timing (`t`) is ignored.

use std::io::{BufRead, Write};
use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
pub(crate) struct Line {
    dir: String,
    msg: Value,
}

impl Line {
    fn is_send(&self) -> bool {
        self.dir == "send"
    }

    fn is_response(&self) -> bool {
        self.dir == "recv" && self.msg.get("method").is_none()
    }
}

pub(crate) fn load(path: &Path) -> Result<Vec<Line>> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {path:?}"))?;
    text.lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(n, line)| {
            serde_json::from_str(line).with_context(|| format!("{path:?} line {}", n + 1))
        })
        .collect()
}

pub(crate) fn serve(script: &[Line], input: impl BufRead, mut out: impl Write) {
    let mut used = vec![false; script.len()];
    for line in input.lines() {
        let Ok(live) = line.and_then(|l| Ok(serde_json::from_str::<Value>(&l)?)) else {
            break;
        };
        let is_request = live.get("method").is_some() && live.get("id").is_some();
        let mut reply = Vec::new();
        match find(script, &used, &live) {
            Some(i) => {
                used[i] = true;
                reply.extend(tail(script, i));
                if is_request {
                    let recorded_id = script[i].msg.get("id");
                    let response = script[i..]
                        .iter()
                        .position(|l| l.is_response() && l.msg.get("id") == recorded_id)
                        .map(|offset| i + offset);
                    match response {
                        Some(r) => {
                            let mut msg = script[r].msg.clone();
                            msg["id"] = live["id"].clone();
                            reply.push(msg);
                            reply.extend(tail(script, r));
                        }
                        None => reply.push(error(&live, "has no recorded response")),
                    }
                }
            }
            None if is_request => reply.push(error(&live, "was not recorded")),
            None => {}
        }
        for msg in reply {
            if writeln!(out, "{msg}").is_err() {
                return;
            }
        }
    }
}

/// Recorded notifications and server requests right after line `i`.
fn tail(script: &[Line], i: usize) -> impl Iterator<Item = Value> + '_ {
    script[i + 1..]
        .iter()
        .take_while(|l| !l.is_send() && !l.is_response())
        .map(|l| l.msg.clone())
}

fn find(script: &[Line], used: &[bool], live: &Value) -> Option<usize> {
    let params = |m: &Value| m.get("params").filter(|p| !p.is_null()).cloned();
    let method = live.get("method");
    let has_id = live.get("id").is_some();
    let unused_sends = || {
        script
            .iter()
            .enumerate()
            .filter(|&(i, l)| !used[i] && l.is_send())
            .map(|(i, l)| (i, &l.msg))
    };
    let same_kind = |m: &Value| m.get("method") == method && m.get("id").is_some() == has_id;
    let exact = unused_sends().find(|(_, m)| match method {
        Some(_) => same_kind(m) && params(m) == params(live),
        None => m.get("method").is_none() && m.get("id") == live.get("id"),
    });
    // ponytail: params drift (cwd, client name) falls back to recording order per method.
    let by_method = || unused_sends().find(|(_, m)| method.is_some() && has_id && same_kind(m));
    exact.or_else(by_method).map(|(i, _)| i)
}

fn error(live: &Value, why: &str) -> Value {
    let method = live["method"].as_str().unwrap_or_default();
    json!({ "id": live["id"], "error": { "code": -32601, "message": format!("replay: {method} {why}") } })
}
