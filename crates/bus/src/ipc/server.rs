//! Blocking Unix-socket IPC server.

use std::io::{ErrorKind, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use omacell_core::command::Origin;
use omacell_core::error::CoreError;

use super::discover::{
    instance_path, prepare_runtime_dir, remove_stale_socket, socket_path, write_discovery,
};
use super::dispatch::{Dispatch, dispatch_bus_request, dispatch_runner_request};
use super::protocol::{
    FrameBuf, MAX_CONNECTIONS, MAX_EVENT_QUEUE, MAX_EVENT_QUEUE_BYTES, Reply, Request,
    ServerRecord, encode_line, event_type_name,
};
use crate::error;
use crate::event::SubscriberId;
use crate::runner::TaskRunnerHandle;
use crate::session::Bus;

const READ_TICK: Duration = Duration::from_millis(100);
const WRITE_TIMEOUT: Duration = Duration::from_secs(2);

/// Running server handle. Dropping it shuts the accept loop down.
pub struct IpcHandle {
    dir: PathBuf,
    pid: u32,
    path: PathBuf,
    shutdown: Arc<AtomicBool>,
    accept: Option<JoinHandle<()>>,
    clients: Arc<Mutex<Vec<JoinHandle<()>>>>,
}

impl IpcHandle {
    /// Bound socket path.
    #[must_use]
    pub fn socket_path(&self) -> &Path {
        &self.path
    }

    /// Runtime directory (`…/omacell`).
    #[must_use]
    pub fn runtime_dir(&self) -> &Path {
        &self.dir
    }

    /// Signal shutdown and join the accept thread.
    pub fn shutdown(mut self) {
        self.stop();
    }

    fn stop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        // Unblock accept with a dummy connect.
        let _ = UnixStream::connect(&self.path);
        if let Some(handle) = self.accept.take() {
            let _ = handle.join();
        }
        let clients = {
            let mut clients = lock_clients(&self.clients);
            clients.drain(..).collect::<Vec<_>>()
        };
        for client in clients {
            let _ = client.join();
        }
        let _ = std::fs::remove_file(&self.path);
        let _ = std::fs::remove_file(instance_path(&self.dir, self.pid));
    }
}

impl Drop for IpcHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Bind `{dir}/{pid}.sock` and serve `bus` on std threads.
pub fn serve(dir: PathBuf, bus: Bus) -> Result<IpcHandle, CoreError> {
    serve_shared(dir, Arc::new(Mutex::new(bus)))
}

/// Bind `{dir}/{pid}.sock` and serve an existing shared bus on std threads.
///
/// Callers that also host MCP or need direct inspection retain their own
/// [`Arc`] instead of reaching back through the server handle.
pub fn serve_shared(dir: PathBuf, bus: Arc<Mutex<Bus>>) -> Result<IpcHandle, CoreError> {
    prepare_runtime_dir(&dir)?;
    let pid = std::process::id();
    remove_stale_socket(&dir, pid)?;
    let path = socket_path(&dir, pid);
    if path
        .symlink_metadata()
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
    {
        return Err(error::ipc_socket(format!(
            "{} is a symlink",
            path.display()
        )));
    }
    let listener = UnixListener::bind(&path)
        .map_err(|err| error::ipc_socket(format!("bind {}: {err}", path.display())))?;
    let mut cleanup = BoundCleanup::new(path.clone());
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
        .map_err(|err| error::ipc_socket(format!("chmod {}: {err}", path.display())))?;
    listener
        .set_nonblocking(true)
        .map_err(|err| error::ipc_socket(format!("set nonblocking: {err}")))?;
    write_discovery(&dir, pid)?;
    cleanup.instance = Some(instance_path(&dir, pid));

    let shutdown = Arc::new(AtomicBool::new(false));
    let connections = Arc::new(AtomicUsize::new(0));
    let clients = Arc::new(Mutex::new(Vec::new()));
    let accept_shutdown = shutdown.clone();
    let accept_bus = bus.clone();
    let accept_clients = clients.clone();
    let accept = thread::Builder::new()
        .name("omacell-ipc-accept".into())
        .spawn(move || {
            accept_loop(
                listener,
                accept_bus,
                accept_shutdown,
                connections,
                accept_clients,
            );
        })
        .map_err(|err| error::ipc_socket(format!("spawn accept: {err}")))?;
    cleanup.disarm();
    Ok(IpcHandle {
        dir,
        pid,
        path,
        shutdown,
        accept: Some(accept),
        clients,
    })
}

/// Serve IPC by submitting to the single-writer task runner.
pub fn serve_runner(dir: PathBuf, runner: TaskRunnerHandle) -> Result<IpcHandle, CoreError> {
    prepare_runtime_dir(&dir)?;
    let pid = std::process::id();
    remove_stale_socket(&dir, pid)?;
    let path = socket_path(&dir, pid);
    if path
        .symlink_metadata()
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
    {
        return Err(error::ipc_socket(format!(
            "{} is a symlink",
            path.display()
        )));
    }
    let listener = UnixListener::bind(&path)
        .map_err(|err| error::ipc_socket(format!("bind {}: {err}", path.display())))?;
    let mut cleanup = BoundCleanup::new(path.clone());
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
        .map_err(|err| error::ipc_socket(format!("chmod {}: {err}", path.display())))?;
    listener
        .set_nonblocking(true)
        .map_err(|err| error::ipc_socket(format!("set nonblocking: {err}")))?;
    write_discovery(&dir, pid)?;
    cleanup.instance = Some(instance_path(&dir, pid));

    let shutdown = Arc::new(AtomicBool::new(false));
    let connections = Arc::new(AtomicUsize::new(0));
    let clients = Arc::new(Mutex::new(Vec::new()));
    let accept_shutdown = shutdown.clone();
    let accept_clients = clients.clone();
    let accept_runner = runner.clone();
    let accept = thread::Builder::new()
        .name("omacell-ipc-accept".into())
        .spawn(move || {
            accept_loop_runner(
                listener,
                accept_runner,
                accept_shutdown,
                connections,
                accept_clients,
            );
        })
        .map_err(|err| error::ipc_socket(format!("spawn accept: {err}")))?;
    cleanup.disarm();
    Ok(IpcHandle {
        dir,
        pid,
        path,
        shutdown,
        accept: Some(accept),
        clients,
    })
}

fn accept_loop(
    listener: UnixListener,
    bus: Arc<Mutex<Bus>>,
    shutdown: Arc<AtomicBool>,
    connections: Arc<AtomicUsize>,
    clients: Arc<Mutex<Vec<JoinHandle<()>>>>,
) {
    while !shutdown.load(Ordering::SeqCst) {
        reap_finished_clients(&clients);
        match listener.accept() {
            Ok((stream, _)) => {
                if shutdown.load(Ordering::SeqCst) {
                    break;
                }
                let current = connections.fetch_add(1, Ordering::SeqCst);
                if current >= MAX_CONNECTIONS {
                    connections.fetch_sub(1, Ordering::SeqCst);
                    let _ = write_error_and_close(
                        stream,
                        error::ipc_limit(format!("at most {MAX_CONNECTIONS} IPC clients")),
                    );
                    continue;
                }
                let bus = bus.clone();
                let client_connections = connections.clone();
                let client_shutdown = shutdown.clone();
                match thread::Builder::new()
                    .name("omacell-ipc-client".into())
                    .spawn(move || {
                        client_loop(stream, bus, client_shutdown);
                        client_connections.fetch_sub(1, Ordering::SeqCst);
                    }) {
                    Ok(handle) => lock_clients(&clients).push(handle),
                    Err(_) => {
                        connections.fetch_sub(1, Ordering::SeqCst);
                    }
                }
            }
            Err(err) if err.kind() == ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(20));
            }
            Err(_) => thread::sleep(Duration::from_millis(20)),
        }
    }
    reap_finished_clients(&clients);
}

fn accept_loop_runner(
    listener: UnixListener,
    runner: TaskRunnerHandle,
    shutdown: Arc<AtomicBool>,
    connections: Arc<AtomicUsize>,
    clients: Arc<Mutex<Vec<JoinHandle<()>>>>,
) {
    while !shutdown.load(Ordering::SeqCst) {
        reap_finished_clients(&clients);
        match listener.accept() {
            Ok((stream, _)) => {
                if shutdown.load(Ordering::SeqCst) {
                    break;
                }
                let current = connections.fetch_add(1, Ordering::SeqCst);
                if current >= MAX_CONNECTIONS {
                    connections.fetch_sub(1, Ordering::SeqCst);
                    let _ = write_error_and_close(
                        stream,
                        error::ipc_limit(format!("at most {MAX_CONNECTIONS} IPC clients")),
                    );
                    continue;
                }
                let runner = runner.clone();
                let client_connections = connections.clone();
                let client_shutdown = shutdown.clone();
                match thread::Builder::new()
                    .name("omacell-ipc-client".into())
                    .spawn(move || {
                        client_loop_runner(stream, runner, client_shutdown);
                        client_connections.fetch_sub(1, Ordering::SeqCst);
                    }) {
                    Ok(handle) => lock_clients(&clients).push(handle),
                    Err(_) => {
                        connections.fetch_sub(1, Ordering::SeqCst);
                    }
                }
            }
            Err(err) if err.kind() == ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(20));
            }
            Err(_) => thread::sleep(Duration::from_millis(20)),
        }
    }
    reap_finished_clients(&clients);
}

fn client_loop_runner(stream: UnixStream, runner: TaskRunnerHandle, shutdown: Arc<AtomicBool>) {
    let _ = stream.set_read_timeout(Some(READ_TICK));
    let _ = stream.set_write_timeout(Some(WRITE_TIMEOUT));
    let mut writer = match stream.try_clone() {
        Ok(w) => w,
        Err(_) => return,
    };
    let mut reader = stream;
    let mut frames = FrameBuf::new();
    let mut chunk = [0u8; 8192];
    while !shutdown.load(Ordering::SeqCst) {
        match reader.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => match frames.push(&chunk[..n]) {
                Ok(lines) => {
                    for line in lines {
                        let request = match super::protocol::decode_request_bytes(&line) {
                            Ok(r) => r,
                            Err(err) => {
                                let _ = write_reply(&mut writer, Reply::err(0, err));
                                continue;
                            }
                        };
                        let reply = dispatch_runner(&runner, request);
                        if write_reply(&mut writer, reply).is_err() {
                            return;
                        }
                    }
                }
                Err(err) => {
                    let _ = write_reply(&mut writer, Reply::err(0, err));
                    return;
                }
            },
            Err(err)
                if err.kind() == ErrorKind::WouldBlock || err.kind() == ErrorKind::TimedOut => {}
            Err(err) if err.kind() == ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
}

fn dispatch_runner(runner: &TaskRunnerHandle, request: Request) -> Reply {
    dispatch_runner_request(runner, Origin::Ipc, request)
        .reject_subscriptions("event subscriptions are not available on the UI task runner")
}

fn write_error_and_close(mut stream: UnixStream, err: CoreError) -> Result<(), CoreError> {
    let reply = Reply::err(0, err);
    let line = encode_line(&reply)?;
    let _ = stream.write_all(line.as_bytes());
    Ok(())
}

fn client_loop(stream: UnixStream, bus: Arc<Mutex<Bus>>, shutdown: Arc<AtomicBool>) {
    let _ = stream.set_read_timeout(Some(READ_TICK));
    let _ = stream.set_write_timeout(Some(WRITE_TIMEOUT));
    let mut writer = match stream.try_clone() {
        Ok(w) => w,
        Err(_) => return,
    };
    let mut reader = stream;
    let mut frames = FrameBuf::new();
    let mut chunk = [0u8; 8192];
    let mut sub: Option<SubscriberId> = None;
    let mut filter: Vec<String> = Vec::new();
    'client: while !shutdown.load(Ordering::SeqCst) {
        match reader.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => match frames.push(&chunk[..n]) {
                Ok(lines) => {
                    for line in lines {
                        if !handle_line(&mut writer, &bus, &mut sub, &mut filter, &line) {
                            break 'client;
                        }
                    }
                }
                Err(err) => {
                    let _ = write_reply(&mut writer, Reply::err(0, err));
                    break 'client;
                }
            },
            Err(err)
                if err.kind() == ErrorKind::WouldBlock || err.kind() == ErrorKind::TimedOut =>
            {
                if !flush_events(&mut writer, &bus, &mut sub, &filter) {
                    break 'client;
                }
            }
            Err(err) if err.kind() == ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
    if let Some(id) = sub.take() {
        lock_bus(&bus).unsubscribe(id);
    }
}

fn handle_line(
    writer: &mut UnixStream,
    bus: &Arc<Mutex<Bus>>,
    sub: &mut Option<SubscriberId>,
    filter: &mut Vec<String>,
    line: &[u8],
) -> bool {
    let request = match super::protocol::decode_request_bytes(line) {
        Ok(r) => r,
        Err(err) => {
            let _ = write_reply(writer, Reply::err(0, err));
            return true;
        }
    };
    let reply = dispatch(bus, sub, filter, request);
    if write_reply(writer, reply).is_err() {
        return false;
    }
    flush_events(writer, bus, sub, filter)
}

fn dispatch(
    bus: &Arc<Mutex<Bus>>,
    sub: &mut Option<SubscriberId>,
    filter: &mut Vec<String>,
    request: Request,
) -> Reply {
    let mut bus = lock_bus(bus);
    match dispatch_bus_request(&mut bus, Origin::Ipc, request) {
        Dispatch::Reply(reply) => reply,
        Dispatch::Subscribe { id, events } => {
            if let Some(old) = sub.take() {
                bus.unsubscribe(old);
            }
            *filter = events;
            *sub = Some(bus.subscribe_ipc(MAX_EVENT_QUEUE, MAX_EVENT_QUEUE_BYTES, filter));
            Reply::ok(id, serde_json::json!({ "subscribed": filter.clone() }))
        }
        Dispatch::Unsubscribe { id } => {
            if let Some(old) = sub.take() {
                bus.unsubscribe(old);
            }
            filter.clear();
            Reply::ok(id, serde_json::json!({ "unsubscribed": true }))
        }
    }
}

fn flush_events(
    writer: &mut UnixStream,
    bus: &Arc<Mutex<Bus>>,
    sub: &mut Option<SubscriberId>,
    filter: &[String],
) -> bool {
    let Some(id) = *sub else {
        return true;
    };
    let (dropped, events) = {
        let mut bus = lock_bus(bus);
        let dropped = bus.dropped(id);
        let events = bus.drain(id);
        (dropped, events)
    };
    if dropped > 0 {
        let _ = write_record(writer, &ServerRecord::overflow(dropped));
        if let Some(id) = sub.take() {
            lock_bus(bus).unsubscribe(id);
        }
        return false;
    }
    let mut queued_bytes = 0usize;
    for event in events {
        if !filter.is_empty() && !filter.iter().any(|n| n == event_type_name(&event)) {
            continue;
        }
        let record = ServerRecord::event(event);
        match encode_line(&record) {
            Ok(line) => {
                queued_bytes = queued_bytes.saturating_add(line.len());
                if queued_bytes > super::protocol::MAX_EVENT_QUEUE_BYTES {
                    let _ = write_record(writer, &ServerRecord::overflow(1));
                    if let Some(id) = sub.take() {
                        lock_bus(bus).unsubscribe(id);
                    }
                    return false;
                }
                if writer.write_all(line.as_bytes()).is_err() {
                    return false;
                }
            }
            Err(_) => return false,
        }
    }
    true
}

fn write_reply(writer: &mut UnixStream, reply: Reply) -> Result<(), CoreError> {
    let line = encode_line(&reply)?;
    writer
        .write_all(line.as_bytes())
        .map_err(|_| error::ipc_disconnected())?;
    Ok(())
}

fn write_record(writer: &mut UnixStream, record: &ServerRecord) -> Result<(), CoreError> {
    let line = encode_line(record)?;
    writer
        .write_all(line.as_bytes())
        .map_err(|_| error::ipc_disconnected())?;
    Ok(())
}

fn lock_bus(bus: &Arc<Mutex<Bus>>) -> std::sync::MutexGuard<'_, Bus> {
    bus.lock().unwrap_or_else(|poison| poison.into_inner())
}

fn lock_clients(
    clients: &Arc<Mutex<Vec<JoinHandle<()>>>>,
) -> std::sync::MutexGuard<'_, Vec<JoinHandle<()>>> {
    clients.lock().unwrap_or_else(|poison| poison.into_inner())
}

fn reap_finished_clients(clients: &Arc<Mutex<Vec<JoinHandle<()>>>>) {
    let finished = {
        let mut clients = lock_clients(clients);
        let mut finished = Vec::new();
        let mut index = 0;
        while index < clients.len() {
            if clients[index].is_finished() {
                finished.push(clients.swap_remove(index));
            } else {
                index += 1;
            }
        }
        finished
    };
    for client in finished {
        let _ = client.join();
    }
}

struct BoundCleanup {
    socket: PathBuf,
    instance: Option<PathBuf>,
    armed: bool,
}

impl BoundCleanup {
    fn new(socket: PathBuf) -> Self {
        Self {
            socket,
            instance: None,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for BoundCleanup {
    fn drop(&mut self) {
        if self.armed {
            let _ = std::fs::remove_file(&self.socket);
            if let Some(instance) = &self.instance {
                let _ = std::fs::remove_file(instance);
            }
        }
    }
}
