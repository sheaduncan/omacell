//! `omacell mcp` — rmcp stdio and Unix-socket server over [`omacell_bus::mcp`].

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use omacell_bus::mcp::{McpCtx, McpSession, TOOLS};
use omacell_bus::{Bus, codes};
use omacell_conf::{Config, NotifyKind, notify_send};
use rmcp::handler::server::ServerHandler;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
    ListResourceTemplatesResult, ListResourcesResult, ListToolsResult, ReadResourceRequestParams,
    ReadResourceResponse, ReadResourceResult, Resource, ResourceContents, ResourceTemplate,
    ServerCapabilities, ServerInfo, Tool,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData as McpError, RoleServer, ServiceExt};
use serde_json::{Map, Value};

use crate::error::CliError;

/// Shared MCP server state.
#[derive(Clone)]
pub struct OmacellMcp {
    bus: Arc<Mutex<Bus>>,
    ctx: Arc<Mutex<McpCtx>>,
}

impl OmacellMcp {
    /// Wrap a live bus (shared with the WP-07b IPC server).
    #[must_use]
    pub fn new(bus: Arc<Mutex<Bus>>, ctx: Arc<Mutex<McpCtx>>) -> Self {
        Self { bus, ctx }
    }

    fn lock_bus(&self) -> std::sync::MutexGuard<'_, Bus> {
        self.bus.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn lock_ctx(&self) -> std::sync::MutexGuard<'_, McpCtx> {
        self.ctx.lock().unwrap_or_else(|p| p.into_inner())
    }
}

impl ServerHandler for OmacellMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
        )
        .with_server_info(Implementation::new("omacell", env!("CARGO_PKG_VERSION")).with_title("Omacell"))
        .with_instructions(
            "Prefer changeset proposals. Apply with omacell changeset apply. Recalc --wait and audit --json before declaring done.",
        )
    }

    async fn list_tools(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        Ok(ListToolsResult {
            tools: mcp_tools(),
            ..Default::default()
        })
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        let args = request
            .arguments
            .map(Value::Object)
            .unwrap_or(Value::Object(Map::new()));
        let mut bus = self.lock_bus();
        let mut ctx = self.lock_ctx();
        match McpSession::call(&mut bus, &mut ctx, &request.name, args) {
            Ok(value) => Ok(CallToolResult::success(vec![ContentBlock::text(
                serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string()),
            )])
            .into()),
            Err(err) => {
                if err.code == codes::MCP_UNKNOWN {
                    Err(McpError::invalid_params(err.message, None))
                } else {
                    Ok(CallToolResult::error(vec![ContentBlock::text(
                        serde_json::to_string(&err).unwrap_or(err.message),
                    )])
                    .into())
                }
            }
        }
    }

    async fn list_resources(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        let bus = self.lock_bus();
        let ctx = self.lock_ctx();
        let resources = McpSession::list_resources(&bus, &ctx)
            .into_iter()
            .filter_map(|value| {
                let uri = value.get("uri")?.as_str()?.to_string();
                let name = value.get("name")?.as_str()?.to_string();
                let mut resource = Resource::new(uri, name);
                if let Some(mime) = value.get("mimeType").and_then(Value::as_str) {
                    resource = resource.with_mime_type(mime);
                }
                Some(resource)
            })
            .collect();
        Ok(ListResourcesResult {
            resources,
            ..Default::default()
        })
    }

    async fn list_resource_templates(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, McpError> {
        Ok(ListResourceTemplatesResult {
            resource_templates: vec![
                ResourceTemplate::new("omacell://{file}/card", "card")
                    .with_description("Workbook card (summary until WP-22)")
                    .with_mime_type("application/json"),
                ResourceTemplate::new("omacell://{file}/{sheet}", "sheet")
                    .with_description("Sheet summary")
                    .with_mime_type("application/json"),
            ],
            ..Default::default()
        })
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, McpError> {
        let bus = self.lock_bus();
        let ctx = self.lock_ctx();
        match McpSession::read_resource(&bus, &ctx, &request.uri) {
            Ok(value) => {
                let text =
                    serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string());
                Ok(ReadResourceResult::new(vec![ResourceContents::text(text, request.uri)]).into())
            }
            Err(err) => Err(McpError::resource_not_found(err.message, None)),
        }
    }
}

fn mcp_tools() -> Vec<Tool> {
    TOOLS
        .iter()
        .map(|spec| {
            let schema = (spec.schema)();
            let object = schema.as_object().cloned().unwrap_or_else(|| {
                let mut map = Map::new();
                map.insert("type".into(), Value::String("object".into()));
                map
            });
            Tool::new(spec.name, spec.doc, std::sync::Arc::new(object))
        })
        .collect()
}

/// Serve MCP on stdio or a Unix socket. Blocks until the client disconnects.
pub async fn serve(handler: OmacellMcp, socket: Option<PathBuf>) -> Result<(), CliError> {
    if let Some(path) = socket {
        serve_socket(handler, &path).await
    } else {
        let running = handler
            .serve(rmcp::transport::stdio())
            .await
            .map_err(|err| CliError::new("mcp.serve", err.to_string()))?;
        running
            .waiting()
            .await
            .map_err(|err| CliError::new("mcp.serve", err.to_string()))?;
        Ok(())
    }
}

async fn serve_socket(handler: OmacellMcp, path: &Path) -> Result<(), CliError> {
    if path.exists() {
        std::fs::remove_file(path).map_err(|err| CliError::new("mcp.socket", err.to_string()))?;
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| CliError::new("mcp.socket", err.to_string()))?;
    }
    let listener = tokio::net::UnixListener::bind(path)
        .map_err(|err| CliError::new("mcp.socket", err.to_string()))?;
    loop {
        let (stream, _) = listener
            .accept()
            .await
            .map_err(|err| CliError::new("mcp.socket", err.to_string()))?;
        let handler = handler.clone();
        let running = handler
            .serve(stream)
            .await
            .map_err(|err| CliError::new("mcp.serve", err.to_string()))?;
        running
            .waiting()
            .await
            .map_err(|err| CliError::new("mcp.serve", err.to_string()))?;
    }
}

/// Hook an ExternalAgent proposal to the desktop notification adapter.
pub fn proposal_notifier(config: Config) -> omacell_bus::mcp::ProposeHook {
    Box::new(move |changeset| {
        notify_send(
            &config,
            NotifyKind::AgentProposal,
            "Omacell",
            &format!(
                "Changeset {} proposed by an external agent",
                changeset.id.as_str()
            ),
        );
    })
}
