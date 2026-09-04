//! `omacell mcp` — rmcp stdio and Unix-socket server over [`omacell_bus::mcp`].

use std::io;
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use omacell_ai::card::{CardLevel, CardRequest};
use omacell_ai::policy::{PolicySnapshot, build_card, provider_is_local};
use omacell_ai::{Slot, route_slot};
use omacell_bus::mcp::{McpCtx, McpSession, TOOLS, stub_card};
use omacell_bus::{Bus, codes};
use omacell_conf::{Config, NotifyKind, ReloadHandle, notify_send};
use omacell_core::workbook::Workbook;
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
use tokio::io::{AsyncRead, ReadBuf};
use tokio::sync::Semaphore;

use crate::error::CliError;

/// Maximum accepted JSON-RPC line, including the MCP envelope.
pub const MAX_MCP_FRAME_BYTES: usize = 2 * 1_048_576;
/// Maximum simultaneous clients on the optional Unix socket.
pub const MAX_MCP_CONNECTIONS: usize = 32;

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
                    .with_description("Workbook card (summary)")
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
        let (stdin, stdout) = rmcp::transport::stdio();
        let running = handler
            .serve((BoundedLines::new(stdin), stdout))
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
    prepare_socket_path(path)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| CliError::new("mcp.socket", err.to_string()))?;
    }
    let listener = tokio::net::UnixListener::bind(path)
        .map_err(|err| CliError::new("mcp.socket", err.to_string()))?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .map_err(|err| CliError::new("mcp.socket", err.to_string()))?;
    let _socket_guard = SocketGuard(path.to_path_buf());
    let permits = Arc::new(Semaphore::new(MAX_MCP_CONNECTIONS));
    loop {
        let (stream, _) = listener
            .accept()
            .await
            .map_err(|err| CliError::new("mcp.socket", err.to_string()))?;
        let permit = permits
            .clone()
            .acquire_owned()
            .await
            .map_err(|err| CliError::new("mcp.socket", err.to_string()))?;
        let handler = handler.clone();
        tokio::spawn(async move {
            let _permit = permit;
            let (read, write) = tokio::io::split(stream);
            match handler.serve((BoundedLines::new(read), write)).await {
                Ok(running) => {
                    if let Err(error) = running.waiting().await {
                        tracing::debug!(%error, "MCP socket client stopped with an error");
                    }
                }
                Err(error) => tracing::debug!(%error, "MCP socket handshake failed"),
            }
        });
    }
}

fn prepare_socket_path(path: &Path) -> Result<(), CliError> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_socket() => {
            std::fs::remove_file(path).map_err(|err| CliError::new("mcp.socket", err.to_string()))
        }
        Ok(_) => Err(CliError::new(
            "mcp.socket",
            format!("refusing to replace non-socket path {}", path.display()),
        )),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(CliError::new("mcp.socket", err.to_string())),
    }
}

struct SocketGuard(PathBuf);

impl Drop for SocketGuard {
    fn drop(&mut self) {
        if std::fs::symlink_metadata(&self.0).is_ok_and(|metadata| metadata.file_type().is_socket())
        {
            let _ = std::fs::remove_file(&self.0);
        }
    }
}

struct BoundedLines<R> {
    inner: R,
    line_bytes: usize,
    max_line_bytes: usize,
    rejected: bool,
}

impl<R> BoundedLines<R> {
    fn new(inner: R) -> Self {
        Self::with_limit(inner, MAX_MCP_FRAME_BYTES)
    }

    fn with_limit(inner: R, max_line_bytes: usize) -> Self {
        Self {
            inner,
            line_bytes: 0,
            max_line_bytes,
            rejected: false,
        }
    }
}

impl<R: AsyncRead + Unpin> AsyncRead for BoundedLines<R> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<Result<(), io::Error>> {
        if self.rejected {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "MCP frame exceeds size limit",
            )));
        }
        if buf.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }
        let mut scratch = [0u8; 8 * 1_024];
        let capacity = buf.remaining().min(scratch.len());
        let mut read_buf = ReadBuf::new(&mut scratch[..capacity]);
        match Pin::new(&mut self.inner).poll_read(cx, &mut read_buf) {
            Poll::Ready(Ok(())) => {
                let bytes = read_buf.filled();
                let mut accepted = bytes.len();
                for (index, byte) in bytes.iter().enumerate() {
                    if *byte == b'\n' {
                        self.line_bytes = 0;
                    } else if self.line_bytes == self.max_line_bytes {
                        self.rejected = true;
                        accepted = index;
                        break;
                    } else {
                        self.line_bytes = self.line_bytes.saturating_add(1);
                    }
                }
                buf.put_slice(&bytes[..accepted]);
                if accepted == 0 && self.rejected {
                    return Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "MCP frame exceeds size limit",
                    )));
                }
                Poll::Ready(Ok(()))
            }
            other => other,
        }
    }
}

/// Session context for `omacell mcp`: proposal notify + WP-22 summary card.
pub fn ctx_for_cli(open_path: Option<String>, reload: ReloadHandle) -> McpCtx {
    let notify_config = reload.snapshot().config;
    McpCtx {
        open_path,
        gui_running: false,
        on_external_propose: Some(proposal_notifier(notify_config)),
        card: Some(Box::new(move |wb, path| {
            summary_card(&reload.snapshot().config, wb, path)
        })),
    }
}

fn summary_card(config: &Config, wb: &Workbook, path: Option<&str>) -> Value {
    let (provider, _) = route_slot(config, Slot::Default);
    let local = provider_is_local(config, &provider);
    let policy = PolicySnapshot::capture(config, Some(wb), local);
    let request = CardRequest {
        level: CardLevel::Summary,
        file: path.map(str::to_string),
        ..CardRequest::default()
    };
    match build_card(wb, None, request, &policy) {
        Ok((card, _)) => card,
        Err(_) => stub_card(wb, path),
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

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use tokio::io::AsyncReadExt;

    use super::BoundedLines;

    #[tokio::test]
    async fn bounded_lines_accepts_separate_lines_within_the_limit() {
        let mut input = BoundedLines::with_limit(&b"12345678\nabcdefgh\n"[..], 8);
        let mut output = Vec::new();
        input.read_to_end(&mut output).await.unwrap();
        assert_eq!(output, b"12345678\nabcdefgh\n");
    }

    #[tokio::test]
    async fn bounded_lines_rejects_an_oversized_frame() {
        let mut input = BoundedLines::with_limit(&b"123456789\n"[..], 8);
        let mut output = Vec::new();
        let error = input.read_to_end(&mut output).await.unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert_eq!(output, b"12345678");
    }

    #[test]
    fn explicit_socket_parent_must_be_private() {
        let temp = tempfile::tempdir().unwrap();
        let public = temp.path().join("public");
        std::fs::create_dir(&public).unwrap();
        std::fs::set_permissions(&public, std::fs::Permissions::from_mode(0o755)).unwrap();

        let error = super::prepare_socket_parent(&public.join("omacell.sock")).unwrap_err();
        assert_eq!(error.code, "mcp.socket");
        assert!(error.message.contains("700"), "{}", error.message);
    }
}
