//! What the window remembers between commands. Each page's state sits behind its own lock.

use std::sync::Mutex;
use std::sync::atomic::AtomicBool;

use toolbox_core::toolbox::ToolboxSettings;

use crate::cleanup::Cln;
use crate::updates::Upd;

pub struct AppState {
    pub settings: Mutex<ToolboxSettings>,
    pub upd: Mutex<Upd>,
    pub cln: Mutex<Cln>,
    /// Why closing now would lose work (an update run, a clean, a repair), else None.
    pub busy: Mutex<Option<String>>,
    pub force_close: AtomicBool,
    pub start_page: Mutex<Option<String>>,
    pub repair_running: AtomicBool,
}

impl AppState {
    pub fn new(start_page: Option<String>) -> Self {
        AppState {
            settings: Mutex::new(ToolboxSettings::load()),
            upd: Mutex::new(Upd::default()),
            cln: Mutex::new(Cln::default()),
            busy: Mutex::new(None),
            force_close: AtomicBool::new(false),
            start_page: Mutex::new(start_page),
            repair_running: AtomicBool::new(false),
        }
    }

    pub fn set_busy(&self, reason: Option<&str>) {
        *self.busy.lock().unwrap() = reason.map(str::to_string);
    }
}
