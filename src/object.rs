use dusa_collection_utils::core::types::pathtype::PathType;
use dusa_collection_utils::{
    core::errors::{ErrorArray, ErrorArrayItem, Errors},
    core::logger::LogLevel,
    core::types::{controls::ToggleControl, rwarc::LockWithTimeout},
    log,
};
use notify::Event;
use notify::RecursiveMode;
use notify::event::{AccessKind, AccessMode, CreateKind, MetadataKind, ModifyKind, RemoveKind};
use notify::{Config, EventKind, RecommendedWatcher, Watcher};
use std::sync::atomic::Ordering;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64},
};
use std::time::Duration;
use tokio::sync::broadcast::{self, Receiver};
use tokio::sync::{Mutex, RwLock};
use tokio::task::JoinHandle;
use tokio::time::sleep;

use crate::options::Options;

pub type FileMonitor = Arc<RawFileMonitor>;

#[derive(Clone, Debug)]
pub enum MonitorMode {
    ALL,
    Modify,
    Access,
}

#[derive(Debug)]
pub struct RawFileMonitor {
    sender: RwLock<Option<Arc<broadcast::Sender<Event>>>>, // needs to be in a rwlock or mutex
    handle: RwLock<Option<JoinHandle<()>>>,                // needs to be in a mutex or rwlock
    events: AtomicU64,
    controls: Arc<ToggleControl>,
    watcher: RwLock<Option<LockWithTimeout<RecommendedWatcher>>>,
    poisoned: AtomicBool,
    errors: ErrorArray,
    dir: PathType,
    ignored: Vec<PathType>,
    timeout: AtomicU64,
    recursive: RecursiveMode,
    monitor: MonitorMode,
    missing: AtomicBool,
}

impl RawFileMonitor {
    pub async fn new(options: Options) -> Self {
        let mut error_array = ErrorArray::new_container();
        let mut poisoned: bool = false;
        let timeout: u64 = options.get_interval();
        let controls: Arc<ToggleControl> = Arc::new(ToggleControl::new());
        controls.pause(); // set initial state as stopped

        // validating and formatting paths
        if options.get_validation() {
            if !options.base_dir().exists() {
                let ea: ErrorArrayItem = ErrorArrayItem::new(
                    Errors::NotFound,
                    format!("{}: Not found", options.base_dir()),
                );
                error_array.push(ea);
                poisoned = true;
            }
        }

        let ignored: Vec<PathType> =
            Self::sanitize_ignored_dirs(options.base_dir(), options.ignored_dirs());

        Self {
            sender: RwLock::new(None),
            handle: RwLock::new(None),
            events: AtomicU64::from(0),
            controls,
            watcher: RwLock::new(None),
            poisoned: AtomicBool::from(poisoned),
            errors: error_array,
            dir: options.base_dir(),
            ignored,
            timeout: AtomicU64::from(timeout),
            recursive: options.get_mode(),
            missing: AtomicBool::new(false),
            monitor: options.get_monitor_mode(),
        }
    }

    pub async fn start(&self) {
        if let Some((watcher_tx, event_tx)) = self.initialize_watcher_channels().await {
            self.controls.resume();
            self.spawn_event_loop(watcher_tx, event_tx);
        } else {
            log!(
                LogLevel::Error,
                "Failed to start watcher due to initialization errors"
            );
        }
    }

    async fn initialize_watcher_channels(
        &self,
    ) -> Option<(
        Arc<Mutex<broadcast::Sender<Event>>>,
        Arc<broadcast::Sender<Event>>,
    )> {
        let (watcher_tx, _) = broadcast::channel::<Event>(2048);
        let (event_tx, _) = broadcast::channel(10240);

        let watcher_tx = Arc::new(Mutex::new(watcher_tx));
        let event_tx = Arc::new(event_tx);

        let handler_tx = watcher_tx.clone();
        let handler = move |res: notify::Result<Event>| {
            let handler_tx = handler_tx.clone();
            if let Ok(event) = res {
                let lock = handler_tx.blocking_lock();
                if let Err(err) = lock.send(event) {
                    log!(LogLevel::Error, "Failed to send from handler: {}", err);
                }
            }
        };

        let watcher = match RecommendedWatcher::new(handler, Config::default()) {
            Ok(val) => Some(LockWithTimeout::new(val)),
            Err(err) => {
                let mut err_lock = self.errors.0.write().unwrap();
                err_lock.push(ErrorArrayItem::new(Errors::InputOutput, err.to_string()));
                self.poisoned.store(true, Ordering::Relaxed);
                None
            }
        };

        if let Some(watcher) = watcher {
            match watcher.try_write().await {
                Ok(mut watcher) => {
                    if let Err(err) = watcher.watch(&self.dir, self.recursive) {
                        let mut err_lock = self.errors.0.write().unwrap();
                        err_lock.push(ErrorArrayItem::new(Errors::InputOutput, err.to_string()));
                        self.poisoned.store(true, Ordering::Relaxed);
                        return None;
                    }
                }
                Err(err) => {
                    let mut err_lock = self.errors.0.write().unwrap();
                    err_lock.push(ErrorArrayItem::new(Errors::InputOutput, err.to_string()));
                    self.poisoned.store(true, Ordering::Relaxed);
                    return None;
                }
            }

            if let Ok(mut inner_watcher) = self.watcher.try_write() {
                *inner_watcher = Some(watcher)
            }
            // self.watcher = Some(watcher);
            Some((watcher_tx, event_tx))
        } else {
            None
        }
    }

    fn spawn_event_loop(
        &self,
        watcher_tx: Arc<Mutex<broadcast::Sender<Event>>>,
        event_tx: Arc<broadcast::Sender<Event>>,
    ) {
        let cloned_controls = self.controls.clone();
        let cloned_ignored = self.ignored.clone();
        let cloned_timeout = self.timeout.load(Ordering::Relaxed);
        let cloned_event_tx = event_tx.clone();

        let mut watcher_rx = tokio::task::block_in_place(|| {
            // Must block to safely acquire lock in sync context
            tokio::runtime::Handle::current()
                .block_on(watcher_tx.lock())
                .subscribe()
        });

        let ignored_action = match self.monitor {
            MonitorMode::ALL => Vec::new(),
            MonitorMode::Modify => {
                let mut v = Vec::new();
                v.push(EventKind::Access(AccessKind::Open(AccessMode::Any)));
                v.push(EventKind::Access(AccessKind::Close(AccessMode::Any)));
                v.push(EventKind::Access(AccessKind::Close(AccessMode::Write)));
                v.push(EventKind::Modify(ModifyKind::Metadata(MetadataKind::Any)));
                v.push(EventKind::Access(AccessKind::Read));

                v
            }
            MonitorMode::Access => {
                let mut v = Vec::new();
                v.push(EventKind::Modify(ModifyKind::Any));
                v.push(EventKind::Remove(RemoveKind::Any));
                v.push(EventKind::Create(CreateKind::Any));
                v.push(EventKind::Modify(ModifyKind::Metadata(MetadataKind::Any)));
                v
            }
        };

        let handle = tokio::spawn(async move {
            loop {
                cloned_controls.wait_if_paused().await;

                match watcher_rx.recv().await {
                    Ok(event) => {
                        let ignored_path: bool = event.paths.iter().any(|path| {
                            cloned_ignored
                                .iter()
                                .any(|ignored| path.starts_with(ignored))
                        });

                        let ignored_operation: bool = ignored_action.contains(&event.kind);

                        if ignored_operation {
                            log!(LogLevel::Trace, "Ignored operation: {:#?}", event);
                            continue;
                        }

                        if ignored_path {
                            log!(
                                LogLevel::Trace,
                                "Ignoring event for ignored subdirectory: {:#?}",
                                event
                            );
                            continue;
                        }

                        if cloned_event_tx.send(event).is_err() {
                            log!(
                                LogLevel::Error,
                                "Failed to forward event: Event channel closed."
                            );
                            continue;
                        }
                    }
                    Err(err) => {
                        log!(
                            LogLevel::Error,
                            "Watcher channel closed unexpectedly. {}",
                            err.to_string()
                        );
                        sleep(Duration::from_secs(cloned_timeout)).await;
                    }
                }
            }
        });

        if let Ok(mut inner_handle) = self.handle.try_write() {
            *inner_handle = Some(handle);
        }

        if let Ok(mut inner_sender) = self.sender.try_write() {
            *inner_sender = Some(event_tx)
        }
    }

    /// remedy will automatically try to resolve the [`FileMonitor`] poison
    /// *THIS FUNCTION MAY PANIC* in cases of recovering channels and respawning
    /// if the monitor get's poisions while re-initializing we panic
    pub async fn health_check(&self, die_on_fail: bool) {
        self.events.fetch_add(1, Ordering::Relaxed);

        let mut bad_handle = false;
        let mut bad_chanel = false;
        let mut bad_errors = false;

        if self.missing.load(Ordering::Relaxed) == false {
            match self.dir.exists() {
                true => {
                    self.missing.store(false, Ordering::Relaxed);
                }
                false => {
                    self.poison("Target dir gone".into());
                    self.missing.store(true, Ordering::Relaxed);
                }
            }
        }

        if self.poisoned.load(Ordering::Relaxed) {
            // check handle
            if let Ok(handle) = self.handle.try_read() {
                match &*handle {
                    Some(data) => {
                        if data.is_finished() {
                            log!(LogLevel::Trace, "Handle finished, incorrectly");
                            bad_handle = true;
                        }
                    }
                    None => {
                        // log!(LogLevel::Warn, "No handle");
                        bad_handle = true;
                    }
                };
            }

            // check channel

            if let Ok(channel) = self.sender.try_read() {
                match &*channel {
                    Some(writer) => {
                        let reader = writer.subscribe();
                        if reader.is_closed() {
                            log!(LogLevel::Trace, "reader serves a closed channel");
                            bad_chanel = true;
                        }
                    }
                    None => {
                        log!(LogLevel::Trace, "Failed to get channel lock, skipping")
                    }
                }
            }

            // Event and error counts
            if self.errors.len() > 0 {
                bad_errors = true
            }

            // remedy
            if bad_chanel {
                // we'll kill the current monitor running on the invalid channel and restart it
                let handle_guard = self.handle.read().await;
                if let Some(handle) = &*handle_guard {
                    handle.abort();
                    bad_handle = true;
                }
            }

            if bad_chanel && bad_handle {
                if self.missing.load(Ordering::Relaxed) {
                    return;
                }

                if let Some(channels) = self.initialize_watcher_channels().await {
                    self.spawn_event_loop(channels.0, channels.1);
                } else {
                    if let Ok(mut err) = self.errors.0.try_write() {
                        err.push(ErrorArrayItem::new(
                            Errors::InputOutput,
                            "Failed establishing new channels",
                        ));
                        bad_errors = true
                    }
                }
            }

            if bad_errors {
                self.errors.clone().display(die_on_fail);
            }

            // clearing the poison
            if let Ok(mut err) = self.errors.0.try_write() {
                err.clear();
                self.poisoned.store(false, Ordering::Relaxed);
                self.events.store(0, Ordering::Relaxed);
            } else {
                log!(LogLevel::Error, "Failed to clear poison")
            }
        }
    }

    pub fn pause(&self) {
        self.controls.pause();
    }

    pub fn resume(&self) {
        self.controls.resume();
    }

    pub fn stop(&mut self) {
        match self.handle.try_write() {
            Ok(mut option_handle) => {
                if let Some(h) = option_handle.take() {
                    h.abort();
                }
            }
            Err(err) => {
                log!(
                    LogLevel::Error,
                    "Failed to stop monitor: {}",
                    err.to_string()
                );
                self.poison(err.to_string());
            }
        }
    }

    pub async fn subscribe(&self) -> Option<Receiver<Event>> {
        let sender_lock = self.sender.read().await;
        if let Some(sender) = &*sender_lock {
            Some(sender.subscribe())
        } else {
            if self.poisoned.load(Ordering::Relaxed) {
                log!(LogLevel::Warn, "Directory Monitor is poisoned");
            }
            None
        }
    }

    fn poison(&self, reason: String) {
        self.poisoned.store(true, Ordering::Relaxed);
        log!(
            LogLevel::Warn,
            "File system monitor poisoned: \"{}\".",
            reason
        );
        let mut err_lock = self.errors.0.write().unwrap();
        err_lock.push(ErrorArrayItem::new(Errors::InputOutput, reason));
    }

    fn sanitize_ignored_dirs(dir: PathType, dirs: Vec<PathType>) -> Vec<PathType> {
        if dirs.is_empty() {
            return Vec::new();
        }

        dirs.into_iter()
            .map(|path| PathType::PathBuf(dir.join(path))) // Convert to full paths relative to the monitored directory
            .collect()
    }
}

// fn directory_monitor()
