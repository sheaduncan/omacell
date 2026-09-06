use std::sync::{Arc, Mutex};

use omacell_ai::http::{HttpRequest, HttpResponse, SharedTransport, Transport};
use omacell_ai::{AiRuntime, PromptSet, register_ai_functions};
use omacell_bus::Bus;
use omacell_conf::schema::package_defaults;
use omacell_core::eval::FnRegistry;
use omacell_core::recalc::RecalcEngine;
use omacell_core::workbook::Workbook;
use omacell_fn::register_all;
use serde_json::json;

pub(super) struct CaptureTransport {
    content: String,
    pub(super) requests: Mutex<Vec<HttpRequest>>,
}

#[async_trait::async_trait]
impl Transport for CaptureTransport {
    async fn send(&self, req: HttpRequest) -> Result<HttpResponse, omacell_ai::AiError> {
        self.requests.lock().unwrap().push(req);
        Ok(HttpResponse {
            status: 200,
            body: json!({"choices": [{"message": {"content": self.content}}]}),
            chunks: Vec::new(),
        })
    }
}

pub(super) struct AiTestBus {
    pub(super) bus: Bus,
    pub(super) runtime: Arc<AiRuntime>,
    pub(super) transport: Arc<CaptureTransport>,
    _tokio: tokio::runtime::Runtime,
    _temp: tempfile::TempDir,
}

pub(super) fn ai_test_bus(
    content: &str,
    configure: impl FnOnce(&mut omacell_conf::schema::Config),
) -> AiTestBus {
    let mut config = package_defaults().unwrap();
    config.ai.enabled = true;
    config.ai.providers.insert(
        "test".into(),
        omacell_conf::schema::AiProvider {
            kind: "openai_compatible".into(),
            endpoint: "http://127.0.0.1:9/v1".into(),
            local: true,
            secret_env: None,
            secret_cmd: None,
            timeout: 0,
            headers: Default::default(),
        },
    );
    config.ai.models.default = "test:model".into();
    configure(&mut config);
    let tokio = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let temp = tempfile::TempDir::new().unwrap();
    let transport = Arc::new(CaptureTransport {
        content: content.into(),
        requests: Mutex::new(Vec::new()),
    });
    let shared: SharedTransport = transport.clone();
    let runtime = AiRuntime::new(
        tokio.handle().clone(),
        config,
        shared,
        PromptSet::builtin(),
        temp.path().join("cache"),
        temp.path().join("state"),
        Default::default(),
    );
    runtime.set_catalog(vec![(
        "cell.set".into(),
        json!({"id":"cell.set","doc":"Set a cell","args":{"type":"object"}}),
    )]);
    let mut registry = FnRegistry::new();
    register_all(&mut registry);
    register_ai_functions(&mut registry);
    let mut engine = RecalcEngine::new(registry);
    engine.set_async_provider(runtime.clone());
    let mut bus = Bus::new(Workbook::new(), engine).unwrap();
    super::register_ai_commands(
        &mut bus,
        super::AiSession {
            runtime: runtime.clone(),
        },
    )
    .unwrap();
    AiTestBus {
        bus,
        runtime,
        transport,
        _tokio: tokio,
        _temp: temp,
    }
}
