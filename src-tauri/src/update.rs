//! The updater: a check after launch, another every six hours, and an install
//! only when the banner is clicked.
//!
//! The window is React, but the updater stays here rather than in
//! `@tauri-apps/plugin-updater`. No other plugin is reachable from JavaScript in
//! this app, and keeping the check in Rust means the window has three commands
//! and no more: read the state, ask again, install what is waiting.

use serde::Serialize;
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Manager};
use tauri_plugin_updater::UpdaterExt;

/// Late enough that a slow start is not competing with a download, early enough
/// that a released fix is offered while the window is still open. Crossup is
/// usually left in the tray, so the repeat check is what most people see.
const CHECK_AFTER_LAUNCH: Duration = Duration::from_secs(20);
const CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateView {
    current_version: String,
    /// idle, checking, available, downloading, installing, restarting, upToDate, error
    status: String,
    version: Option<String>,
    notes: Option<String>,
    downloaded: u64,
    total: Option<u64>,
    error: Option<String>,
}

struct Inner {
    status: String,
    version: Option<String>,
    notes: Option<String>,
    downloaded: u64,
    total: Option<u64>,
    error: Option<String>,
}

pub struct UpdateHolder {
    inner: Mutex<Inner>,
}

impl UpdateHolder {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                status: "idle".into(),
                version: None,
                notes: None,
                downloaded: 0,
                total: None,
                error: None,
            }),
        }
    }

    fn view(&self, app: &AppHandle) -> UpdateView {
        let inner = self.inner.lock().unwrap();
        UpdateView {
            current_version: app.package_info().version.to_string(),
            status: inner.status.clone(),
            version: inner.version.clone(),
            notes: inner.notes.clone(),
            downloaded: inner.downloaded,
            total: inner.total,
            error: inner.error.clone(),
        }
    }

    /// Whether a download or an install is already past the point of no return.
    fn busy(&self) -> bool {
        let inner = self.inner.lock().unwrap();
        matches!(
            inner.status.as_str(),
            "downloading" | "installing" | "restarting"
        )
    }
}

/// Mark a check as started. False means something was already running, and the
/// caller answers with the state it has rather than starting a second request.
fn begin(app: &AppHandle) -> bool {
    let holder = app.state::<UpdateHolder>();
    if holder.busy() {
        return false;
    }
    let mut inner = holder.inner.lock().unwrap();
    inner.status = "checking".into();
    inner.error = None;
    true
}

fn available(app: &AppHandle, update: tauri_plugin_updater::Update) {
    let holder = app.state::<UpdateHolder>();
    let mut inner = holder.inner.lock().unwrap();
    inner.status = "available".into();
    inner.version = Some(update.version);
    inner.notes = update.body;
    inner.downloaded = 0;
    inner.total = None;
    inner.error = None;
}

fn up_to_date(app: &AppHandle) {
    let holder = app.state::<UpdateHolder>();
    let mut inner = holder.inner.lock().unwrap();
    inner.status = "upToDate".into();
    inner.version = None;
    inner.notes = None;
    inner.downloaded = 0;
    inner.total = None;
    inner.error = None;
}

fn failed(app: &AppHandle, error: String) {
    let holder = app.state::<UpdateHolder>();
    let mut inner = holder.inner.lock().unwrap();
    inner.status = "error".into();
    inner.error = Some(error);
}

/// Check the endpoint and record the answer. A network failure is state, not a
/// rejected command, so the window can say why nothing was offered.
pub async fn check(app: &AppHandle) -> UpdateView {
    if !begin(app) {
        return app.state::<UpdateHolder>().view(app);
    }

    let result = match app.updater() {
        Ok(updater) => updater.check().await,
        Err(error) => Err(error),
    };

    match result {
        Ok(Some(update)) => available(app, update),
        Ok(None) => up_to_date(app),
        Err(error) => failed(app, error.to_string()),
    }

    app.state::<UpdateHolder>().view(app)
}

/// Download the advertised release, verify its signature and install it.
///
/// The block callback keeps the banner's byte count moving, so a slow connection
/// reads as progress rather than a stalled click. Windows finishes inside the
/// NSIS installer; the restart below is what brings the new build back.
async fn install(app: &AppHandle) {
    if !begin(app) {
        return;
    }

    let update = match app.updater() {
        Ok(updater) => match updater.check().await {
            Ok(Some(update)) => update,
            Ok(None) => {
                up_to_date(app);
                return;
            }
            Err(error) => {
                failed(app, error.to_string());
                return;
            }
        },
        Err(error) => {
            failed(app, error.to_string());
            return;
        }
    };

    {
        let holder = app.state::<UpdateHolder>();
        let mut inner = holder.inner.lock().unwrap();
        inner.status = "downloading".into();
        inner.version = Some(update.version.clone());
        inner.notes = update.body.clone();
        inner.downloaded = 0;
        inner.total = None;
        inner.error = None;
    }

    let progress = app.clone();
    let finished = app.clone();
    let result = update
        .download_and_install(
            move |chunk, total| {
                let holder = progress.state::<UpdateHolder>();
                let mut inner = holder.inner.lock().unwrap();
                inner.downloaded += chunk as u64;
                if total.is_some() {
                    inner.total = total;
                }
            },
            move || {
                let holder = finished.state::<UpdateHolder>();
                holder.inner.lock().unwrap().status = "installing".into();
            },
        )
        .await;

    if let Err(error) = result {
        failed(app, error.to_string());
        return;
    }

    {
        let holder = app.state::<UpdateHolder>();
        holder.inner.lock().unwrap().status = "restarting".into();
    }
    app.restart();
}

#[tauri::command]
pub fn update_state(app: AppHandle) -> UpdateView {
    app.state::<UpdateHolder>().view(&app)
}

#[tauri::command]
pub async fn update_check(app: AppHandle) -> UpdateView {
    check(&app).await
}

/// Started rather than awaited: a download is minutes long and the window has to
/// keep drawing, so the banner follows it through `update_state` instead.
#[tauri::command]
pub fn update_install(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        install(&app).await;
    });
}

/// The launch check runs off the UI thread, so the window never waits on GitHub.
/// A failed check is deliberately quiet: this build keeps working and the next
/// launch tries again.
pub fn watch(app: AppHandle) {
    std::thread::spawn(move || {
        std::thread::sleep(CHECK_AFTER_LAUNCH);
        loop {
            let handle = app.clone();
            let _ = tauri::async_runtime::block_on(check(&handle));
            std::thread::sleep(CHECK_EVERY);
        }
    });
}
