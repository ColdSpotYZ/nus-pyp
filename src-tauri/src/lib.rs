use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

use reqwest::header::{COOKIE, USER_AGENT};
use reqwest::Url;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, Position, Size, State, Window,
    WebviewUrl, WebviewWindowBuilder, WindowEvent,
};

const AUTH_WINDOW_LABEL: &str = "auth-window";
const AUTH_WINDOW_URL: &str = "https://digitalgems.nus.edu.sg/browse/collection/31";
const SEARCH_LIMIT: usize = 10;
const MAIN_WINDOW_LABEL: &str = "main";
const MAIN_WINDOW_STATE_FILE: &str = "main-window-state.json";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SearchCriterion {
    field: String,
    condition: String,
    operator: String,
    value: Option<String>,
    value2: Option<String>,
    values: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ExamPaperResult {
    id: String,
    title: String,
    course_code: Option<String>,
    course_name: Option<String>,
    year: Option<String>,
    semester: Option<String>,
    view_url: String,
    download_url: Option<String>,
    downloadable: bool,
    unavailable_reason: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SearchResponse {
    results: Vec<ExamPaperResult>,
    total_results: Option<usize>,
    facets: Vec<SearchFacetGroup>,
    search_url: Option<String>,
    cursor: Option<String>,
    has_more: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SearchFacetGroup {
    id: String,
    title: String,
    values: Vec<SearchFacetValue>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SearchFacetValue {
    id: String,
    label: String,
    count: usize,
    href: String,
    query_clauses: Vec<String>,
}

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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AuthSessionStatus {
    ready: bool,
    current_url: String,
    message: String,
}

#[derive(Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct MainWindowState {
    user_resized: bool,
    is_maximized: bool,
    width: Option<u32>,
    height: Option<u32>,
    x: Option<i32>,
    y: Option<i32>,
}

fn main_window_state_path(app: &AppHandle) -> Result<PathBuf, String> {
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?;
    fs::create_dir_all(&app_data_dir).map_err(|error| error.to_string())?;
    Ok(app_data_dir.join(MAIN_WINDOW_STATE_FILE))
}

fn load_main_window_state(app: &AppHandle) -> Result<MainWindowState, String> {
    let path = main_window_state_path(app)?;
    if !path.exists() {
        return Ok(MainWindowState::default());
    }

    let contents = fs::read_to_string(path).map_err(|error| error.to_string())?;
    serde_json::from_str(&contents).map_err(|error| error.to_string())
}

fn save_main_window_state(app: &AppHandle, state: &MainWindowState) -> Result<(), String> {
    let path = main_window_state_path(app)?;
    let contents = serde_json::to_string(state).map_err(|error| error.to_string())?;
    fs::write(path, contents).map_err(|error| error.to_string())
}

fn persist_main_window_state(window: &Window) -> Result<(), String> {
    let app = window.app_handle();
    let is_maximized = window.is_maximized().map_err(|error| error.to_string())?;
    let mut state = load_main_window_state(&app)?;
    state.user_resized = true;
    state.is_maximized = is_maximized;

    if !is_maximized {
        let size = window.outer_size().map_err(|error| error.to_string())?;
        let position = window.outer_position().map_err(|error| error.to_string())?;
        state.width = Some(size.width);
        state.height = Some(size.height);
        state.x = Some(position.x);
        state.y = Some(position.y);
    }

    save_main_window_state(&app, &state)
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

#[tauri::command]
fn open_auth_window(app: AppHandle) -> Result<(), String> {
    let window = ensure_auth_window(&app, true)?;
    window.set_focus().map_err(|error| error.to_string())?;
    Ok(())
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
        Ok(header) => header,
        Err(_) => {
            return Ok(AuthSessionStatus {
                ready: false,
                current_url: current_url.to_string(),
                message: "No saved Digital Gems session was found. Sign in to continue."
                    .to_string(),
            })
        }
    };

    let client = build_http_client()?;
    let response = client
        .get(collection_url.clone())
        .header(USER_AGENT, browser_user_agent())
        .header(COOKIE, cookie_header)
        .send()
        .await
        .map_err(|error| error.to_string())?;

    let status = response.status();
    let final_url = response.url().clone();
    let html = response.text().await.map_err(|error| error.to_string())?;
    if !status.is_success() {
        return Ok(AuthSessionStatus {
            ready: false,
            current_url: final_url.to_string(),
            message: format!(
                "Your Digital Gems session is missing or expired. Validation returned HTTP {}.",
                status
            ),
        });
    }
    let ready = is_authenticated_digital_gems_page(&final_url, &html);

    if ready {
        let _ = window.navigate(collection_url);
        if auto_close {
            let _ = window.hide();
        }
        Ok(AuthSessionStatus {
            ready: true,
            current_url: final_url.to_string(),
            message:
                "Saved Digital Gems session loaded. Search and downloads are ready.".to_string(),
        })
    } else {
        Ok(AuthSessionStatus {
            ready: false,
            current_url: final_url.to_string(),
            message:
                "Your Digital Gems session is missing or expired. Sign in to continue."
                    .to_string(),
        })
    }
}

#[tauri::command]
async fn search_exam_papers(
    app: AppHandle,
    criteria: Vec<SearchCriterion>,
    search_url: Option<String>,
    raw_query_clauses: Option<Vec<String>>,
    facet_clauses: Option<Vec<String>>,
    cursor: Option<String>,
) -> Result<SearchResponse, String> {
    let window = ensure_auth_window(&app, false)?;

    let collection_url: Url = AUTH_WINDOW_URL
        .parse()
        .map_err(|error| format!("invalid collection url: {error}"))?;
    let cookie_header = get_authenticated_cookie_header(&window, &collection_url)?;

    let offset = cursor
        .as_deref()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    let page = (offset / SEARCH_LIMIT) + 1;

    let url = if let Some(search_url) = search_url {
        apply_page_to_search_url(&search_url, page)?
    } else {
        let mut built = collection_url;
        let mut query = built.query_pairs_mut();
        query.append_pair("limit", &SEARCH_LIMIT.to_string());
        if page > 1 {
            query.append_pair("page", &page.to_string());
        }
        if let Some(raw_query_clauses) = raw_query_clauses {
            for clause in raw_query_clauses {
                query.append_pair("q", &clause);
            }
        } else {
            for clause in facet_clauses.unwrap_or_default() {
                query.append_pair("q", &clause);
            }
            for criterion in &criteria {
                for clause in build_query_clauses(criterion)? {
                    query.append_pair("q", &clause);
                }
            }
        }
        drop(query);
        built
    };

    let client = build_http_client()?;

    let response = client
        .get(url.clone())
        .header(
            USER_AGENT,
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko)",
        )
        .header(COOKIE, cookie_header)
        .send()
        .await
        .map_err(|error| error.to_string())?;

    let status = response.status();
    let final_url = response.url().clone();
    let html = response.text().await.map_err(|error| error.to_string())?;
    if !status.is_success() {
        let page_title = extract_page_title(&html).unwrap_or_else(|| "Unknown page".to_string());
        return Err(format!(
            "Digital Gems returned HTTP {} while searching. Final URL: {}. Page title: {}.",
            status, final_url, page_title
        ));
    }
    if !is_authenticated_digital_gems_page(&final_url, &html) {
        return Err(format!(
            "The Digital Gems session is missing or expired. Sign in again first. Validation URL: {}",
            final_url
        ));
    }
    let results = parse_search_results(&html, &final_url)?;
    let facets = parse_search_facets(&html)?;
    let total_results = extract_total_results(&facets, &html);
    if results.is_empty() && !html_explicitly_has_no_results(&html) {
        let page_title = extract_page_title(&html).unwrap_or_else(|| "Unknown page".to_string());
        return Err(format!(
            "Digital Gems returned an unexpected page shape while searching. Final URL: {}. Page title: {}.",
            final_url, page_title
        ));
    }
    let offset = cursor
        .as_deref()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    let has_more = total_results
        .map(|total| offset + results.len() < total)
        .unwrap_or(results.len() == SEARCH_LIMIT);
    let next_cursor = has_more.then(|| (offset + SEARCH_LIMIT).to_string());

    Ok(SearchResponse {
        results,
        total_results,
        facets,
        search_url: Some(final_url.to_string()),
        cursor: next_cursor,
        has_more,
    })
}

#[tauri::command]
async fn start_download(
    app: AppHandle,
    state: State<'_, DownloadState>,
    request: DownloadRequest,
) -> Result<(), String> {
    let window = ensure_auth_window(&app, false)?;

    let view_url = Url::parse(&request.view_url).map_err(|error| error.to_string())?;
    let cookie_header = get_authenticated_cookie_header(&window, &view_url)?;
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
        let target_path =
            prepare_download_path(request.destination_directory.clone(), request.requested_name.clone())?;

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
    let sanitized_name = sanitize_filename(&requested_name);
    let target_directory = PathBuf::from(directory);

    if !target_directory.exists() {
        return Err("The chosen destination folder does not exist.".into());
    }

    let stem = Path::new(&sanitized_name)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("exam-paper");
    let extension = Path::new(&sanitized_name)
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| format!(".{value}"))
        .unwrap_or_default();

    for index in 0..10_000 {
        let candidate_name = if index == 0 {
            format!("{stem}{extension}")
        } else {
            format!("{stem} ({index}){extension}")
        };
        let candidate_path = target_directory.join(candidate_name);
        if !candidate_path.exists() {
            return Ok(candidate_path.to_string_lossy().to_string());
        }
    }

    Err("Unable to allocate a unique filename in the selected folder.".into())
}

#[tauri::command]
fn write_binary_file(path: String, bytes: Vec<u8>) -> Result<(), String> {
    fs::write(path, bytes).map_err(|error| error.to_string())
}

fn sanitize_filename(input: &str) -> String {
    let mut sanitized = String::with_capacity(input.len());

    for character in input.chars() {
        let blocked = matches!(
            character,
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' | '\0'
        );
        if blocked || character.is_control() {
            sanitized.push('_');
        } else {
            sanitized.push(character);
        }
    }

    let compact = sanitized.trim().trim_matches('.').to_string();
    if compact.is_empty() {
        "exam-paper.pdf".into()
    } else {
        compact
    }
}

fn build_query_clauses(criterion: &SearchCriterion) -> Result<Vec<String>, String> {
    let field = normalize_field(&criterion.field);
    let condition = criterion.condition.trim();
    let operator = criterion.operator.trim();

    match operator {
        "range" => {
            let min = criterion
                .value
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| "Range search requires a minimum value.".to_string())?;
            let max = criterion
                .value2
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| "Range search requires a maximum value.".to_string())?;
            Ok(vec![format!("{condition},{field},{operator},{min},{max}")])
        }
        "terms" => {
            let values = criterion
                .values
                .clone()
                .unwrap_or_default()
                .into_iter()
                .filter(|value| !value.trim().is_empty())
                .collect::<Vec<_>>();
            if values.is_empty() {
                return Err("Multi-value search requires at least one value.".into());
            }
            Ok(values
                .into_iter()
                .map(|value| format!("{condition},{field},term,{value}"))
                .collect())
        }
        _ => {
            let value = criterion
                .value
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| "Search value is required.".to_string())?;
            Ok(vec![format!("{condition},{field},{operator},{value}")])
        }
    }
}

fn build_http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
        .map_err(|error| error.to_string())
}

fn browser_user_agent() -> &'static str {
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko)"
}

fn is_authenticated_digital_gems_page(final_url: &Url, html: &str) -> bool {
    let host = final_url.host_str().unwrap_or_default();
    let path = final_url.path();
    let normalized_html = html.to_lowercase();

    let login_like_page = path.contains("/login")
        || host.contains("access.libnova.com")
        || normalized_html.contains("sign in")
        || normalized_html.contains("single sign-on")
        || normalized_html.contains("log in");

    let exam_collection_like_page = normalized_html.contains("browse/collection/31")
        || normalized_html.contains("container-result")
        || normalized_html.contains("search-field")
        || normalized_html.contains("examination papers");

    host.contains("digitalgems.nus.edu.sg") && !login_like_page && exam_collection_like_page
}

fn html_explicitly_has_no_results(html: &str) -> bool {
    let normalized_html = html.to_lowercase();
    normalized_html.contains("no results")
        || normalized_html.contains("no examination papers")
        || normalized_html.contains("0 results")
        || normalized_html.contains("sorry, we couldn’t find that")
        || normalized_html.contains("sorry, we couldn't find that")
        || (normalized_html.contains("some troubleshooting advice")
            && normalized_html.contains("have you spelled your search terms correctly"))
}

fn extract_page_title(html: &str) -> Option<String> {
    let document = Html::parse_document(html);
    let title_selector = selector("title").ok()?;
    let title = document
        .select(&title_selector)
        .next()
        .map(|node| text_content(node.text()))?;

    if title.is_empty() {
        None
    } else {
        Some(title)
    }
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

fn normalize_field(field: &str) -> String {
    if field == "parents" || field.ends_with(".keyword") {
        field.to_string()
    } else {
        format!("{field}.keyword")
    }
}

fn parse_search_results(html: &str, base_url: &Url) -> Result<Vec<ExamPaperResult>, String> {
    let document = Html::parse_document(html);
    let result_selector =
        selector(".container-result .result, .content-result .result, .col-12.result")?;
    let title_selector = selector("span.h5 a[href*='/view/']")?;
    let fallback_title_selector = selector("a[href*='/view/']")?;
    let dl_selector = selector("dl.item-list")?;
    let dt_selector = selector("dt")?;
    let dd_selector = selector("dd")?;

    let results = document
        .select(&result_selector)
        .filter_map(|node| {
            let link = node
                .select(&title_selector)
                .next()
                .or_else(|| node.select(&fallback_title_selector).next())?;
            let href = link.value().attr("href")?;
            let view_url = base_url.join(href).ok()?.to_string();
            let title = text_content(link.text());
            let mut course_code = None;
            let mut course_name = None;
            let mut year = None;
            let mut semester = None;

            for field in node.select(&dl_selector) {
                let label = field
                    .select(&dt_selector)
                    .next()
                    .map(|value| normalize_label(&text_content(value.text())));
                let value = field
                    .select(&dd_selector)
                    .next()
                    .map(|value| text_content(value.text()))
                    .filter(|value| !value.is_empty());
                match (label.as_deref(), value) {
                    (Some("course code"), Some(parsed)) => course_code = Some(parsed),
                    (Some("course name"), Some(parsed)) => course_name = Some(parsed),
                    (Some("year of examination"), Some(parsed)) => year = Some(parsed),
                    (Some("semester"), Some(parsed)) => semester = Some(parsed),
                    _ => {}
                }
            }

            if course_name.is_none() {
                course_name = Some(title.clone());
            }

            Some(ExamPaperResult {
                id: view_url.clone(),
                title,
                course_code,
                course_name,
                year,
                semester,
                view_url,
                download_url: None,
                downloadable: true,
                unavailable_reason: None,
            })
        })
        .collect::<Vec<_>>();

    Ok(results)
}

fn parse_search_facets(html: &str) -> Result<Vec<SearchFacetGroup>, String> {
    let document = Html::parse_document(html);
    let group_selector = selector(".sidebar .card[id^='facet-']")?;
    let title_selector = selector(".card-header")?;
    let value_selector = selector(".list-group a[href]")?;
    let badge_selector = selector(".badge")?;

    let facets = document
        .select(&group_selector)
        .filter_map(|group| {
            let id = group.value().attr("id")?.trim().to_string();
            let title = group
                .select(&title_selector)
                .next()
                .map(|node| text_content(node.text()))
                .filter(|value| !value.is_empty())?;

            let values = group
                .select(&value_selector)
                .filter_map(|node| {
                    let href = node.value().attr("href")?;
                    let count = node
                        .select(&badge_selector)
                        .next()
                        .map(|badge| text_content(badge.text()))
                        .and_then(|value| value.parse::<usize>().ok())?;
                    let combined_text = text_content(node.text());
                    let count_text = count.to_string();
                    let label = combined_text
                        .strip_prefix(&count_text)
                        .unwrap_or(&combined_text)
                        .trim()
                        .to_string();
                    let query_clauses = parse_facet_clauses_from_href(href)?;

                    Some(SearchFacetValue {
                        id: format!("{}::{}", id, normalize_label(&label)),
                        label,
                        count,
                        href: if let Ok(parsed) = Url::parse(href) {
                            parsed.to_string()
                        } else {
                            Url::parse(AUTH_WINDOW_URL).ok()?.join(href).ok()?.to_string()
                        },
                        query_clauses,
                    })
                })
                .collect::<Vec<_>>();

            if values.is_empty() {
                None
            } else {
                Some(SearchFacetGroup { id, title, values })
            }
        })
        .collect::<Vec<_>>();

    Ok(facets)
}

fn parse_facet_clauses_from_href(href: &str) -> Option<Vec<String>> {
    let url = if let Ok(parsed) = Url::parse(href) {
        parsed
    } else {
        let base = Url::parse(AUTH_WINDOW_URL).ok()?;
        base.join(href).ok()?
    };

    let clauses = url
        .query_pairs()
        .filter_map(|(key, value)| {
            if key == "q" {
                Some(value.into_owned())
            } else {
                None
            }
        })
        .collect::<Vec<_>>();

    if clauses.is_empty()
        || clauses
            .iter()
            .any(|clause| clause.split(',').map(str::trim).collect::<Vec<_>>().len() < 4)
    {
        return None;
    }

    Some(clauses)
}

fn apply_page_to_search_url(search_url: &str, page: usize) -> Result<Url, String> {
    let mut parsed =
        Url::parse(search_url).map_err(|error| format!("invalid search url: {error}"))?;
    let mut retained_pairs = parsed
        .query_pairs()
        .filter(|(key, _)| key != "page")
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect::<Vec<_>>();
    if page > 1 {
        retained_pairs.push(("page".to_string(), page.to_string()));
    }
    {
        let mut query = parsed.query_pairs_mut();
        query.clear();
        for (key, value) in retained_pairs {
            query.append_pair(&key, &value);
        }
    }
    Ok(parsed)
}

fn extract_total_results(facets: &[SearchFacetGroup], html: &str) -> Option<usize> {
    facets
        .iter()
        .find(|group| group.id == "facet-parents")
        .and_then(|group| group.values.first())
        .map(|value| value.count)
        .or_else(|| extract_total_results_from_pagination(html))
}

fn extract_total_results_from_pagination(html: &str) -> Option<usize> {
    let document = Html::parse_document(html);
    let current_page_selector = selector(".pagination .page-link.btn-primary-disabled").ok()?;
    let last_page_selector = selector(".pagination .last_tag_open a[data-ci-pagination-page]").ok()?;
    let result_selector = selector(".container-result .result, .content-result .result, .col-12.result").ok()?;

    let current_page = document
        .select(&current_page_selector)
        .next()
        .map(|node| text_content(node.text()))
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(1);
    let last_page = document
        .select(&last_page_selector)
        .next()
        .and_then(|node| node.value().attr("data-ci-pagination-page"))
        .and_then(|value| value.parse::<usize>().ok())?;
    let current_page_result_count = document.select(&result_selector).count();

    if last_page == current_page {
        Some((last_page.saturating_sub(1) * SEARCH_LIMIT) + current_page_result_count)
    } else {
        Some(last_page * SEARCH_LIMIT)
    }
}

async fn resolve_download_url(
    client: &reqwest::Client,
    cookie_header: &str,
    view_url: &str,
    hinted_download_url: Option<&str>,
) -> Result<String, String> {
    if let Some(url) = hinted_download_url {
        return Ok(url.to_string());
    }

    let view_url = Url::parse(view_url).map_err(|error| error.to_string())?;
    let iframe_selector = selector("iframe.viewer-iframe, iframe#First-Iframe, .container-iframe iframe")?;
    let view_html = fetch_html_document(client, cookie_header, &view_url).await?;
    let iframe_src = {
        let document = Html::parse_document(&view_html);
        document
            .select(&iframe_selector)
            .next()
            .and_then(|node| node.value().attr("src"))
            .map(str::to_string)
            .ok_or_else(|| "Viewer iframe not found on the Digital Gems paper page.".to_string())?
    };
    let iframe_url = view_url.join(&iframe_src).map_err(|error| error.to_string())?;

    let viewer_response = client
        .get(iframe_url.clone())
        .header(USER_AGENT, browser_user_agent())
        .header(COOKIE, cookie_header)
        .send()
        .await
        .map_err(|error| error.to_string())?;

    let content_type = viewer_response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_lowercase();
    let final_url = viewer_response.url().clone();

    if content_type.contains("application/pdf") {
        return Ok(final_url.to_string());
    }

    let nested_selector = selector("iframe[src], embed[src], object[data], a[href$='.pdf'], a[href*='download']")?;
    let html = viewer_response.text().await.map_err(|error| error.to_string())?;

    if let Some(pdf_url) = extract_pdf_url_from_viewer_html(&final_url, &html) {
        return Ok(pdf_url);
    }

    let nested_src = {
        let document = Html::parse_document(&html);
        document
            .select(&nested_selector)
            .next()
            .and_then(|node| {
                node.value()
                    .attr("src")
                    .or_else(|| node.value().attr("data"))
                    .or_else(|| node.value().attr("href"))
            })
            .map(str::to_string)
    };

    if let Some(src) = nested_src {
        return final_url
            .join(&src)
            .map(|url| url.to_string())
            .map_err(|error| error.to_string());
    }

    Ok(final_url.to_string())
}

fn extract_pdf_url_from_viewer_html(base_url: &Url, html: &str) -> Option<String> {
    for marker in ["var url_pdf", "PDFViewerApplication.open"] {
        let Some(start) = html.find(marker) else {
            continue;
        };
        let snippet = &html[start..];
        let equals_index = snippet.find('=');
        let open_paren_index = snippet.find('(');
        let value_start = match (equals_index, open_paren_index) {
            (Some(equals), Some(paren)) => equals.min(paren) + 1,
            (Some(equals), None) => equals + 1,
            (None, Some(paren)) => paren + 1,
            (None, None) => continue,
        };

        let remainder = snippet[value_start..].trim_start();
        let Some(quote) = remainder.chars().next().filter(|character| *character == '"' || *character == '\'') else {
            continue;
        };
        let content = &remainder[quote.len_utf8()..];
        let Some(end_offset) = content.find(quote) else {
            continue;
        };
        let candidate = content[..end_offset].trim();
        if candidate.is_empty() {
            continue;
        }

        if let Ok(url) = base_url.join(candidate) {
            return Some(url.to_string());
        }
    }

    None
}

async fn fetch_html_document(
    client: &reqwest::Client,
    cookie_header: &str,
    url: &Url,
) -> Result<String, String> {
    let response = client
        .get(url.clone())
        .header(USER_AGENT, browser_user_agent())
        .header(COOKIE, cookie_header)
        .send()
        .await
        .map_err(|error| error.to_string())?;

    if !response.status().is_success() {
        return Err(format!(
            "Digital Gems returned {} while loading the paper page.",
            response.status()
        ));
    }

    response.text().await.map_err(|error| error.to_string())
}

fn selector(value: &str) -> Result<Selector, String> {
    Selector::parse(value).map_err(|error| error.to_string())
}

fn text_content<'a>(parts: impl Iterator<Item = &'a str>) -> String {
    parts
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn normalize_label(value: &str) -> String {
    value.trim()
        .trim_end_matches(':')
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::{
        apply_page_to_search_url, extract_total_results, html_explicitly_has_no_results,
        parse_search_facets,
    };

    #[test]
    fn detects_digital_gems_empty_state_copy() {
        let html = r#"
            <div class="row">
                <div class="col-md-12 mt-5">
                    <h1 class="text-center direction-LTR font-weight-bold">Sorry, we couldn’t find that! ☹</h1>
                    <h3 class="text-center">Some troubleshooting advice:</h3>
                    <ul class="d-table mx-auto my-0">
                        <li>Have you spelled your search terms correctly?</li>
                    </ul>
                </div>
            </div>
        "#;

        assert!(html_explicitly_has_no_results(html));
    }

    #[test]
    fn ignores_regular_browse_page_without_empty_state_markers() {
        let html = r#"
            <html>
                <head><title>Digital Gems | Browse</title></head>
                <body>
                    <div class="container-result">
                        <div class="search-field">Search</div>
                    </div>
                </body>
            </html>
        "#;

        assert!(!html_explicitly_has_no_results(html));
    }

    #[test]
    fn parses_facets_and_total_result_count() {
        let html = r#"
            <div class="sidebar">
                <div class="card" id="facet-parents">
                    <button class="card-header">Collections</button>
                    <div class="list-group">
                        <a href="https://digitalgems.nus.edu.sg/browse/collection/31?q=facet,parents,equals,31&amp;q=must,metadata.Department.en.keyword,contains,Computing&amp;limit=10">
                            <span class="badge">353</span>
                            Examination Papers Database
                        </a>
                    </div>
                </div>
                <div class="card" id="facet-semester">
                    <button class="card-header">Semester</button>
                    <div class="list-group">
                        <a href="https://digitalgems.nus.edu.sg/browse/collection/31?q=facet,metadata.Semester.en.keyword,equals,1&amp;q=must,metadata.Department.en.keyword,contains,Computing&amp;limit=10">
                            <span class="badge">171</span>
                            1
                        </a>
                    </div>
                </div>
            </div>
        "#;

        let facets = parse_search_facets(html).expect("facets should parse");

        assert_eq!(facets.len(), 2);
        assert_eq!(extract_total_results(&facets, html), Some(353));
        assert_eq!(
            facets[1].values[0].query_clauses,
            vec![
                "facet,metadata.Semester.en.keyword,equals,1".to_string(),
                "must,metadata.Department.en.keyword,contains,Computing".to_string(),
            ]
        );
    }

    #[test]
    fn keeps_year_labels_intact_when_count_shares_digits() {
        let html = r#"
            <div class="sidebar">
                <div class="card" id="facet-year-of-examination">
                    <button class="card-header">Year of Examination</button>
                    <div class="list-group">
                        <a href="https://digitalgems.nus.edu.sg/browse/collection/31?q=facet,metadata.YearOfExamination.en.keyword,equals,2025%2F2026&amp;limit=10">
                            <span class="badge">2</span>
                            2025/2026
                        </a>
                        <a href="https://digitalgems.nus.edu.sg/browse/collection/31?q=facet,metadata.YearOfExamination.en.keyword,equals,2021%2F2022&amp;limit=10">
                            <span class="badge">1</span>
                            2021/2022
                        </a>
                    </div>
                </div>
            </div>
        "#;

        let facets = parse_search_facets(html).expect("facets should parse");

        assert_eq!(facets[0].values[0].label, "2025/2026");
        assert_eq!(facets[0].values[1].label, "2021/2022");
    }

    #[test]
    fn parses_all_facet_clauses_from_refinement_link() {
        let html = r#"
            <div class="sidebar">
                <div class="card" id="facet-year-of-examination">
                    <button class="card-header">Year of Examination</button>
                    <div class="list-group">
                        <a href="https://digitalgems.nus.edu.sg/browse/collection/31?q=facet,metadata.Semester.en.keyword,equals,2&amp;q=facet,metadata.YearOfExamination.en.keyword,equals,2024%2F2025&amp;q=must,metadata.CourseCode.en.keyword,contains,CS2030&amp;limit=10">
                            <span class="badge">1</span>
                            2024/2025
                        </a>
                    </div>
                </div>
            </div>
        "#;

        let facets = parse_search_facets(html).expect("facets should parse");

        assert_eq!(
            facets[0].values[0].query_clauses,
            vec![
                "facet,metadata.Semester.en.keyword,equals,2".to_string(),
                "facet,metadata.YearOfExamination.en.keyword,equals,2024/2025".to_string(),
                "must,metadata.CourseCode.en.keyword,contains,CS2030".to_string(),
            ]
        );
    }

    #[test]
    fn removes_existing_page_param_when_returning_to_first_page() {
        let url = apply_page_to_search_url(
            "https://digitalgems.nus.edu.sg/browse/collection/31?q=must,metadata.Department.en.keyword,contains,computing&page=2&limit=10",
            1,
        )
        .expect("search url should parse");

        assert_eq!(
            url.as_str(),
            "https://digitalgems.nus.edu.sg/browse/collection/31?q=must%2Cmetadata.Department.en.keyword%2Ccontains%2Ccomputing&limit=10"
        );
        assert!(url.query_pairs().all(|(key, _)| key != "page"));
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(DownloadState::default())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let state = load_main_window_state(app.handle())?;
            if let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) {
                if state.user_resized {
                    if state.is_maximized {
                        window.maximize().map_err(|error| error.to_string())?;
                    } else {
                        if let (Some(width), Some(height)) = (state.width, state.height) {
                            window
                                .set_size(Size::Physical(PhysicalSize::new(width, height)))
                                .map_err(|error| error.to_string())?;
                        }
                        if let (Some(x), Some(y)) = (state.x, state.y) {
                            window
                                .set_position(Position::Physical(PhysicalPosition::new(x, y)))
                                .map_err(|error| error.to_string())?;
                        }
                    }
                } else {
                    window.maximize().map_err(|error| error.to_string())?;
                }
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() != MAIN_WINDOW_LABEL {
                return;
            }

            if matches!(event, WindowEvent::Resized(_) | WindowEvent::Moved(_)) {
                let _ = persist_main_window_state(window);
            }
        })
        .invoke_handler(tauri::generate_handler![
            open_auth_window,
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
