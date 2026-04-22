use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use clap::{Args, Parser, Subcommand, ValueEnum};
use serde::Serialize;
use reqwest::Url;
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use tauri_appnus_pyp_lib::core::{default_limit, download_exam_paper_to_path, page_to_cursor, parse_facet_clauses_from_href, sanitize_filename, search_exam_papers_with_cookie, validate_session_cookie_header};
use tauri_appnus_pyp_lib::models::{AuthSessionStatus, SearchCriterion, SearchRequest, AUTH_WINDOW_URL};
use tauri_appnus_pyp_lib::session_store::{load_session_snapshot, save_session_snapshot, session_store_path, SessionSnapshot};

const CLI_AUTH_WINDOW_LABEL: &str = "cli-auth-window";

#[derive(Parser)]
#[command(name = "nus-exam-papers")]
#[command(about = "Search and download NUS Digital Gems examination papers")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Auth(AuthArgs),
    Search(SearchArgs),
    Refine(RefineArgs),
    Download(DownloadArgs),
}

#[derive(Args)]
struct AuthArgs {
    #[command(subcommand)]
    command: AuthCommands,
}

#[derive(Subcommand)]
enum AuthCommands {
    Status {
        #[arg(long, value_enum, default_value_t = OutputFormat::Json)]
        format: OutputFormat,
    },
    Login {
        #[arg(long)]
        cookie_header: Option<String>,
        #[arg(long)]
        cookie_file: Option<PathBuf>,
        #[arg(long, value_enum, default_value_t = OutputFormat::Json)]
        format: OutputFormat,
    },
}

#[derive(Args)]
struct SearchArgs {
    #[arg(long = "field")]
    fields: Vec<String>,
    #[arg(long = "condition")]
    conditions: Vec<String>,
    #[arg(long = "operator")]
    operators: Vec<String>,
    #[arg(long = "value")]
    values: Vec<String>,
    #[arg(long)]
    search_url: Option<String>,
    #[arg(long = "q")]
    raw_query_clauses: Vec<String>,
    #[arg(long)]
    page: Option<usize>,
    #[arg(long)]
    limit: Option<usize>,
    #[arg(long, value_enum, default_value_t = OutputFormat::Json)]
    format: OutputFormat,
}

#[derive(Args)]
struct RefineArgs {
    #[arg(long)]
    facet_href: Option<String>,
    #[arg(long = "q")]
    raw_query_clauses: Vec<String>,
    #[arg(long)]
    page: Option<usize>,
    #[arg(long)]
    limit: Option<usize>,
    #[arg(long, value_enum, default_value_t = OutputFormat::Json)]
    format: OutputFormat,
}

#[derive(Args)]
struct DownloadArgs {
    #[arg(long)]
    output_dir: PathBuf,
    #[arg(long = "view-url")]
    view_urls: Vec<String>,
    #[arg(long = "download-url")]
    download_urls: Vec<String>,
    #[arg(long = "file-name")]
    file_names: Vec<String>,
    #[arg(long, value_enum, default_value_t = OutputFormat::Json)]
    format: OutputFormat,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum OutputFormat {
    Json,
    Pretty,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AuthStatusPayload {
    ready: bool,
    current_url: String,
    message: String,
    session_store_path: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DownloadItemPayload {
    view_url: String,
    requested_name: String,
    output_path: Option<String>,
    message: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DownloadPayload {
    requested_items: usize,
    started_count: usize,
    completed_count: usize,
    failed_count: usize,
    items: Vec<DownloadItemPayload>,
}

fn main() -> ExitCode {
    match tauri::async_runtime::block_on(run(Cli::parse())) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(1)
        }
    }
}

async fn run(cli: Cli) -> Result<ExitCode, String> {
    match cli.command {
        Commands::Auth(args) => run_auth(args).await,
        Commands::Search(args) => run_search(args).await,
        Commands::Refine(args) => run_refine(args).await,
        Commands::Download(args) => run_download(args).await,
    }
}

async fn run_auth(args: AuthArgs) -> Result<ExitCode, String> {
    match args.command {
        AuthCommands::Status { format } => {
            let path = session_store_path()?;
            let payload = match load_session_snapshot() {
                Ok(snapshot) => {
                    let status = validate_session_cookie_header(&snapshot.cookie_header).await?;
                    AuthStatusPayload {
                        ready: status.ready,
                        current_url: status.current_url,
                        message: status.message,
                        session_store_path: path.to_string_lossy().to_string(),
                    }
                }
                Err(_) => AuthStatusPayload {
                    ready: false,
                    current_url: String::new(),
                    message: "No shared Digital Gems session was found. Sign in through the desktop app once or import a cookie header with `auth login --cookie-header`.".to_string(),
                    session_store_path: path.to_string_lossy().to_string(),
                },
            };

            print_payload(format, &payload)?;
            Ok(if payload.ready { ExitCode::SUCCESS } else { ExitCode::from(1) })
        }
        AuthCommands::Login {
            cookie_header,
            cookie_file,
            format,
        } => {
            let payload = if let Some(cookie_header) =
                load_cookie_header_input(cookie_header, cookie_file)?
            {
                import_cookie_header(cookie_header).await?
            } else {
                run_interactive_login()?
            };

            print_payload(format, &payload)?;
            Ok(if payload.ready { ExitCode::SUCCESS } else { ExitCode::from(1) })
        }
    }
}

async fn run_search(args: SearchArgs) -> Result<ExitCode, String> {
    let criteria = build_criteria(&args.fields, &args.conditions, &args.operators, &args.values)?;
    let limit = default_limit(args.limit);
    let page = args.page.unwrap_or(1);
    let cookie_header = require_cookie_header()?;
    let response = search_exam_papers_with_cookie(
        &cookie_header,
        SearchRequest {
            criteria,
            search_url: args.search_url,
            raw_query_clauses: (!args.raw_query_clauses.is_empty()).then_some(args.raw_query_clauses),
            facet_clauses: None,
            cursor: page_to_cursor(page, limit),
            limit: Some(limit),
        },
    )
    .await?;

    print_payload(args.format, &response)?;
    Ok(ExitCode::SUCCESS)
}

async fn run_refine(args: RefineArgs) -> Result<ExitCode, String> {
    let limit = default_limit(args.limit);
    let page = args.page.unwrap_or(1);
    let raw_query_clauses = if let Some(facet_href) = args.facet_href {
        parse_facet_clauses_from_href(&facet_href)
            .ok_or_else(|| "Unable to parse q clauses from facet href.".to_string())?
    } else if !args.raw_query_clauses.is_empty() {
        args.raw_query_clauses
    } else {
        return Err("Provide --facet-href or at least one --q clause.".to_string());
    };

    let cookie_header = require_cookie_header()?;
    let response = search_exam_papers_with_cookie(
        &cookie_header,
        SearchRequest {
            criteria: Vec::new(),
            search_url: None,
            raw_query_clauses: Some(raw_query_clauses),
            facet_clauses: None,
            cursor: page_to_cursor(page, limit),
            limit: Some(limit),
        },
    )
    .await?;

    print_payload(args.format, &response)?;
    Ok(ExitCode::SUCCESS)
}

async fn run_download(args: DownloadArgs) -> Result<ExitCode, String> {
    if args.view_urls.is_empty() {
        return Err("Provide at least one --view-url.".to_string());
    }

    let cookie_header = require_cookie_header()?;
    let mut items = Vec::with_capacity(args.view_urls.len());
    let mut completed_count = 0usize;

    for (index, view_url) in args.view_urls.iter().enumerate() {
        let requested_name = args
            .file_names
            .get(index)
            .cloned()
            .unwrap_or_else(|| fallback_file_name(view_url));
        let hinted_download_url = args.download_urls.get(index).map(String::as_str);

        match download_exam_paper_to_path(
            &cookie_header,
            view_url,
            hinted_download_url,
            &args.output_dir.to_string_lossy(),
            &requested_name,
        )
        .await
        {
            Ok(output_path) => {
                completed_count += 1;
                items.push(DownloadItemPayload {
                    view_url: view_url.clone(),
                    requested_name,
                    output_path: Some(output_path),
                    message: None,
                });
            }
            Err(message) => {
                items.push(DownloadItemPayload {
                    view_url: view_url.clone(),
                    requested_name,
                    output_path: None,
                    message: Some(message),
                });
            }
        }
    }

    let payload = DownloadPayload {
        requested_items: args.view_urls.len(),
        started_count: args.view_urls.len(),
        completed_count,
        failed_count: args.view_urls.len().saturating_sub(completed_count),
        items,
    };

    print_payload(args.format, &payload)?;
    Ok(if payload.failed_count == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    })
}

fn build_criteria(
    fields: &[String],
    conditions: &[String],
    operators: &[String],
    values: &[String],
) -> Result<Vec<SearchCriterion>, String> {
    if fields.is_empty() {
        return Ok(Vec::new());
    }
    if operators.len() != fields.len() || values.len() != fields.len() {
        return Err("Each --field requires a matching --operator and --value.".to_string());
    }
    if !conditions.is_empty() && conditions.len() != fields.len() {
        return Err("If provided, --condition must appear once per --field.".to_string());
    }

    Ok(fields
        .iter()
        .enumerate()
        .map(|(index, field)| SearchCriterion {
            field: field.clone(),
            condition: conditions
                .get(index)
                .cloned()
                .unwrap_or_else(|| "must".to_string()),
            operator: operators[index].clone(),
            value: Some(values[index].clone()),
            value2: None,
            values: None,
        })
        .collect())
}

fn require_cookie_header() -> Result<String, String> {
    let snapshot = load_session_snapshot().map_err(|_| {
        "No shared Digital Gems session was found. Sign in through the desktop app once or import a cookie header with `auth login --cookie-header`.".to_string()
    })?;
    Ok(snapshot.cookie_header)
}

fn load_cookie_header_input(
    cookie_header: Option<String>,
    cookie_file: Option<PathBuf>,
) -> Result<Option<String>, String> {
    match (cookie_header, cookie_file) {
        (Some(_), Some(_)) => Err(
            "Use either --cookie-header or --cookie-file, not both.".to_string(),
        ),
        (Some(value), None) => {
            if value.trim() == "-" {
                let mut buffer = String::new();
                std::io::stdin()
                    .read_line(&mut buffer)
                    .map_err(|error| error.to_string())?;
                Ok(Some(normalize_cookie_header(&buffer)))
            } else {
                Ok(Some(normalize_cookie_header(&value)))
            }
        }
        (None, Some(path)) => {
            let contents = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
            Ok(Some(normalize_cookie_header(&contents)))
        }
        (None, None) => Ok(None),
    }
}

fn normalize_cookie_header(input: &str) -> String {
    input
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .trim_end_matches(';')
        .trim()
        .to_string()
}

async fn import_cookie_header(cookie_header: String) -> Result<AuthStatusPayload, String> {
    let normalized = normalize_cookie_header(&cookie_header);
    if normalized.is_empty() {
        return Err("Cookie header cannot be empty.".to_string());
    }

    let snapshot = SessionSnapshot::new(
        normalized.clone(),
        AUTH_WINDOW_URL.to_string(),
    );
    save_session_snapshot(&snapshot)?;
    let status = validate_session_cookie_header(&snapshot.cookie_header).await?;

    Ok(AuthStatusPayload {
        ready: status.ready,
        current_url: status.current_url,
        message: status.message,
        session_store_path: session_store_path()?.to_string_lossy().to_string(),
    })
}

fn run_interactive_login() -> Result<AuthStatusPayload, String> {
    println!(
        "Opening a Digital Gems login window. Complete the sign-in flow there. The CLI will close the window and save the session once login is ready."
    );

    #[cfg(target_os = "windows")]
    std::env::set_var(
        "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS",
        "--disable-logging --log-level=3",
    );

    let login_url: Url = AUTH_WINDOW_URL
        .parse()
        .map_err(|error| format!("invalid auth url: {error}"))?;
    let state = Arc::new(Mutex::new(None::<Result<AuthStatusPayload, String>>));
    let state_for_setup = Arc::clone(&state);
    let state_for_events = Arc::clone(&state);

    let app = tauri::Builder::default()
        .setup(move |app| {
            WebviewWindowBuilder::new(
                app,
                CLI_AUTH_WINDOW_LABEL,
                WebviewUrl::External(login_url.clone()),
            )
            .title("Digital Gems CLI Login")
            .inner_size(1180.0, 860.0)
            .resizable(true)
            .center()
            .build()?;

            let app_handle = app.handle().clone();
            let state_for_task = Arc::clone(&state_for_setup);

            tauri::async_runtime::spawn(async move {
                let validation_url: Url = match AUTH_WINDOW_URL.parse() {
                    Ok(url) => url,
                    Err(error) => {
                        *state_for_task.lock().expect("state poisoned") =
                            Some(Err(format!("invalid auth url: {error}")));
                        if let Some(window) = app_handle.get_webview_window(CLI_AUTH_WINDOW_LABEL) {
                            let _ = window.close();
                        } else {
                            app_handle.exit(1);
                        }
                        return;
                    }
                };

                loop {
                    let Some(window) = app_handle.get_webview_window(CLI_AUTH_WINDOW_LABEL) else {
                        let mut guard = state_for_task.lock().expect("state poisoned");
                        if guard.is_none() {
                            *guard = Some(Err(
                                "Login window was closed before a valid Digital Gems session was established."
                                    .to_string(),
                            ));
                        }
                        return;
                    };

                    let current_url = window
                        .url()
                        .map(|url| url.to_string())
                        .unwrap_or_else(|_| AUTH_WINDOW_URL.to_string());

                    if let Ok(cookies) = window.cookies_for_url(validation_url.clone()) {
                        if !cookies.is_empty() {
                            let cookie_header = cookies
                                .into_iter()
                                .map(|cookie| format!("{}={}", cookie.name(), cookie.value()))
                                .collect::<Vec<_>>()
                                .join("; ");

                            match validate_session_cookie_header(&cookie_header).await {
                                Ok(status) if status.ready => {
                                    let snapshot = SessionSnapshot::new(cookie_header, current_url);
                                    let result = save_session_snapshot(&snapshot)
                                        .and_then(|_| build_auth_payload(status));
                                    *state_for_task.lock().expect("state poisoned") =
                                        Some(result);
                                    let _ = window.close();
                                    return;
                                }
                                Ok(_) => {}
                                Err(_) => {}
                            }
                        }
                    }

                    std::thread::sleep(Duration::from_millis(1200));
                }
            });

            Ok(())
        })
        .on_window_event(move |window, event| {
            if window.label() != CLI_AUTH_WINDOW_LABEL {
                return;
            }

            if matches!(event, WindowEvent::Destroyed) {
                let mut guard = state_for_events.lock().expect("state poisoned");
                if guard.is_none() {
                    *guard = Some(Err(
                        "Login window was closed before a valid Digital Gems session was established."
                            .to_string(),
                    ));
                }
                let exit_code = if matches!(guard.as_ref(), Some(Ok(_))) { 0 } else { 1 };
                window.app_handle().exit(exit_code);
            }
        })
        .build(tauri::generate_context!())
        .map_err(|error| error.to_string())?;

    app.run(|_, _| {});

    let result = state
        .lock()
        .map_err(|_| "Login state lock poisoned.".to_string())?
        .take()
        .unwrap_or_else(|| {
            Err("Interactive login ended before a session result was captured.".to_string())
        })?;

    Ok(result)
}

fn build_auth_payload(status: AuthSessionStatus) -> Result<AuthStatusPayload, String> {
    Ok(AuthStatusPayload {
        ready: status.ready,
        current_url: status.current_url,
        message: status.message,
        session_store_path: session_store_path()?.to_string_lossy().to_string(),
    })
}

fn fallback_file_name(view_url: &str) -> String {
    let tail = view_url
        .rsplit('/')
        .next()
        .filter(|value| !value.is_empty())
        .unwrap_or("exam-paper");
    format!("{}.pdf", sanitize_filename(tail))
}

fn print_payload<T: Serialize>(format: OutputFormat, payload: &T) -> Result<(), String> {
    match format {
        OutputFormat::Json => {
            println!(
                "{}",
                serde_json::to_string_pretty(payload).map_err(|error| error.to_string())?
            );
        }
        OutputFormat::Pretty => {
            println!(
                "{}",
                serde_json::to_string_pretty(payload).map_err(|error| error.to_string())?
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{build_criteria, normalize_cookie_header};

    #[test]
    fn builds_criteria_from_parallel_cli_flags() {
        let criteria = build_criteria(
            &["metadata.CourseCode.en".to_string()],
            &[],
            &["contains".to_string()],
            &["CS2030".to_string()],
        )
        .expect("criteria should build");

        assert_eq!(criteria.len(), 1);
        assert_eq!(criteria[0].condition, "must");
        assert_eq!(criteria[0].operator, "contains");
        assert_eq!(criteria[0].value.as_deref(), Some("CS2030"));
    }

    #[test]
    fn normalizes_imported_cookie_headers() {
        assert_eq!(
            normalize_cookie_header("  foo=bar; baz=qux; \n"),
            "foo=bar; baz=qux"
        );
        assert_eq!(
            normalize_cookie_header("foo=bar;\n\nbaz=qux"),
            "foo=bar; baz=qux"
        );
    }
}
