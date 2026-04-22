pub mod core;
pub mod models;
pub mod session_store;

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

use reqwest::header::{COOKIE, USER_AGENT};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use tauri::path::BaseDirectory;
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder};

use crate::core::{
    browser_user_agent, build_http_client, prepare_download_path as shared_prepare_download_path,
    resolve_download_url, search_exam_papers_with_cookie, validate_session_cookie_header,
};
use crate::models::{AuthSessionStatus, SearchCriterion, SearchRequest, AUTH_WINDOW_URL};
use crate::session_store::{load_session_snapshot, save_session_snapshot, SessionSnapshot};

const AUTH_WINDOW_LABEL: &str = "auth-window";
const MAIN_WINDOW_LABEL: &str = "main";
const CLI_COMMAND_NAME: &str = "nus-exam-papers";

#[cfg(target_os = "windows")]
const CLI_RESOURCE_NAME: &str = "cli/nus-exam-papers.exe";

#[cfg(not(target_os = "windows"))]
const CLI_RESOURCE_NAME: &str = "cli/nus-exam-papers";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DownloadRequest {
    job_id: String,
    destination_directory: String,
    requested_name: String,
    view_url: String,
    download_url: Option<String>,
}

#[derive(Clone, Default)]
struct DownloadState {
    cancellations: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct DownloadProgressPayload {
    job_id: String,
    bytes_received: u64,
    bytes_total: Option<u64>,
    progress_percent: Option<u64>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct DownloadCompletedPayload {
    job_id: String,
    destination_path: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct DownloadFailedPayload {
    job_id: String,
    message: String,
    cancelled: bool,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct CliToolStatus {
    platform: String,
    command_name: String,
    bundled: bool,
    bundled_path: Option<String>,
    install_action_available: bool,
    install_path: Option<String>,
    path_managed_by_installer: bool,
    in_path: bool,
    message: String,
}

fn ensure_auth_window(app: &AppHandle, visible: bool) -> Result<tauri::WebviewWindow, String> {
    if let Some(window) = app.get_webview_window(AUTH_WINDOW_LABEL) {
        if visible {
            window.show().map_err(|error| error.to_string())?;
        }
        return Ok(window);
    }

    let url = AUTH_WINDOW_URL
        .parse()
        .map_err(|error| format!("invalid auth url: {error}"))?;

    WebviewWindowBuilder::new(app, AUTH_WINDOW_LABEL, WebviewUrl::External(url))
        .title("Digital Gems Login")
        .inner_size(1180.0, 860.0)
        .resizable(true)
        .center()
        .visible(visible)
        .build()
        .map_err(|error| error.to_string())
}

fn get_authenticated_cookie_header(
    window: &tauri::WebviewWindow,
    url: &Url,
) -> Result<String, String> {
    let cookies = window
        .cookies_for_url(url.clone())
        .map_err(|error| error.to_string())?;

    if cookies.is_empty() {
        return Err("No authenticated cookies were found. Log in again first.".into());
    }

    Ok(cookies
        .into_iter()
        .map(|cookie| format!("{}={}", cookie.name(), cookie.value()))
        .collect::<Vec<_>>()
        .join("; "))
}

fn persist_session_cookie_header(cookie_header: String, source_url: &str) {
    let snapshot = SessionSnapshot::new(cookie_header, source_url.to_string());
    let _ = save_session_snapshot(&snapshot);
}

fn resolve_cookie_header_from_window_or_store(
    window: &tauri::WebviewWindow,
    url: &Url,
) -> Result<String, String> {
    if let Ok(cookie_header) = get_authenticated_cookie_header(window, url) {
        persist_session_cookie_header(cookie_header.clone(), url.as_str());
        return Ok(cookie_header);
    }

    let snapshot = load_session_snapshot()?;
    Ok(snapshot.cookie_header)
}

fn bundled_cli_resource_path(app: &AppHandle) -> Result<PathBuf, String> {
    for candidate in bundled_cli_resource_candidates(app)? {
        if candidate.exists() {
            return Ok(candidate);
        }
    }

    app.path()
        .resolve(CLI_RESOURCE_NAME, BaseDirectory::Resource)
        .map_err(|error| error.to_string())
}

fn bundled_cli_resource_candidates(app: &AppHandle) -> Result<Vec<PathBuf>, String> {
    let mut candidates = Vec::new();

    if let Ok(path) = app.path().resolve(CLI_RESOURCE_NAME, BaseDirectory::Resource) {
        candidates.push(path);
    }

    #[cfg(target_os = "windows")]
    if let Ok(path) = app
        .path()
        .resolve("nus-exam-papers.exe", BaseDirectory::Resource)
    {
        candidates.push(path);
    }

    #[cfg(not(target_os = "windows"))]
    if let Ok(path) = app.path().resolve("nus-exam-papers", BaseDirectory::Resource) {
        candidates.push(path);
    }

    if let Ok(resource_dir) = app.path().resource_dir() {
        candidates.push(resource_dir.join(CLI_RESOURCE_NAME));
        #[cfg(target_os = "windows")]
        candidates.push(resource_dir.join("nus-exam-papers.exe"));
        #[cfg(not(target_os = "windows"))]
        candidates.push(resource_dir.join("nus-exam-papers"));
    }

    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            #[cfg(target_os = "windows")]
            candidates.push(exe_dir.join("nus-exam-papers.exe"));
            #[cfg(not(target_os = "windows"))]
            candidates.push(exe_dir.join("nus-exam-papers"));

            candidates.push(exe_dir.join(CLI_RESOURCE_NAME));
            if let Some(parent) = exe_dir.parent() {
                candidates.push(parent.join("Resources").join(CLI_RESOURCE_NAME));
                #[cfg(target_os = "windows")]
                candidates.push(parent.join("Resources").join("nus-exam-papers.exe"));
                #[cfg(not(target_os = "windows"))]
                candidates.push(parent.join("Resources").join("nus-exam-papers"));
            }
        }
    }

    candidates.sort();
    candidates.dedup();
    Ok(candidates)
}

fn cli_path_contains(target_directory: &std::path::Path) -> bool {
    std::env::var_os("PATH")
        .map(|value| {
            std::env::split_paths(&value).any(|entry| {
                entry
                    .canonicalize()
                    .ok()
                    .zip(target_directory.canonicalize().ok())
                    .map(|(left, right)| left == right)
                    .unwrap_or_else(|| entry == target_directory)
            })
        })
        .unwrap_or(false)
}

fn cli_tool_status(app: &AppHandle) -> Result<CliToolStatus, String> {
    let bundled_path = bundled_cli_resource_path(app)?;
    let bundled = bundled_path.exists();

    #[cfg(target_os = "macos")]
    {
        let home_dir = app.path().home_dir().map_err(|error| error.to_string())?;
        let preferred_install_path = home_dir.join(".local").join("bin").join(CLI_COMMAND_NAME);
        let install_dir = preferred_install_path.parent().map(PathBuf::from);
        let in_path = install_dir
            .as_deref()
            .map(cli_path_contains)
            .unwrap_or(false);

        return Ok(CliToolStatus {
            platform: "macos".to_string(),
            command_name: CLI_COMMAND_NAME.to_string(),
            bundled,
            bundled_path: bundled.then(|| bundled_path.to_string_lossy().to_string()),
            install_action_available: true,
            install_path: Some(preferred_install_path.to_string_lossy().to_string()),
            path_managed_by_installer: false,
            in_path,
            message: if bundled {
                if in_path {
                    "The bundled CLI is available and ~/.local/bin is already on PATH.".to_string()
                } else {
                    "Install the bundled CLI for terminal use. If ~/.local/bin is not on PATH yet, the app will remind you after installation.".to_string()
                }
            } else {
                "The CLI is installed automatically in packaged builds. It is not bundled in this development run.".to_string()
            },
        });
    }

    #[cfg(target_os = "windows")]
    {
        let install_dir = bundled_path.parent().map(PathBuf::from);
        let in_path = install_dir
            .as_deref()
            .map(cli_path_contains)
            .unwrap_or(false);

        return Ok(CliToolStatus {
            platform: "windows".to_string(),
            command_name: CLI_COMMAND_NAME.to_string(),
            bundled,
            bundled_path: bundled.then(|| bundled_path.to_string_lossy().to_string()),
            install_action_available: false,
            install_path: install_dir.map(|path| path.to_string_lossy().to_string()),
            path_managed_by_installer: true,
            in_path,
            message: if bundled {
                "The MSI installer adds the app install directory to PATH. Open a new terminal after installation to use the CLI command.".to_string()
            } else {
                "The packaged Windows installer adds CLI access to PATH. This development build does not stage the packaged CLI resource.".to_string()
            },
        });
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        Ok(CliToolStatus {
            platform: std::env::consts::OS.to_string(),
            command_name: CLI_COMMAND_NAME.to_string(),
            bundled,
            bundled_path: bundled.then(|| bundled_path.to_string_lossy().to_string()),
            install_action_available: false,
            install_path: None,
            path_managed_by_installer: false,
            in_path: false,
            message: "CLI installation helpers are currently only wired for the packaged macOS and Windows app flows.".to_string(),
        })
    }
}

#[tauri::command]
fn open_auth_window(app: AppHandle) -> Result<(), String> {
    let window = ensure_auth_window(&app, true)?;
    window.set_focus().map_err(|error| error.to_string())?;
    Ok(())
}

#[tauri::command]
fn get_cli_tool_status(app: AppHandle) -> Result<CliToolStatus, String> {
    cli_tool_status(&app)
}

#[tauri::command]
fn install_cli_tool(_app: AppHandle) -> Result<CliToolStatus, String> {
    #[cfg(target_os = "macos")]
    {
        let app = _app;
        use std::os::unix::fs::symlink;

        let bundled_path = bundled_cli_resource_path(&app)?;
        if !bundled_path.exists() {
            return Err(
                "The packaged CLI binary was not found in this app build. Build a bundled macOS app to install terminal support."
                    .to_string(),
            );
        }

        let home_dir = app.path().home_dir().map_err(|error| error.to_string())?;
        let local_bin_dir = home_dir.join(".local").join("bin");
        fs::create_dir_all(&local_bin_dir).map_err(|error| error.to_string())?;
        let target_path = local_bin_dir.join(CLI_COMMAND_NAME);

        if target_path.exists() || target_path.is_symlink() {
            fs::remove_file(&target_path).map_err(|error| error.to_string())?;
        }

        symlink(&bundled_path, &target_path).map_err(|error| error.to_string())?;

        let mut status = cli_tool_status(&app)?;
        status.install_path = Some(target_path.to_string_lossy().to_string());
        status.in_path = cli_path_contains(&local_bin_dir);
        status.message = if status.in_path {
            format!(
                "CLI installed at {}. Open a new terminal and run `{}`.",
                target_path.to_string_lossy(),
                CLI_COMMAND_NAME
            )
        } else {
            format!(
                "CLI installed at {}. Add {} to PATH, then run `{}` from a new terminal.",
                target_path.to_string_lossy(),
                local_bin_dir.to_string_lossy(),
                CLI_COMMAND_NAME
            )
        };
        return Ok(status);
    }

    #[cfg(not(target_os = "macos"))]
    {
        Err("In-app CLI installation is currently only needed on macOS. On Windows, install the MSI and open a new terminal to use the bundled CLI from PATH.".to_string())
    }
}

#[tauri::command]
fn show_auth_window(app: AppHandle, url: Option<String>) -> Result<(), String> {
    let window = ensure_auth_window(&app, true)?;

    if let Some(target_url) = url {
        let parsed = target_url
            .parse()
            .map_err(|error| format!("invalid target url: {error}"))?;
        window
            .navigate(parsed)
            .map_err(|error| error.to_string())?;
    }

    window.show().map_err(|error| error.to_string())?;
    window.set_focus().map_err(|error| error.to_string())?;
    Ok(())
}

#[tauri::command]
fn hide_auth_window(app: AppHandle) -> Result<(), String> {
    let window = ensure_auth_window(&app, false)?;
    window.hide().map_err(|error| error.to_string())
}

#[tauri::command]
async fn bootstrap_auth_session(app: AppHandle) -> Result<AuthSessionStatus, String> {
    let window = ensure_auth_window(&app, false)?;
    validate_auth_session(&window, true).await
}

#[tauri::command]
async fn confirm_auth_session(
    app: AppHandle,
    auto_close: bool,
) -> Result<AuthSessionStatus, String> {
    let window = ensure_auth_window(&app, false)?;
    validate_auth_session(&window, auto_close).await
}

#[tauri::command]
fn eval_auth_script(app: AppHandle, label: String, script: String) -> Result<(), String> {
    let window = app
        .get_webview_window(&label)
        .ok_or_else(|| format!("window not found: {label}"))?;
    window.eval(&script).map_err(|error| error.to_string())
}

async fn validate_auth_session(
    window: &tauri::WebviewWindow,
    auto_close: bool,
) -> Result<AuthSessionStatus, String> {
    let collection_url: Url = AUTH_WINDOW_URL
        .parse()
        .map_err(|error| format!("invalid collection url: {error}"))?;

    let current_url = window.url().map_err(|error| error.to_string())?;
    let cookie_header = match get_authenticated_cookie_header(window, &collection_url) {
        Ok(header) => {
            persist_session_cookie_header(header.clone(), current_url.as_str());
            header
        }
        Err(_) => match load_session_snapshot() {
            Ok(snapshot) => snapshot.cookie_header,
            Err(_) => {
                return Ok(AuthSessionStatus {
                    ready: false,
                    current_url: current_url.to_string(),
                    message: "No saved Digital Gems session was found. Sign in to continue."
                        .to_string(),
                })
            }
        },
    };

    let status = validate_session_cookie_header(&cookie_header).await?;

    if status.ready {
        let _ = window.navigate(collection_url);
        if auto_close {
            let _ = window.hide();
        }
        persist_session_cookie_header(cookie_header, &status.current_url);
    }

    Ok(status)
}

#[tauri::command]
async fn search_exam_papers(
    app: AppHandle,
    criteria: Vec<SearchCriterion>,
    search_url: Option<String>,
    raw_query_clauses: Option<Vec<String>>,
    facet_clauses: Option<Vec<String>>,
    cursor: Option<String>,
) -> Result<crate::models::SearchResponse, String> {
    let window = ensure_auth_window(&app, false)?;
    let collection_url: Url = AUTH_WINDOW_URL
        .parse()
        .map_err(|error| format!("invalid collection url: {error}"))?;
    let cookie_header = resolve_cookie_header_from_window_or_store(&window, &collection_url)?;

    let response = search_exam_papers_with_cookie(
        &cookie_header,
        SearchRequest {
            criteria,
            search_url,
            raw_query_clauses,
            facet_clauses,
            cursor,
            limit: None,
        },
    )
    .await?;

    persist_session_cookie_header(cookie_header, response.search_url.as_deref().unwrap_or(AUTH_WINDOW_URL));
    Ok(response)
}

#[tauri::command]
async fn start_download(
    app: AppHandle,
    state: State<'_, DownloadState>,
    request: DownloadRequest,
) -> Result<(), String> {
    let window = ensure_auth_window(&app, false)?;
    let view_url = Url::parse(&request.view_url).map_err(|error| error.to_string())?;
    let cookie_header = resolve_cookie_header_from_window_or_store(&window, &view_url)?;
    let client = build_http_client()?;
    let cancel_flag = Arc::new(AtomicBool::new(false));

    {
        let mut cancellations = state
            .cancellations
            .lock()
            .map_err(|_| "Download state lock poisoned.".to_string())?;
        cancellations.insert(request.job_id.clone(), cancel_flag.clone());
    }

    let result = async {
        let download_url = resolve_download_url(
            &client,
            &cookie_header,
            &request.view_url,
            request.download_url.as_deref(),
        )
        .await?;
        let target_path = shared_prepare_download_path(
            request.destination_directory.clone(),
            request.requested_name.clone(),
        )?;

        let mut response = client
            .get(download_url.clone())
            .header(USER_AGENT, browser_user_agent())
            .header(COOKIE, cookie_header.clone())
            .send()
            .await
            .map_err(|error| error.to_string())?;

        if !response.status().is_success() {
            return Err(format!(
                "Digital Gems returned {} while downloading the PDF.",
                response.status()
            ));
        }

        let total = response.content_length();
        let mut file = fs::File::create(&target_path).map_err(|error| error.to_string())?;
        let mut received = 0u64;

        while let Some(chunk) = response.chunk().await.map_err(|error| error.to_string())? {
            if cancel_flag.load(Ordering::SeqCst) {
                let _ = fs::remove_file(&target_path);
                let _ = app.emit(
                    "download:failed",
                    DownloadFailedPayload {
                        job_id: request.job_id.clone(),
                        message: "Download cancelled.".into(),
                        cancelled: true,
                    },
                );
                return Ok(());
            }

            file.write_all(&chunk).map_err(|error| error.to_string())?;
            received += chunk.len() as u64;
            let progress_percent = total.map(|value| ((received * 100) / value).min(100));
            let _ = app.emit(
                "download:progress",
                DownloadProgressPayload {
                    job_id: request.job_id.clone(),
                    bytes_received: received,
                    bytes_total: total,
                    progress_percent,
                },
            );
        }

        file.flush().map_err(|error| error.to_string())?;
        persist_session_cookie_header(cookie_header.clone(), &request.view_url);
        let _ = app.emit(
            "download:completed",
            DownloadCompletedPayload {
                job_id: request.job_id.clone(),
                destination_path: target_path,
            },
        );
        Ok(())
    }
    .await;

    {
        if let Ok(mut cancellations) = state.cancellations.lock() {
            cancellations.remove(&request.job_id);
        }
    }

    if let Err(message) = result {
        let _ = app.emit(
            "download:failed",
            DownloadFailedPayload {
                job_id: request.job_id,
                message: message.clone(),
                cancelled: false,
            },
        );
        return Err(message);
    }

    Ok(())
}

#[tauri::command]
fn cancel_download(state: State<'_, DownloadState>, job_id: String) -> Result<(), String> {
    let cancellations = state
        .cancellations
        .lock()
        .map_err(|_| "Download state lock poisoned.".to_string())?;
    let flag = cancellations
        .get(&job_id)
        .ok_or_else(|| "Download is no longer running.".to_string())?;
    flag.store(true, Ordering::SeqCst);
    Ok(())
}

#[tauri::command]
fn prepare_download_path(directory: String, requested_name: String) -> Result<String, String> {
    shared_prepare_download_path(directory, requested_name)
}

#[tauri::command]
fn write_binary_file(path: String, bytes: Vec<u8>) -> Result<(), String> {
    fs::write(path, bytes).map_err(|error| error.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(DownloadState::default())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            if let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) {
                window.maximize().map_err(|error| error.to_string())?;
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            open_auth_window,
            get_cli_tool_status,
            install_cli_tool,
            show_auth_window,
            hide_auth_window,
            bootstrap_auth_session,
            confirm_auth_session,
            eval_auth_script,
            search_exam_papers,
            start_download,
            cancel_download,
            prepare_download_path,
            write_binary_file
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
