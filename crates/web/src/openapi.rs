//! Post-processing shared by the generated OpenAPI documents: parameter
//! descriptions and tighter parameter schemas, keyed by parameter name.

use aide::openapi::{OpenApi, Operation, ParameterSchemaOrContent, ReferenceOr};
use rustlog_domain::tiers::DEFAULT_EXCLUDED_BOTS;
use serde_json::json;

/// Fills in what aide cannot infer from the parameter types.
pub fn enrich_openapi(api: &mut OpenApi) {
    let Some(paths) = &mut api.paths else {
        return;
    };

    for path_item in paths.paths.values_mut() {
        let ReferenceOr::Item(path_item) = path_item else {
            continue;
        };

        if let Some(operation) = &mut path_item.get {
            enrich_operation(operation);
        }
        if let Some(operation) = &mut path_item.post {
            enrich_operation(operation);
        }
        if let Some(operation) = &mut path_item.delete {
            enrich_operation(operation);
        }
        if let Some(operation) = &mut path_item.put {
            enrich_operation(operation);
        }
        if let Some(operation) = &mut path_item.patch {
            enrich_operation(operation);
        }
    }
}

fn enrich_operation(operation: &mut Operation) {
    for parameter in &mut operation.parameters {
        let Some(parameter) = parameter.as_item_mut() else {
            continue;
        };

        {
            let data = parameter.parameter_data_mut();
            if data.description.is_none() {
                data.description = parameter_description(&data.name).map(str::to_owned);
            }
            if let ParameterSchemaOrContent::Schema(schema) = &mut data.format {
                enrich_parameter_schema(&data.name, schema);
            }
            if data.name == "exclude_bots" {
                data.explode = Some(false);
            }
        }
    }
}

fn parameter_description(name: &str) -> Option<&'static str> {
    Some(match name {
        "channel_id_type" => {
            "Use `channel` for a Twitch login or `channelid` for a Twitch user id."
        }
        "user_id_type" => "Use `user` for a Twitch login or `userid` for a Twitch user id.",
        "channel" => "Twitch channel login or id, depending on the selected channel id type.",
        "channelid" => {
            "Twitch channel user id. Use this instead of `channel` when you already know the id."
        }
        "user" => "Twitch user login or id, depending on the selected user id type.",
        "userid" => "Twitch user id. Use this instead of `user` when you already know the id.",
        "year" => "UTC year.",
        "month" => "UTC month number from 1 to 12.",
        "day" => "UTC day of month.",
        "from" => "RFC 3339 inclusive start timestamp.",
        "to" => "RFC 3339 exclusive end timestamp.",
        "q" => "Search text.",
        "json" => "Return full JSON messages.",
        "jsonBasic" => "Return compact JSON messages.",
        "raw" => "Return raw IRC lines.",
        "reverse" => "Return newest messages first.",
        "ndjson" => "Return newline-delimited JSON.",
        "limit" => "Maximum number of messages to return.",
        "offset" => "Number of messages to skip.",
        "mode" => "Tier mode: all messages, online stream windows, or offline windows.",
        "exclude_bots" => "Bot logins excluded from tier tables.",
        "X-Api-Key" => "Configured admin API key.",
        _ => return None,
    })
}

fn enrich_parameter_schema(name: &str, schema: &mut aide::openapi::SchemaObject) {
    match name {
        "channel_id_type" => set_schema(
            schema,
            json!({
                "type": "string",
                "enum": ["channel", "channelid"]
            }),
        ),
        "user_id_type" => set_schema(
            schema,
            json!({
                "type": "string",
                "enum": ["user", "userid"]
            }),
        ),
        "mode" => set_schema(
            schema,
            json!({
                "type": "string",
                "enum": ["all", "online", "offline"]
            }),
        ),
        "exclude_bots" => set_schema(
            schema,
            json!({
                "type": "array",
                "items": {
                    "type": "string",
                    "enum": DEFAULT_EXCLUDED_BOTS
                },
                "uniqueItems": true
            }),
        ),
        "json" | "jsonBasic" | "raw" | "reverse" | "ndjson" => {
            let object = schema.json_schema.ensure_object();
            object.insert("type".to_owned(), json!("boolean"));
            object.remove("default");
            object.remove("examples");
        }
        "month" => {
            set_schema(
                schema,
                json!({
                    "type": "integer",
                    "enum": [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12],
                    "minimum": 1,
                    "maximum": 12
                }),
            );
        }
        "limit" => {
            let object = schema.json_schema.ensure_object();
            object.insert("minimum".to_owned(), json!(1));
        }
        "offset" => {
            let object = schema.json_schema.ensure_object();
            object.insert("minimum".to_owned(), json!(0));
        }
        _ => clear_examples_and_defaults(schema),
    }
}

fn clear_examples_and_defaults(schema: &mut aide::openapi::SchemaObject) {
    let object = schema.json_schema.ensure_object();
    object.remove("example");
    object.remove("examples");
    object.remove("default");
}

fn set_schema(schema: &mut aide::openapi::SchemaObject, value: serde_json::Value) {
    schema.json_schema = value
        .try_into()
        .expect("OpenAPI parameter schema must be a JSON object");
}
