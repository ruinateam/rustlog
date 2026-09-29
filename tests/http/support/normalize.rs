//! Makes response bodies deterministic for snapshots: message tags come from
//! hash maps, so JSON object keys and IRC tags are sorted.

use serde_json::{Map, Value};

pub fn body(body: &str) -> String {
    if let Ok(value) = serde_json::from_str::<Value>(body) {
        return serde_json::to_string_pretty(&canonical(value)).unwrap();
    }

    body.split('\n')
        .map(|line| match serde_json::from_str::<Value>(line) {
            Ok(value) => serde_json::to_string(&canonical(value)).unwrap(),
            Err(_) => sort_irc_tags(line),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn canonical(value: Value) -> Value {
    match value {
        Value::String(text) => Value::String(sort_irc_tags(&text)),
        Value::Array(items) => Value::Array(items.into_iter().map(canonical).collect()),
        Value::Object(object) => {
            let mut entries: Vec<_> = object.into_iter().collect();
            entries.sort_by(|(a, _), (b, _)| a.cmp(b));
            Value::Object(
                entries
                    .into_iter()
                    .map(|(key, value)| (key, canonical(value)))
                    .collect::<Map<_, _>>(),
            )
        }
        other => other,
    }
}

/// Sorts the tags of a raw IRC line (`@a=1;b=2 :prefix COMMAND ...`).
fn sort_irc_tags(line: &str) -> String {
    let Some(rest) = line.strip_prefix('@') else {
        return line.to_owned();
    };
    let Some((tags, message)) = rest.split_once(' ') else {
        return line.to_owned();
    };
    let mut tags: Vec<_> = tags.split(';').collect();
    tags.sort_unstable();
    format!("@{} {message}", tags.join(";"))
}
