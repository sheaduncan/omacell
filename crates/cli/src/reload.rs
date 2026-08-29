//! `theme.reload` command and SIGUSR1 adapter.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use omacell_bus::{Bus, CommandKind, CommandSpec, Effect, Exposure};
use omacell_conf::ReloadHandle;
use omacell_core::error::CoreError;
use omacell_core::event::Event;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use signal_hook::consts::SIGUSR1;
use signal_hook::flag;

/// Empty args for `theme.reload`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ThemeReloadArgs {}

/// Register `theme.reload` against a shared [`ReloadHandle`].
pub fn register_theme_reload(bus: &mut Bus, handle: ReloadHandle) -> Result<(), CoreError> {
    bus.registry_mut().register::<ThemeReloadArgs, _>(
        CommandSpec {
            id: "theme.reload",
            doc: "Reload configuration and the active Omarchy theme",
            kind: CommandKind::Mutating,
            changeset_eligible: false,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        move |_ctx, _args| {
            handle.reload()?;
            let loaded = handle.snapshot();
            let name = loaded.theme.name.clone();
            Ok(Effect {
                events: vec![Event::ThemeChanged { name: name.clone() }],
                result: serde_json::json!({"name": name}),
                auto_recalc: false,
                ..Effect::default()
            })
        },
    )
}

/// Guard that stops the SIGUSR1 poll thread on drop.
pub struct Sigusr1Guard {
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl Drop for Sigusr1Guard {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

/// Bind SIGUSR1 to [`ReloadHandle::reload`] without `unsafe` in this crate.
pub fn spawn_sigusr1_reloader(handle: ReloadHandle) -> Result<Sigusr1Guard, CoreError> {
    let flag = Arc::new(AtomicBool::new(false));
    flag::register(SIGUSR1, Arc::clone(&flag)).map_err(|err| {
        CoreError::new("cli.signal", format!("register SIGUSR1: {err}"))
            .with_hint("theme reload still works via omacell theme reload and ipc")
    })?;
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = Arc::clone(&stop);
    let join = thread::spawn(move || {
        while !thread_stop.load(Ordering::Relaxed) {
            if flag.swap(false, Ordering::SeqCst) {
                let _ = handle.reload();
            }
            thread::sleep(Duration::from_millis(20));
        }
    });
    Ok(Sigusr1Guard {
        stop,
        join: Some(join),
    })
}
