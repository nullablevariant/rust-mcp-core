#![allow(dead_code)]

use std::path::PathBuf;

use rust_mcp_core::config::McpConfig;
use serde::Deserialize;
use serde_json::Value;

#[path = "../config_common/mod.rs"]
mod config_common;

#[derive(Deserialize)]
pub(crate) struct EngineFixture {
    pub config: McpConfig,
    pub args: Value,
    pub expected: Value,
}

#[derive(Deserialize)]
pub(crate) struct EngineConfigFixture {
    pub config: McpConfig,
}

#[derive(Deserialize)]
pub(crate) struct EngineToolFixture {
    pub config: McpConfig,
    pub tool: String,
    pub args: Value,
}

#[derive(Deserialize)]
struct EngineFixturePayload {
    args: Value,
    expected: Value,
}

#[derive(Deserialize)]
struct EngineToolFixturePayload {
    tool: String,
    args: Value,
}

pub(crate) fn fixture_path(name: &str) -> PathBuf {
    config_common::fixture_path(name)
}

fn load_yaml_fixture<T: for<'de> Deserialize<'de>>(name: &str) -> T {
    let raw = std::fs::read_to_string(fixture_path(name)).expect("fixture should read");
    serde_yaml::from_str(&raw).expect("fixture should parse")
}

fn resolve_config_fixture_name(name: &str) -> String {
    let companion = format!("{name}_config");
    if fixture_path(&companion).exists() {
        companion
    } else {
        name.to_owned()
    }
}

fn load_fixture_config(name: &str) -> McpConfig {
    let config_fixture_name = resolve_config_fixture_name(name);
    config_common::load_config_fixture(&config_fixture_name)
}

pub(crate) fn load_fixture(name: &str) -> EngineFixture {
    let payload: EngineFixturePayload = load_yaml_fixture(name);
    EngineFixture {
        config: load_fixture_config(name),
        args: payload.args,
        expected: payload.expected,
    }
}

pub(crate) fn load_config_fixture(name: &str) -> EngineConfigFixture {
    EngineConfigFixture {
        config: load_fixture_config(name),
    }
}

pub(crate) fn load_tool_fixture(name: &str) -> EngineToolFixture {
    let payload: EngineToolFixturePayload = load_yaml_fixture(name);
    EngineToolFixture {
        config: load_fixture_config(name),
        tool: payload.tool,
        args: payload.args,
    }
}
