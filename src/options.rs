use dusa_collection_utils::core::types::pathtype::PathType;
use notify::RecursiveMode;

use crate::object::MonitorMode;

#[derive(Clone, Debug)]
pub struct Options {
    target_dir: PathType,
    scan_interval: u64,
    ignored_subdirs: Vec<PathType>,
    recursive_mode: RecursiveMode,
    monitor_mode: MonitorMode,
    validate_path: bool,
}

impl Options {
    pub fn default() -> Self {
        let target_dir: PathType = PathType::Str("/tmp".into());
        let scan_interval: u64 = 30;
        let ignored_subdirs: Vec<PathType> = Vec::new();
        let recursive_mode = RecursiveMode::Recursive;
        let validate_path = false;
        let monitor_mode = MonitorMode::Modify;

        Self {
            target_dir,
            scan_interval,
            ignored_subdirs,
            recursive_mode,
            validate_path,
            monitor_mode
        }
    }

    pub fn set_target_dir(&mut self, path: PathType) -> Self {
        self.target_dir = path;
        self.clone()
    }

    pub fn set_interval(&mut self, int: u64) -> Self {
        self.scan_interval = int;
        self.clone()
    }

    pub fn add_ignored_dir(&mut self, path: PathType) -> Self {
        self.ignored_subdirs.push(path);
        self.clone()
    }

    pub fn add_ignored_dirs(&mut self, mut paths: Vec<PathType>) -> Self {
        self.ignored_subdirs.append(&mut paths);
        self.clone()
    }

    pub fn set_mode(&mut self, recursive_mode: RecursiveMode) -> Self {
        self.recursive_mode = recursive_mode;
        self.clone()
    }

    pub fn set_validation(&mut self, validate: bool) -> Self {
        self.validate_path = validate;
        self.clone()
    }

    pub fn set_monitor_mode(&mut self, monitor_mode: MonitorMode) -> Self {
        self.monitor_mode = monitor_mode;
        self.clone()
    }

    pub(crate) fn get_monitor_mode(&self) -> MonitorMode {
        self.monitor_mode.clone()
    }

    pub(crate) fn base_dir(&self) -> PathType {
        self.target_dir.clone()
    }

    pub(crate) fn ignored_dirs(&self) -> Vec<PathType> {
        self.ignored_subdirs.clone()
    }

    pub(crate) fn get_interval(&self) ->  u64 {
        self.scan_interval.clone()
    }

    pub(crate) fn get_mode(&self) -> RecursiveMode {
        self.recursive_mode.clone()
    }

    pub(crate) fn get_validation(&self) -> bool {
        self.validate_path
    }
}
