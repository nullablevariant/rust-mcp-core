//! Plugin registry for registering, looking up, and managing plugin instances.

use std::{
    collections::{HashMap, HashSet},
    fmt,
    sync::Arc,
};

use tokio::sync::broadcast;

use super::traits::{AuthPlugin, CompletionPlugin, PromptPlugin, ResourcePlugin, ToolPlugin};
use crate::McpError;

#[cfg(feature = "streamable_http")]
use super::http_router::HttpRouterPlugin;

// ---------------------------------------------------------------------------
// PluginType / PluginRef / PluginLookup
// ---------------------------------------------------------------------------

/// Discriminant for the six plugin categories.
///
/// Used in config `plugins[].type` to declare which plugin type an entry represents,
/// and in [`PluginLookup::get_plugin`] to retrieve a plugin by category and name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginType {
    /// Custom tool execution logic ([`ToolPlugin`]).
    Tool,
    /// Additional token validation ([`AuthPlugin`]).
    Auth,
    /// Custom Axum routes/middleware ([`HttpRouterPlugin`]).
    HttpRouter,
    /// Autocompletion provider ([`CompletionPlugin`]).
    Completion,
    /// Prompt provider ([`PromptPlugin`]).
    Prompt,
    /// Resource provider ([`ResourcePlugin`]).
    Resource,
}

/// A type-erased handle to a registered plugin, returned by [`PluginLookup::get_plugin`].
#[doc(hidden)]
pub enum PluginRef {
    Tool(Arc<dyn ToolPlugin>),
    Auth(Arc<dyn AuthPlugin>),
    #[cfg(feature = "streamable_http")]
    HttpRouter(Arc<dyn HttpRouterPlugin>),
    Completion(Arc<dyn CompletionPlugin>),
    Prompt(Arc<dyn PromptPlugin>),
    Resource(Arc<dyn ResourcePlugin>),
}

impl fmt::Debug for PluginRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Tool(p) => f.debug_tuple("PluginRef::Tool").field(&p.name()).finish(),
            Self::Auth(p) => f.debug_tuple("PluginRef::Auth").field(&p.name()).finish(),
            #[cfg(feature = "streamable_http")]
            Self::HttpRouter(p) => f
                .debug_tuple("PluginRef::HttpRouter")
                .field(&p.name())
                .finish(),
            Self::Completion(p) => f
                .debug_tuple("PluginRef::Completion")
                .field(&p.name())
                .finish(),
            Self::Prompt(p) => f.debug_tuple("PluginRef::Prompt").field(&p.name()).finish(),
            Self::Resource(p) => f
                .debug_tuple("PluginRef::Resource")
                .field(&p.name())
                .finish(),
        }
    }
}

// Trait for looking up registered plugins by type and name.
//
// Implemented by `PluginRegistry`. The engine uses this at startup to resolve
// plugin references from config.
#[doc(hidden)]
pub trait PluginLookup: Send + Sync {
    // Retrieve a plugin by its type and name, or `None` if not registered.
    fn get_plugin(&self, plugin_type: PluginType, name: &str) -> Option<PluginRef>;
    // List all registered plugin names with their types.
    fn names(&self) -> Vec<(String, PluginType)>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[doc(hidden)]
pub enum RegistryEvent {
    ToolChanged,
    PromptChanged,
    ResourceChanged,
}

// ---------------------------------------------------------------------------
// Internal typed registry (generic helper)
// ---------------------------------------------------------------------------

struct TypedRegistry<T: ?Sized> {
    plugins: HashMap<String, Arc<T>>,
}

impl<T: ?Sized> TypedRegistry<T> {
    fn new() -> Self {
        Self {
            plugins: HashMap::new(),
        }
    }

    fn insert(&mut self, name: String, plugin: Arc<T>) -> Option<Arc<T>> {
        self.plugins.insert(name, plugin)
    }

    fn get(&self, name: &str) -> Option<Arc<T>> {
        self.plugins.get(name).cloned()
    }

    fn remove(&mut self, name: &str) -> Option<Arc<T>> {
        self.plugins.remove(name)
    }

    fn names(&self) -> Vec<String> {
        self.plugins.keys().cloned().collect()
    }
}

impl<T: ?Sized> Default for TypedRegistry<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: ?Sized> Clone for TypedRegistry<T> {
    fn clone(&self) -> Self {
        Self {
            plugins: self.plugins.clone(),
        }
    }
}

// ---------------------------------------------------------------------------
// Unified PluginRegistry
// ---------------------------------------------------------------------------

/// Unified registry holding all plugin types.
///
/// Build a registry with the builder-style `register_*` methods, then pass it
/// to [`run_from_config`](crate::runtime::run_from_config). Every plugin
/// declared in config `plugins[]` must be registered here; missing plugins
/// cause a startup error.
///
/// # Examples
///
/// ```rust
/// use rust_mcp_core::PluginRegistry;
/// # use rust_mcp_core::{
/// #     mcp::{CallToolResult, Content, McpError},
/// #     PluginCallParams, ToolPlugin,
/// # };
/// #
/// # struct EchoPlugin;
/// #
/// # #[async_trait::async_trait]
/// # impl ToolPlugin for EchoPlugin {
///     fn name(&self) -> &str {
///         "echo"
///     }
///
///     async fn call(
///         &self,
///         _args: serde_json::Value,
///         _params: PluginCallParams,
///     ) -> Result<CallToolResult, McpError> {
///         Ok(CallToolResult::success(vec![
///             Content::text("ok"),
///         ]))
///     }
/// }
///
/// let plugins = PluginRegistry::new()
///     .register_tool(EchoPlugin)
///     .expect("register tool plugin");
/// # let _ = plugins;
/// ```
#[derive(Clone)]
pub struct PluginRegistry {
    tools: TypedRegistry<dyn ToolPlugin>,
    auth: TypedRegistry<dyn AuthPlugin>,
    #[cfg(feature = "streamable_http")]
    http_router: TypedRegistry<dyn HttpRouterPlugin>,
    completion: TypedRegistry<dyn CompletionPlugin>,
    prompt: TypedRegistry<dyn PromptPlugin>,
    resource: TypedRegistry<dyn ResourcePlugin>,
    all_names: HashSet<String>,
    event_tx: broadcast::Sender<RegistryEvent>,
}

impl Default for PluginRegistry {
    fn default() -> Self {
        let (event_tx, _event_rx) = broadcast::channel(64);
        Self {
            tools: TypedRegistry::default(),
            auth: TypedRegistry::default(),
            #[cfg(feature = "streamable_http")]
            http_router: TypedRegistry::default(),
            completion: TypedRegistry::default(),
            prompt: TypedRegistry::default(),
            resource: TypedRegistry::default(),
            all_names: HashSet::new(),
            event_tx,
        }
    }
}

impl fmt::Debug for PluginRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PluginRegistry")
            .field("tools", &self.tools.names())
            .field("auth", &self.auth.names())
            .field("completion", &self.completion.names())
            .field("prompt", &self.prompt.names())
            .field("resource", &self.resource.names())
            .finish_non_exhaustive()
    }
}

macro_rules! define_register_method {
    ($method:ident, $trait_name:ident, $field:ident, $plugin_type:expr) => {
        pub fn $method<T: $trait_name + 'static>(mut self, plugin: T) -> Result<Self, McpError> {
            let name = plugin.name().to_string();
            self.check_name_available(&name)?;
            self.$field.insert(name.clone(), Arc::new(plugin));
            self.all_names.insert(name);
            self.emit_type_event($plugin_type);
            Ok(self)
        }
    };
}

macro_rules! define_unregister_method {
    ($method:ident, $field:ident, $plugin_type:expr) => {
        pub fn $method(mut self, name: &str) -> Result<Self, McpError> {
            if self.$field.remove(name).is_none() {
                return Err(McpError::invalid_request(
                    format!("{} plugin not registered: {}", $plugin_type.as_str(), name),
                    None,
                ));
            }
            self.all_names.remove(name);
            self.emit_type_event($plugin_type);
            Ok(self)
        }
    };
}

macro_rules! define_replace_method {
    ($method:ident, $trait_name:ident, $field:ident, $plugin_type:expr) => {
        pub fn $method<T: $trait_name + 'static>(mut self, plugin: T) -> Result<Self, McpError> {
            let name = plugin.name().to_string();
            self.check_name_for_replace($plugin_type, &name, self.$field.get(&name).is_some())?;
            self.$field.insert(name, Arc::new(plugin));
            self.emit_type_event($plugin_type);
            Ok(self)
        }
    };
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    fn check_name_available(&self, name: &str) -> Result<(), McpError> {
        if self.all_names.contains(name) {
            return Err(McpError::invalid_request(
                format!("duplicate plugin name: {name}"),
                None,
            ));
        }
        Ok(())
    }

    fn check_name_for_replace(
        &self,
        plugin_type: PluginType,
        name: &str,
        exists_in_type: bool,
    ) -> Result<(), McpError> {
        if exists_in_type {
            return Ok(());
        }

        if self.all_names.contains(name) {
            return Err(McpError::invalid_request(
                format!(
                    "cannot replace {} plugin {}: name is registered as a different plugin type",
                    plugin_type.as_str(),
                    name
                ),
                None,
            ));
        }

        Err(McpError::invalid_request(
            format!("{} plugin not registered: {}", plugin_type.as_str(), name),
            None,
        ))
    }

    fn emit_event(&self, event: RegistryEvent) {
        let _ = self.event_tx.send(event);
    }

    fn emit_type_event(&self, plugin_type: PluginType) {
        let event = match plugin_type {
            PluginType::Tool => Some(RegistryEvent::ToolChanged),
            PluginType::Prompt => Some(RegistryEvent::PromptChanged),
            PluginType::Resource => Some(RegistryEvent::ResourceChanged),
            PluginType::Auth | PluginType::HttpRouter | PluginType::Completion => None,
        };
        if let Some(event) = event {
            self.emit_event(event);
        }
    }

    #[doc(hidden)]
    pub fn subscribe_events(&self) -> broadcast::Receiver<RegistryEvent> {
        self.event_tx.subscribe()
    }

    define_register_method!(register_tool, ToolPlugin, tools, PluginType::Tool);
    define_register_method!(register_auth, AuthPlugin, auth, PluginType::Auth);
    #[cfg(feature = "streamable_http")]
    define_register_method!(
        register_http_router,
        HttpRouterPlugin,
        http_router,
        PluginType::HttpRouter
    );
    define_register_method!(
        register_completion,
        CompletionPlugin,
        completion,
        PluginType::Completion
    );
    define_register_method!(register_prompt, PromptPlugin, prompt, PluginType::Prompt);
    define_register_method!(
        register_resource,
        ResourcePlugin,
        resource,
        PluginType::Resource
    );

    define_unregister_method!(unregister_tool, tools, PluginType::Tool);
    define_unregister_method!(unregister_prompt, prompt, PluginType::Prompt);
    define_unregister_method!(unregister_resource, resource, PluginType::Resource);

    define_replace_method!(replace_tool, ToolPlugin, tools, PluginType::Tool);
    define_replace_method!(replace_prompt, PromptPlugin, prompt, PluginType::Prompt);
    define_replace_method!(
        replace_resource,
        ResourcePlugin,
        resource,
        PluginType::Resource
    );
}

impl PluginType {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Tool => "tool",
            Self::Auth => "auth",
            Self::HttpRouter => "http_router",
            Self::Completion => "completion",
            Self::Prompt => "prompt",
            Self::Resource => "resource",
        }
    }
}

impl PluginLookup for PluginRegistry {
    fn get_plugin(&self, plugin_type: PluginType, name: &str) -> Option<PluginRef> {
        match plugin_type {
            PluginType::Tool => self.tools.get(name).map(PluginRef::Tool),
            PluginType::Auth => self.auth.get(name).map(PluginRef::Auth),
            PluginType::HttpRouter => {
                #[cfg(feature = "streamable_http")]
                {
                    self.http_router.get(name).map(PluginRef::HttpRouter)
                }
                #[cfg(not(feature = "streamable_http"))]
                {
                    let _ = name;
                    None
                }
            }
            PluginType::Completion => self.completion.get(name).map(PluginRef::Completion),
            PluginType::Prompt => self.prompt.get(name).map(PluginRef::Prompt),
            PluginType::Resource => self.resource.get(name).map(PluginRef::Resource),
        }
    }

    fn names(&self) -> Vec<(String, PluginType)> {
        let mut result = Vec::new();
        for name in self.tools.names() {
            result.push((name, PluginType::Tool));
        }
        for name in self.auth.names() {
            result.push((name, PluginType::Auth));
        }
        #[cfg(feature = "streamable_http")]
        for name in self.http_router.names() {
            result.push((name, PluginType::HttpRouter));
        }
        for name in self.completion.names() {
            result.push((name, PluginType::Completion));
        }
        for name in self.prompt.names() {
            result.push((name, PluginType::Prompt));
        }
        for name in self.resource.names() {
            result.push((name, PluginType::Resource));
        }
        result
    }
}
