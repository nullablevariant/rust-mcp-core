//! Template rendering engine for `${field}`, `${field?}`, `${field|csv}`, `${field|default(v)}`, and `${$.path}` syntax.

use std::borrow::Cow;

use rmcp::ErrorData as McpError;
use serde_json::{Map, Value};

#[derive(Clone, Debug)]
pub struct RenderContext<'a> {
    args: &'a Value,
    response: Option<&'a Value>,
}

impl<'a> RenderContext<'a> {
    pub const fn new(args: &'a Value, response: Option<&'a Value>) -> Self {
        Self { args, response }
    }
}

enum Rendered {
    Value(Value),
    Omit,
}

#[derive(Debug)]
enum Filter {
    Default(Value),
    Csv,
}

pub fn render_value(template: &Value, ctx: &RenderContext<'_>) -> Result<Value, McpError> {
    match render_inner(template, ctx)? {
        Rendered::Value(value) => Ok(value),
        Rendered::Omit => Ok(Value::Null),
    }
}

// Recursively walks a JSON template value. Scalars pass through, strings get
// placeholder substitution, and arrays/objects recurse into children.
// Omit-tagged entries (from optional placeholders) are silently dropped.
fn render_inner(template: &Value, ctx: &RenderContext<'_>) -> Result<Rendered, McpError> {
    match template {
        Value::Null => Ok(Rendered::Value(Value::Null)),
        Value::Bool(value) => Ok(Rendered::Value(Value::Bool(*value))),
        Value::Number(value) => Ok(Rendered::Value(Value::Number(value.clone()))),
        Value::String(value) => render_string(value, ctx),
        Value::Array(values) => {
            let mut out = Vec::with_capacity(values.len());
            for value in values {
                match render_inner(value, ctx)? {
                    Rendered::Value(rendered) => out.push(rendered),
                    Rendered::Omit => {}
                }
            }
            Ok(Rendered::Value(Value::Array(out)))
        }
        Value::Object(map) => {
            let mut out = Map::new();
            for (key, value) in map {
                match render_inner(value, ctx)? {
                    Rendered::Value(rendered) => {
                        out.insert(key.clone(), rendered);
                    }
                    Rendered::Omit => {}
                }
            }
            Ok(Rendered::Value(Value::Object(out)))
        }
    }
}

// Handles string template rendering. If the string is a single placeholder
// like "${field}", preserves the original JSON type (number, object, etc.).
// If mixed with literals like "Hello ${name}", concatenates as a string.
fn render_string(value: &str, ctx: &RenderContext<'_>) -> Result<Rendered, McpError> {
    let trimmed = value.trim();
    if let Some(expr) = single_placeholder(trimmed) {
        return render_expression(expr, ctx);
    }

    let mut output = String::new();
    let mut rest = value;
    while let Some(start) = rest.find("${") {
        output.push_str(&rest[..start]);
        rest = &rest[start + 2..];
        let Some(end) = rest.find('}') else {
            return Err(McpError::invalid_params(
                "unterminated template expression".to_owned(),
                None,
            ));
        };
        let expr = &rest[..end];
        rest = &rest[end + 1..];

        match render_expression(expr, ctx)? {
            Rendered::Value(value) => output.push_str(&value_to_string(&value)),
            Rendered::Omit => {}
        }
    }
    output.push_str(rest);
    Ok(Rendered::Value(Value::String(output)))
}

fn single_placeholder(value: &str) -> Option<&str> {
    if value.starts_with("${") && value.ends_with('}') {
        let inner = &value[2..value.len() - 1];
        if !inner.contains("${") && !inner.contains('}') {
            return Some(inner);
        }
    }
    None
}

// Evaluates a single expression: resolves the base path, applies filters
// (default, csv) in order, then returns the value or Omit for optional
// missing values. Required missing values produce an error.
fn render_expression(expr: &str, ctx: &RenderContext<'_>) -> Result<Rendered, McpError> {
    let (base, optional, filters) = parse_expression(expr)?;
    let mut value = resolve_base(&base, ctx).map(Cow::Borrowed);

    for filter in filters {
        match filter {
            Filter::Default(default_value) => {
                if is_missing(value.as_deref()) {
                    value = Some(Cow::Owned(default_value));
                }
            }
            Filter::Csv => {
                if let Some(current) = value.take() {
                    value = Some(Cow::Owned(Value::String(to_csv_string(current.as_ref()))));
                }
            }
        }
    }

    if is_missing(value.as_deref()) {
        if optional {
            return Ok(Rendered::Omit);
        }
        return Err(McpError::invalid_params(
            format!("missing required value for '{base}'"),
            None,
        ));
    }

    // Safe: is_missing(value.as_deref()) above returned false, so value is Some(non-Null).
    let value = value
        .expect("value is non-missing after is_missing check")
        .into_owned();
    Ok(Rendered::Value(value))
}

// Parses "field|csv|default(42)?" into (base_path, is_optional, filters).
// Trailing '?' marks the expression as optional; pipe-separated filters
// are applied left to right during rendering.
fn parse_expression(expr: &str) -> Result<(String, bool, Vec<Filter>), McpError> {
    let trimmed = expr.trim();
    let optional = trimmed.ends_with('?');
    let expr = trimmed.trim_end_matches('?');

    let mut parts = expr.split('|');
    let base = parts.next().unwrap_or("").trim().to_owned();

    if base.is_empty() {
        return Err(McpError::invalid_params(
            "empty template expression".to_owned(),
            None,
        ));
    }

    let mut filters = Vec::new();
    for raw in parts {
        let filter = raw.trim();
        if filter.eq_ignore_ascii_case("csv") {
            filters.push(Filter::Csv);
            continue;
        }

        if let Some(inner) = filter
            .strip_prefix("default(")
            .and_then(|s| s.strip_suffix(')'))
        {
            filters.push(Filter::Default(parse_default_literal(inner)));
            continue;
        }

        return Err(McpError::invalid_params(
            format!("unsupported filter '{filter}'"),
            None,
        ));
    }

    Ok((base, optional, filters))
}

fn parse_default_literal(raw: &str) -> Value {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Value::Null;
    }

    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        return value;
    }

    Value::String(trimmed.to_owned())
}

// Resolves the base value: "$" returns the full response, "$.path" drills
// into the response, anything else drills into tool args.
fn resolve_base<'a>(expr: &str, ctx: &'a RenderContext<'a>) -> Option<&'a Value> {
    if expr == "$" {
        return ctx.response;
    }

    if let Some(path) = expr.strip_prefix("$.") {
        return ctx.response.and_then(|value| select_path(value, path));
    }

    select_path(ctx.args, expr)
}

// Traverses a JSON value by a dot/bracket path like "data.items[0].name".
fn select_path<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let tokens = parse_path(path);
    let mut current = value;

    for token in tokens {
        match token {
            PathToken::Field(name) => {
                current = current.get(&name)?;
            }
            PathToken::Index(index) => {
                let array = current.as_array()?;
                current = array.get(index)?;
            }
        }
    }

    Some(current)
}

enum PathToken {
    Field(String),
    Index(usize),
}

// Tokenizes a dotted path with optional bracket indexing: "a.b[0].c" becomes
// [Field("a"), Field("b"), Index(0), Field("c")].
fn parse_path(path: &str) -> Vec<PathToken> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut chars = path.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '.' => {
                if !current.is_empty() {
                    tokens.push(PathToken::Field(current.clone()));
                    current.clear();
                }
            }
            '[' => {
                if !current.is_empty() {
                    tokens.push(PathToken::Field(current.clone()));
                    current.clear();
                }
                let mut index_str = String::new();
                for next in chars.by_ref() {
                    if next == ']' {
                        break;
                    }
                    index_str.push(next);
                }
                if let Ok(index) = index_str.parse::<usize>() {
                    tokens.push(PathToken::Index(index));
                }
            }
            _ => current.push(ch),
        }
    }

    if !current.is_empty() {
        tokens.push(PathToken::Field(current));
    }

    tokens
}

const fn is_missing(value: Option<&Value>) -> bool {
    matches!(value, None | Some(Value::Null))
}

pub(super) fn value_to_string(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Number(value) => value.to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Null => "null".to_owned(),
        _ => value.to_string(),
    }
}

fn to_csv_string(value: &Value) -> String {
    match value {
        Value::Array(items) => items
            .iter()
            .map(value_to_string)
            .collect::<Vec<_>>()
            .join(","),
        _ => value_to_string(value),
    }
}
