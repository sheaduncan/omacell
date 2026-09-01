//! Background chart rasterization and terminal-protocol encoding.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError, TrySendError, sync_channel};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use omacell_bus::ReaderSnapshot;
use omacell_core::chart::{Chart, ChartId, ChartTheme, Scene, to_svg};
use omacell_core::error::CoreError;
use ratatui::Frame;
use ratatui::layout::{Rect, Size};
use ratatui::widgets::Clear;
use ratatui_image::picker::cap_parser::QueryStdioOptions;
use ratatui_image::picker::{Picker, ProtocolType};
use ratatui_image::protocol::Protocol;
use ratatui_image::{Image, Resize};

use crate::theme::{graphics_protocol, graphics_query_allowed};

const WORK_QUEUE: usize = 8;
const READY_CACHE: usize = 8;
const FAILED_CACHE: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct ImageKey {
    chart: ChartId,
    width: u16,
    height: u16,
}

struct RenderRequest {
    generation: u64,
    key: ImageKey,
    snapshot: Arc<ReaderSnapshot>,
    chart: Chart,
    theme: ChartTheme,
}

struct RenderResponse {
    generation: u64,
    key: ImageKey,
    rendered: Result<RenderedChart, String>,
}

struct RenderedChart {
    scene: Scene,
    protocol: Option<Protocol>,
}

struct ReadyImage {
    key: ImageKey,
    rendered: RenderedChart,
}

/// Live sixel/Kitty cache. Expensive rasterization and encoding stay off the UI thread.
pub(crate) struct ChartGraphics {
    tx: Option<SyncSender<RenderRequest>>,
    rx: Receiver<RenderResponse>,
    worker: Option<JoinHandle<()>>,
    stop: Arc<AtomicBool>,
    generation: u64,
    snapshot: Option<Arc<ReaderSnapshot>>,
    theme: Option<ChartTheme>,
    pending: BTreeSet<ImageKey>,
    failed: BTreeSet<ImageKey>,
    ready: BTreeMap<ChartId, ReadyImage>,
    ready_order: VecDeque<ChartId>,
}

impl ChartGraphics {
    pub(crate) fn detect(setting: &str) -> Result<Self, CoreError> {
        if !graphics_query_allowed(setting) {
            return Self::spawn(None);
        }
        let options = QueryStdioOptions {
            timeout: Duration::from_millis(75),
            ..QueryStdioOptions::default()
        };
        let mut picker =
            Picker::from_query_stdio_with_options(options).unwrap_or_else(|_| Picker::halfblocks());
        if let Some(protocol) = graphics_protocol(setting) {
            picker.set_protocol_type(match protocol {
                "sixel" => ProtocolType::Sixel,
                "kitty" => ProtocolType::Kitty,
                _ => ProtocolType::Halfblocks,
            });
        }
        let picker = matches!(
            picker.protocol_type(),
            ProtocolType::Sixel | ProtocolType::Kitty
        )
        .then_some(picker);
        Self::spawn(picker)
    }

    pub(crate) fn spawn(picker: Option<Picker>) -> Result<Self, CoreError> {
        let (tx, work) = sync_channel::<RenderRequest>(WORK_QUEUE);
        let (completed, rx) = std::sync::mpsc::channel::<RenderResponse>();
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let worker = thread::Builder::new()
            .name("omacell-tui-chart".into())
            .spawn(move || render_worker(picker, work, completed, &worker_stop))
            .map_err(|error| CoreError::new("tui.graphics", format!("spawn renderer: {error}")))?;
        Ok(Self {
            tx: Some(tx),
            rx,
            worker: Some(worker),
            stop,
            generation: 0,
            snapshot: None,
            theme: None,
            pending: BTreeSet::new(),
            failed: BTreeSet::new(),
            ready: BTreeMap::new(),
            ready_order: VecDeque::new(),
        })
    }

    #[cfg(test)]
    fn protocol_type(&self) -> Option<ProtocolType> {
        self.ready
            .values()
            .next()
            .and_then(|ready| ready.rendered.protocol.as_ref())
            .map(|protocol| match protocol {
                Protocol::Sixel(_) => ProtocolType::Sixel,
                Protocol::Kitty(_) => ProtocolType::Kitty,
                Protocol::Halfblocks(_) => ProtocolType::Halfblocks,
                Protocol::ITerm2(_) => ProtocolType::Iterm2,
            })
    }

    pub(crate) fn refresh(&mut self, snapshot: &Arc<ReaderSnapshot>, theme: &ChartTheme) {
        let snapshot_changed = self
            .snapshot
            .as_ref()
            .is_none_or(|current| !Arc::ptr_eq(current, snapshot));
        if snapshot_changed || self.theme.as_ref() != Some(theme) {
            self.generation = self.generation.wrapping_add(1);
            self.snapshot = Some(snapshot.clone());
            self.theme = Some(theme.clone());
            self.pending.clear();
            self.failed.clear();
            self.ready.clear();
            self.ready_order.clear();
        }
        loop {
            match self.rx.try_recv() {
                Ok(response) if response.generation == self.generation => {
                    self.pending.remove(&response.key);
                    match response.rendered {
                        Ok(rendered) => {
                            if !self.ready.contains_key(&response.key.chart) {
                                while self.ready.len() >= READY_CACHE {
                                    if let Some(evicted) = self.ready_order.pop_front() {
                                        self.ready.remove(&evicted);
                                    } else {
                                        break;
                                    }
                                }
                                self.ready_order.push_back(response.key.chart);
                            }
                            self.ready.insert(
                                response.key.chart,
                                ReadyImage {
                                    key: response.key,
                                    rendered,
                                },
                            );
                        }
                        Err(_) => {
                            while self.failed.len() >= FAILED_CACHE {
                                if let Some(first) = self.failed.first().copied() {
                                    self.failed.remove(&first);
                                } else {
                                    break;
                                }
                            }
                            self.failed.insert(response.key);
                        }
                    }
                }
                Ok(_) => {}
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            }
        }
    }

    pub(crate) fn request(
        &mut self,
        snapshot: Arc<ReaderSnapshot>,
        chart: &Chart,
        theme: &ChartTheme,
        area: Rect,
    ) {
        if area.width < 4 || area.height < 3 {
            return;
        }
        let key = ImageKey {
            chart: chart.id,
            width: area.width,
            height: area.height,
        };
        if self.pending.contains(&key)
            || self.failed.contains(&key)
            || self
                .ready
                .get(&chart.id)
                .is_some_and(|ready| ready.key == key)
        {
            return;
        }
        let Some(tx) = self.tx.as_ref() else {
            return;
        };
        let request = RenderRequest {
            generation: self.generation,
            key,
            snapshot,
            chart: chart.clone(),
            theme: theme.clone(),
        };
        match tx.try_send(request) {
            Ok(()) => {
                self.pending.insert(key);
            }
            Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => {}
        }
    }

    pub(crate) fn render_protocol(
        &self,
        frame: &mut Frame<'_>,
        chart: ChartId,
        area: Rect,
    ) -> bool {
        let Some(ready) = self.ready.get(&chart) else {
            return false;
        };
        if ready.key.width != area.width || ready.key.height != area.height {
            return false;
        }
        let Some(protocol) = ready.rendered.protocol.as_ref() else {
            return false;
        };
        frame.render_widget(Clear, area);
        frame.render_widget(Image::new(protocol).allow_clipping(true), area);
        true
    }

    pub(crate) fn scene(&self, chart: ChartId, area: Rect) -> Option<&Scene> {
        let ready = self.ready.get(&chart)?;
        (ready.key.width == area.width && ready.key.height == area.height)
            .then_some(&ready.rendered.scene)
    }

    pub(crate) fn failed(&self, chart: ChartId, area: Rect) -> bool {
        self.failed.contains(&ImageKey {
            chart,
            width: area.width,
            height: area.height,
        })
    }
}

impl Drop for ChartGraphics {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.tx.take();
        drop(self.worker.take());
    }
}

fn render_worker(
    picker: Option<Picker>,
    requests: Receiver<RenderRequest>,
    completed: std::sync::mpsc::Sender<RenderResponse>,
    stop: &AtomicBool,
) {
    let font = picker
        .as_ref()
        .map_or_else(|| Picker::halfblocks().font_size(), Picker::font_size);
    while let Ok(request) = requests.recv() {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let width = u32::from(request.key.width)
            .saturating_mul(u32::from(font.width))
            .max(1);
        let height = u32::from(request.key.height)
            .saturating_mul(u32::from(font.height))
            .max(1);
        let rendered = omacell_io::chart_export::chart_scene(
            &request.snapshot.workbook,
            &request.chart,
            &request.theme,
            width as f32,
            height as f32,
        )
        .map_err(|error| error.to_string())
        .map(|scene| {
            let protocol = picker.as_ref().and_then(|picker| {
                let svg = to_svg(&scene);
                omacell_io::chart_export::rasterize_svg(&svg, width, height)
                    .map_err(|error| error.to_string())
                    .and_then(|bytes| {
                        image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)
                            .map_err(|error| error.to_string())
                    })
                    .and_then(|image| {
                        picker
                            .new_protocol(
                                image,
                                Size::new(request.key.width, request.key.height),
                                Resize::Fit(None),
                            )
                            .map_err(|error| error.to_string())
                    })
                    .ok()
            });
            RenderedChart { scene, protocol }
        });
        if completed
            .send(RenderResponse {
                generation: request.generation,
                key: request.key,
                rendered,
            })
            .is_err()
        {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use omacell_core::addr::{CellRef, RangeRef};
    use omacell_core::chart::{Axis, ChartAnchor, ChartKind, LegendPos, Series};
    use omacell_core::spill::SpillTable;
    use omacell_core::workbook::Workbook;

    use super::*;

    fn fixture() -> (Arc<ReaderSnapshot>, Chart) {
        let mut workbook = Workbook::new();
        let sheet = workbook.active_sheet();
        for (row, value) in [1.0, 4.0, 2.0].into_iter().enumerate() {
            workbook.set_number(sheet, row as u32, 0, value).unwrap();
        }
        let chart = Chart {
            id: ChartId::new(1),
            kind: ChartKind::Line,
            title: Some("Worker".into()),
            categories: None,
            series: vec![Series {
                name: "Series".into(),
                values: RangeRef::from_corners(
                    CellRef::new(0, 0).unwrap(),
                    CellRef::new(2, 0).unwrap(),
                ),
                x: None,
                size: None,
                color: None,
                secondary_axis: false,
                trendline: None,
            }],
            category_axis: Axis::default(),
            value_axis: Axis::default(),
            secondary_axis: None,
            legend: LegendPos::None,
            data_labels: false,
            anchor: ChartAnchor::default(),
            sheet,
        };
        let snapshot = Arc::new(ReaderSnapshot {
            workbook,
            spill: SpillTable::new(),
        });
        (snapshot, chart)
    }

    #[test]
    fn explicit_protocols_rasterize_and_encode_off_thread() {
        for expected in [ProtocolType::Sixel, ProtocolType::Kitty] {
            let mut picker = Picker::halfblocks();
            picker.set_protocol_type(expected);
            let mut graphics = ChartGraphics::spawn(Some(picker)).unwrap();
            let (snapshot, chart) = fixture();
            let theme = ChartTheme::neutral();
            graphics.refresh(&snapshot, &theme);
            graphics.request(snapshot.clone(), &chart, &theme, Rect::new(0, 0, 20, 10));
            let started = Instant::now();
            while graphics.protocol_type().is_none() {
                graphics.refresh(&snapshot, &theme);
                assert!(started.elapsed() < Duration::from_secs(5));
                std::thread::yield_now();
            }
            assert_eq!(graphics.protocol_type(), Some(expected));
        }
    }

    #[test]
    fn unicode_scene_is_prepared_off_thread_without_a_binary_protocol() {
        let mut graphics = ChartGraphics::spawn(None).unwrap();
        let (snapshot, chart) = fixture();
        let theme = ChartTheme::neutral();
        let area = Rect::new(0, 0, 20, 10);
        graphics.refresh(&snapshot, &theme);
        graphics.request(snapshot.clone(), &chart, &theme, area);
        let started = Instant::now();
        while graphics.scene(chart.id, area).is_none() {
            graphics.refresh(&snapshot, &theme);
            assert!(started.elapsed() < Duration::from_secs(5));
            std::thread::yield_now();
        }
        assert_eq!(graphics.protocol_type(), None);
    }
}
