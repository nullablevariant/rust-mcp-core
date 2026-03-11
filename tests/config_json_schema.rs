use rust_mcp_core::config::config_schema;
use serde_json::Value;

// TODO: Expand schema-parity assertions to cover all top-level config fields so
// tests that construct McpConfig directly cannot mask schema drift.

#[test]
fn config_schema_returns_json_object() {
    let schema = config_schema();
    assert!(schema.is_object());
}

#[test]
fn config_schema_has_top_level_keys() {
    let schema = config_schema();
    let object = schema.as_object().expect("schema should be an object");
    assert!(object.contains_key("type"));
    assert!(object.contains_key("properties"));
    assert!(object.contains_key("required"));

    // Assert key values/types, not just existence
    assert_eq!(
        object.get("type"),
        Some(&Value::String("object".to_owned())),
        "top-level type must be 'object'"
    );

    let required = object
        .get("required")
        .and_then(|v| v.as_array())
        .expect("required must be an array");
    let required_strings: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
    assert_eq!(
        required_strings,
        vec!["version"],
        "required array must contain exactly ['version']"
    );

    assert_eq!(
        object.get("title"),
        Some(&Value::String("MCP Config".to_owned())),
        "schema title must be 'MCP Config'"
    );
}

#[test]
fn config_schema_type_is_object() {
    let schema = config_schema();
    assert_eq!(
        schema.get("type"),
        Some(&Value::String("object".to_owned()))
    );
}

#[test]
fn config_schema_required_contains_core_fields() {
    let schema = config_schema();
    let required = schema
        .get("required")
        .and_then(|v| v.as_array())
        .expect("required should be an array");
    let required_strings: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
    assert!(required_strings.contains(&"version"));
    assert!(!required_strings.contains(&"server"));
    assert!(!required_strings.contains(&"tools"));
}

#[test]
fn config_schema_properties_contain_known_keys() {
    let schema = config_schema();
    let properties = schema
        .get("properties")
        .and_then(|v| v.as_object())
        .expect("properties should be an object");
    assert!(properties.contains_key("server"));
    assert!(properties.contains_key("tools"));
    assert!(properties.contains_key("version"));
    assert!(properties.contains_key("outbound_http"));
    assert!(properties.contains_key("upstreams"));
    assert!(properties.contains_key("client_logging"));
    assert!(properties.contains_key("progress"));
    assert!(properties.contains_key("prompts"));
    assert!(properties.contains_key("resources"));
    assert!(properties.contains_key("plugins"));
    assert!(!properties.contains_key("server_info"));
    assert!(!properties.contains_key("instructions"));
}

#[test]
fn config_schema_critical_property_structures() {
    let schema = config_schema();
    let properties = schema
        .get("properties")
        .and_then(|v| v.as_object())
        .expect("properties should be an object");

    let tools = properties.get("tools").expect("tools must exist");
    let tools_types: Vec<&str> = tools
        .get("type")
        .and_then(Value::as_array)
        .expect("tools type should be an array")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(
        tools_types.contains(&"object") && tools_types.contains(&"null"),
        "tools must accept object or null"
    );
    let tools_properties = tools
        .get("properties")
        .and_then(Value::as_object)
        .expect("tools must expose properties");
    assert!(
        tools_properties.contains_key("notify_list_changed"),
        "tools must define notify_list_changed"
    );
    assert!(
        tools_properties.contains_key("items"),
        "tools must define items"
    );

    let version = properties.get("version").expect("version must exist");
    assert_eq!(
        version.get("type").and_then(Value::as_str),
        Some("integer"),
        "version must be integer type"
    );
    assert_eq!(
        version.get("minimum").and_then(Value::as_u64),
        Some(1),
        "version minimum must be 1"
    );

    let mcp_prop = properties.get("server").expect("server must exist");
    assert_eq!(
        mcp_prop.get("type").and_then(Value::as_str),
        Some("object"),
        "server must be object type"
    );
    assert!(
        mcp_prop.get("properties").is_some(),
        "server must have properties"
    );

    let upstreams = properties.get("upstreams").expect("upstreams must exist");
    assert_eq!(
        upstreams.get("type").and_then(Value::as_str),
        Some("object"),
        "upstreams must be object type"
    );
    assert!(
        upstreams.get("additionalProperties").is_some(),
        "upstreams must have additionalProperties"
    );
}

#[test]
fn config_schema_server_has_auth_section() {
    let schema = config_schema();
    let server = schema
        .get("properties")
        .and_then(|v| v.get("server"))
        .and_then(|v| v.get("properties"))
        .and_then(|v| v.as_object())
        .expect("server properties should be an object");
    assert!(server.contains_key("auth"));
    assert!(server.contains_key("host"));
    assert!(server.contains_key("port"));
    assert!(server.contains_key("transport"));
    assert!(server.contains_key("logging"));

    // Auth section allows object/null and carries enabled/providers/oauth.
    let auth = server.get("auth").expect("auth must exist");
    let auth_types: Vec<&str> = auth
        .get("type")
        .and_then(Value::as_array)
        .expect("auth type must be array")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(
        auth_types.contains(&"object") && auth_types.contains(&"null"),
        "auth must allow object|null"
    );
    let auth_props = auth
        .get("properties")
        .and_then(Value::as_object)
        .expect("auth must have properties");
    assert!(auth_props.contains_key("enabled"), "auth must have enabled");
    assert!(auth_props.contains_key("oauth"), "auth must have oauth");

    // auth.providers must be an array of tagged provider objects
    let providers = auth_props
        .get("providers")
        .expect("auth must have providers");
    assert_eq!(
        providers.get("type").and_then(Value::as_str),
        Some("array"),
        "providers must be array type"
    );
    let provider_ref = providers
        .get("items")
        .and_then(|items| items.get("$ref"))
        .and_then(Value::as_str)
        .expect("providers items must reference authProvider def");
    assert_eq!(provider_ref, "#/$defs/authProvider");

    let auth_provider_def = schema
        .get("$defs")
        .and_then(|defs| defs.get("authProvider"))
        .and_then(|def| def.get("oneOf"))
        .and_then(Value::as_array)
        .expect("authProvider def must be oneOf");
    assert_eq!(
        auth_provider_def.len(),
        4,
        "authProvider must define 4 variants"
    );
    let variant_types: Vec<&str> = auth_provider_def
        .iter()
        .filter_map(|variant| {
            variant
                .get("properties")
                .and_then(|properties| properties.get("type"))
                .and_then(|typ| typ.get("const"))
                .and_then(Value::as_str)
        })
        .collect();
    assert_eq!(
        variant_types,
        vec!["bearer", "jwks", "introspection", "plugin"],
        "authProvider variant tags must match expected provider types"
    );
}

#[test]
fn config_schema_tools_items_have_required_fields() {
    let schema = config_schema();
    let tools_items = schema
        .get("properties")
        .and_then(|v| v.get("tools"))
        .and_then(|v| v.get("properties"))
        .and_then(|v| v.get("items"))
        .and_then(|v| v.get("items"))
        .and_then(|v| v.get("required"))
        .and_then(|v| v.as_array())
        .expect("tools items required should be an array");
    let required_strings: Vec<&str> = tools_items.iter().filter_map(|v| v.as_str()).collect();
    assert!(required_strings.contains(&"name"));
    assert!(required_strings.contains(&"description"));
    assert!(required_strings.contains(&"input_schema"));
    assert!(required_strings.contains(&"execute"));
}

#[test]
fn config_schema_server_info_and_instructions_have_expected_types() {
    let schema = config_schema();
    let properties = schema
        .get("properties")
        .and_then(|v| v.as_object())
        .expect("properties should be an object");

    let server_info_types = properties
        .get("server")
        .and_then(|v| v.get("properties"))
        .and_then(|v| v.get("info"))
        .and_then(|v| v.get("type"))
        .and_then(|v| v.as_array())
        .expect("server.info.type should be an array");
    let server_info_type_strings: Vec<&str> = server_info_types
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert!(server_info_type_strings.contains(&"object"));
    assert!(server_info_type_strings.contains(&"null"));

    let instructions_types = properties
        .get("server")
        .and_then(|v| v.get("properties"))
        .and_then(|v| v.get("info"))
        .and_then(|v| v.get("properties"))
        .and_then(|v| v.get("instructions"))
        .and_then(|v| v.get("type"))
        .and_then(|v| v.as_array())
        .expect("server.info.instructions.type should be an array");
    let instruction_type_strings: Vec<&str> = instructions_types
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert!(instruction_type_strings.contains(&"string"));
    assert!(instruction_type_strings.contains(&"null"));
}

fn find_oauth2_variant(schema: &Value) -> &Value {
    schema
        .get("$defs")
        .and_then(|v| v.get("upstreamAuth"))
        .and_then(|v| v.get("oneOf"))
        .and_then(|v| v.as_array())
        .expect("upstream auth variants should be defined")
        .iter()
        .find(|variant| {
            variant
                .get("properties")
                .and_then(|v| v.get("type"))
                .and_then(|v| v.get("const"))
                .and_then(Value::as_str)
                == Some("oauth2")
        })
        .expect("oauth2 auth variant must exist")
}

#[test]
fn config_schema_defines_upstream_oauth2_auth_variant() {
    let schema = config_schema();
    let oauth2_variant = find_oauth2_variant(schema);

    // Assert oauth2 variant required fields
    let required = oauth2_variant
        .get("required")
        .and_then(Value::as_array)
        .expect("oauth2 variant must have required array");
    let required_strings: Vec<&str> = required.iter().filter_map(Value::as_str).collect();
    assert!(
        required_strings.contains(&"type"),
        "oauth2 variant must require 'type'"
    );
    assert!(
        required_strings.contains(&"grant"),
        "oauth2 variant must require 'grant'"
    );
    assert!(
        required_strings.contains(&"token_url"),
        "oauth2 variant must require 'token_url'"
    );
    assert!(
        required_strings.contains(&"client_id"),
        "oauth2 variant must require 'client_id'"
    );
    assert!(
        required_strings.contains(&"client_secret"),
        "oauth2 variant must require 'client_secret'"
    );

    // Assert additionalProperties: false for strictness
    assert_eq!(
        oauth2_variant
            .get("additionalProperties")
            .and_then(Value::as_bool),
        Some(false),
        "oauth2 variant must disallow additional properties"
    );
}

#[test]
fn config_schema_upstream_oauth2_variant_properties() {
    let schema = config_schema();
    let oauth2_variant = find_oauth2_variant(schema);

    let props = oauth2_variant
        .get("properties")
        .and_then(Value::as_object)
        .expect("oauth2 variant must have properties");

    let grant = props.get("grant").expect("oauth2 must have grant property");
    let grant_values: Vec<&str> = grant
        .get("enum")
        .and_then(Value::as_array)
        .expect("grant must have enum")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert_eq!(
        grant_values,
        vec!["client_credentials", "refresh_token"],
        "grant enum must have expected values"
    );

    assert!(
        props.contains_key("token_url"),
        "oauth2 must have token_url property"
    );
    assert!(
        props.contains_key("client_id"),
        "oauth2 must have client_id property"
    );
    assert!(
        props.contains_key("client_secret"),
        "oauth2 must have client_secret property"
    );
    assert!(
        props.contains_key("scopes"),
        "oauth2 must have scopes property"
    );
    assert!(props.contains_key("mtls"), "oauth2 must have mtls property");
}
