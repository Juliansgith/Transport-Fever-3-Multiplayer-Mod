//! What the window drives: the launcher running in this process, or a
//! stand-in in tests.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use eframe::egui;
use tpf3mp_agent::launcher::{Action, LauncherHandle, State};

/// A launcher the window shows and sends actions to.
pub trait Backend {
    /// What to show now.
    fn state(&self) -> State;
    /// Asks for `action`. Its outcome shows in the state: a refusal in
    /// [`State::error`].
    fn act(&self, action: Action);
    /// Whether an action is still being carried out.
    fn busy(&self) -> bool;
}

/// The launcher running in this process, on a Tokio runtime.
pub struct Local {
    handle: LauncherHandle,
    runtime: tokio::runtime::Handle,
    pending: Arc<AtomicUsize>,
    repaint: Option<egui::Context>,
}

impl Local {
    pub fn new(handle: LauncherHandle, runtime: tokio::runtime::Handle) -> Self {
        Self {
            handle,
            runtime,
            pending: Arc::default(),
            repaint: None,
        }
    }

    /// Repaints the window as soon as an action finishes.
    pub fn repaint_with(&mut self, ctx: egui::Context) {
        self.repaint = Some(ctx);
    }
}

impl Backend for Local {
    fn state(&self) -> State {
        self.handle.state()
    }

    fn act(&self, action: Action) {
        let handle = self.handle.clone();
        let pending = Arc::clone(&self.pending);
        let repaint = self.repaint.clone();
        pending.fetch_add(1, Ordering::SeqCst);
        self.runtime.spawn(async move {
            // A refusal is kept in the state's `error`, which the window shows.
            let _ = handle.act(action).await;
            pending.fetch_sub(1, Ordering::SeqCst);
            if let Some(ctx) = repaint {
                ctx.request_repaint();
            }
        });
    }

    fn busy(&self) -> bool {
        self.pending.load(Ordering::SeqCst) > 0
    }
}
