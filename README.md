# Dir Watcher

Dir Watcher is a small Rust library for monitoring filesystem events on a directory.
It wraps the [`notify`](https://docs.rs/notify/) crate and exposes an async API
for receiving change events via a Tokio `mpsc` channel.

## Features

- Monitor a directory recursively or non-recursively.
- Ignore specific subdirectories.
- Choose which events to track (all events, file modifications only or file access events).
- Built with Tokio and `notify` and can be integrated in asynchronous applications.

## Installation

Add the crate to your `Cargo.toml`:

```toml
[dependencies]
dir_watcher = "1.2.0"
```

## Quick start

```rust
use dir_watcher::{Options, RawFileMonitor, MonitorMode, RecursiveMode};
use dusa_collection_utils::core::types::pathtype::PathType;

#[tokio::main]
async fn main() {
    let mut options = Options::default();
    options
        .set_target_dir(PathType::Str("/tmp".into()))
        .set_interval(30)
        .set_mode(RecursiveMode::Recursive)
        .set_monitor_mode(MonitorMode::ALL);

    let watcher = RawFileMonitor::new(options).await;
    watcher.start().await;

    // subscribe() returns an optional `dir_receiver<Event>`
    if let Some(mut rx) = watcher.subscribe().await {
        while let Ok(event) = rx.recv().await {
            println!("{event:#?}");
        }
    }
}
```

## Options

`Options` provides a builder style API for configuring a watcher. You can set the
base directory, scan interval, whether the watcher validates the directory on
startup and which subdirectories or events to ignore.

## License

This project is released under the MIT license. See [LISCENSE](./LISCENSE) for
more information.
