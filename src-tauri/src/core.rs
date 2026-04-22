use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use reqwest::header::{COOKIE, USER_AGENT};
use reqwest::Url;
use scraper::{Html, Selector};

use crate::models::{
    AuthSessionStatus, ExamPaperResult, SearchCriterion, SearchFacetGroup, SearchFacetValue,
    SearchRequest, SearchResponse, AUTH_WINDOW_URL, SEARCH_LIMIT,
};

pub fn browser_user_agent() -> &'static str {
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko)"
}

pub fn default_limit(limit: Option<usize>) -> usize {
    limit.filter(|value| *value > 0).unwrap_or(SEARCH_LIMIT)
}

pub fn page_to_cursor(page: usize, limit: usize) -> Option<String> {
    if page > 1 {
        Some(((page - 1) * limit).to_string())
    } else {
        None
    }
}

pub async fn validate_session_cookie_header(cookie_header: &str) -> Result<AuthSessionStatus, String> {
    let collection_url: Url = AUTH_WINDOW_URL
        .parse()
        .map_err(|error| format!("invalid collection url: {error}"))?;
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
    let message = if ready {
        "Saved Digital Gems session loaded. Search and downloads are ready.".to_string()
    } else {
        "Your Digital Gems session is missing or expired. Sign in to continue.".to_string()
    };

    Ok(AuthSessionStatus {
        ready,
        current_url: final_url.to_string(),
        message,
    })
}

pub async fn search_exam_papers_with_cookie(
    cookie_header: &str,
    request: SearchRequest,
) -> Result<SearchResponse, String> {
    let collection_url: Url = AUTH_WINDOW_URL
        .parse()
        .map_err(|error| format!("invalid collection url: {error}"))?;

    let limit = default_limit(request.limit);
    let offset = request
        .cursor
        .as_deref()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    let page = (offset / limit) + 1;

    let url = if let Some(search_url) = request.search_url.clone() {
        apply_page_and_limit_to_search_url(&search_url, page, limit)?
    } else {
        let mut built = collection_url;
        let mut query = built.query_pairs_mut();
        query.append_pair("limit", &limit.to_string());
        if page > 1 {
            query.append_pair("page", &page.to_string());
        }
        if let Some(raw_query_clauses) = request.raw_query_clauses.clone() {
            for clause in raw_query_clauses {
                query.append_pair("q", &clause);
            }
        } else {
            for clause in request.facet_clauses.clone().unwrap_or_default() {
                query.append_pair("q", &clause);
            }
            for criterion in &request.criteria {
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
        .header(USER_AGENT, browser_user_agent())
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
    let total_results = extract_total_results(&facets, &html, limit);
    if results.is_empty() && !html_explicitly_has_no_results(&html) {
        let page_title = extract_page_title(&html).unwrap_or_else(|| "Unknown page".to_string());
        return Err(format!(
            "Digital Gems returned an unexpected page shape while searching. Final URL: {}. Page title: {}.",
            final_url, page_title
        ));
    }

    let has_more = total_results
        .map(|total| offset + results.len() < total)
        .unwrap_or(results.len() == limit);
    let next_cursor = has_more.then(|| (offset + limit).to_string());
    let raw_query_clauses = extract_q_clauses_from_url(&final_url);
    let page_count = total_results.map(|total| total.div_ceil(limit));

    Ok(SearchResponse {
        results,
        total_results,
        facets,
        search_url: Some(final_url.to_string()),
        raw_query_clauses,
        cursor: next_cursor,
        has_more,
        page,
        page_size: limit,
        page_count,
        session_ready: true,
    })
}

pub async fn resolve_download_url(
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

pub async fn download_exam_paper_to_path(
    cookie_header: &str,
    view_url: &str,
    hinted_download_url: Option<&str>,
    destination_directory: &str,
    requested_name: &str,
) -> Result<String, String> {
    let client = build_http_client()?;
    let download_url = resolve_download_url(&client, cookie_header, view_url, hinted_download_url).await?;
    let target_path =
        prepare_download_path(destination_directory.to_string(), requested_name.to_string())?;

    let mut response = client
        .get(download_url)
        .header(USER_AGENT, browser_user_agent())
        .header(COOKIE, cookie_header)
        .send()
        .await
        .map_err(|error| error.to_string())?;

    if !response.status().is_success() {
        return Err(format!(
            "Digital Gems returned {} while downloading the PDF.",
            response.status()
        ));
    }

    let mut file = fs::File::create(&target_path).map_err(|error| error.to_string())?;
    while let Some(chunk) = response.chunk().await.map_err(|error| error.to_string())? {
        file.write_all(&chunk).map_err(|error| error.to_string())?;
    }
    file.flush().map_err(|error| error.to_string())?;

    Ok(target_path)
}

pub fn sanitize_filename(input: &str) -> String {
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

pub fn prepare_download_path(directory: String, requested_name: String) -> Result<String, String> {
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
        .unwrap_or_else(|| ".pdf".to_string());

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

pub fn build_query_clauses(criterion: &SearchCriterion) -> Result<Vec<String>, String> {
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

pub fn build_http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
        .map_err(|error| error.to_string())
}

pub fn is_authenticated_digital_gems_page(final_url: &Url, html: &str) -> bool {
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

pub fn html_explicitly_has_no_results(html: &str) -> bool {
    let normalized_html = html.to_lowercase();
    normalized_html.contains("no results")
        || normalized_html.contains("no examination papers")
        || normalized_html.contains("0 results")
        || normalized_html.contains("sorry, we couldn’t find that")
        || normalized_html.contains("sorry, we couldn't find that")
        || (normalized_html.contains("some troubleshooting advice")
            && normalized_html.contains("have you spelled your search terms correctly"))
}

pub fn extract_page_title(html: &str) -> Option<String> {
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

pub fn normalize_field(field: &str) -> String {
    if field == "parents" || field.ends_with(".keyword") {
        field.to_string()
    } else {
        format!("{field}.keyword")
    }
}

pub fn parse_search_results(html: &str, base_url: &Url) -> Result<Vec<ExamPaperResult>, String> {
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

pub fn parse_search_facets(html: &str) -> Result<Vec<SearchFacetGroup>, String> {
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

pub fn parse_facet_clauses_from_href(href: &str) -> Option<Vec<String>> {
    let url = if let Ok(parsed) = Url::parse(href) {
        parsed
    } else {
        let base = Url::parse(AUTH_WINDOW_URL).ok()?;
        base.join(href).ok()?
    };

    let clauses = extract_q_clauses_from_url(&url);

    if clauses.is_empty()
        || clauses
            .iter()
            .any(|clause| clause.split(',').map(str::trim).collect::<Vec<_>>().len() < 4)
    {
        return None;
    }

    Some(clauses)
}

pub fn apply_page_and_limit_to_search_url(
    search_url: &str,
    page: usize,
    limit: usize,
) -> Result<Url, String> {
    let mut parsed =
        Url::parse(search_url).map_err(|error| format!("invalid search url: {error}"))?;
    let mut retained_pairs = parsed
        .query_pairs()
        .filter(|(key, _)| key != "page" && key != "limit")
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect::<Vec<_>>();
    retained_pairs.push(("limit".to_string(), limit.to_string()));
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

pub fn extract_total_results(
    facets: &[SearchFacetGroup],
    html: &str,
    limit: usize,
) -> Option<usize> {
    facets
        .iter()
        .find(|group| group.id == "facet-parents")
        .and_then(|group| group.values.first())
        .map(|value| value.count)
        .or_else(|| extract_total_results_from_pagination(html, limit))
}

pub fn extract_total_results_from_pagination(html: &str, limit: usize) -> Option<usize> {
    let document = Html::parse_document(html);
    let current_page_selector = selector(".pagination .page-link.btn-primary-disabled").ok()?;
    let last_page_selector = selector(".pagination .last_tag_open a[data-ci-pagination-page]").ok()?;
    let result_selector =
        selector(".container-result .result, .content-result .result, .col-12.result").ok()?;

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
        Some((last_page.saturating_sub(1) * limit) + current_page_result_count)
    } else {
        Some(last_page * limit)
    }
}

pub fn extract_q_clauses_from_url(url: &Url) -> Vec<String> {
    url.query_pairs()
        .filter_map(|(key, value)| if key == "q" { Some(value.into_owned()) } else { None })
        .collect()
}

pub async fn fetch_html_document(
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

pub fn extract_pdf_url_from_viewer_html(base_url: &Url, html: &str) -> Option<String> {
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
        let Some(quote) = remainder
            .chars()
            .next()
            .filter(|character| *character == '"' || *character == '\'')
        else {
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

pub fn selector(value: &str) -> Result<Selector, String> {
    Selector::parse(value).map_err(|error| error.to_string())
}

pub fn text_content<'a>(parts: impl Iterator<Item = &'a str>) -> String {
    parts
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn normalize_label(value: &str) -> String {
    value
        .trim()
        .trim_end_matches(':')
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::{
        apply_page_and_limit_to_search_url, build_query_clauses, extract_total_results,
        html_explicitly_has_no_results, page_to_cursor, parse_search_facets,
    };
    use crate::models::SearchCriterion;

    #[test]
    fn builds_cursor_from_page_and_limit() {
        assert_eq!(page_to_cursor(1, 10), None);
        assert_eq!(page_to_cursor(3, 10), Some("20".to_string()));
    }

    #[test]
    fn updates_limit_when_replaying_search_url() {
        let url = apply_page_and_limit_to_search_url(
            "https://digitalgems.nus.edu.sg/browse/collection/31?q=must,metadata.Department.en.keyword,contains,computing&page=2&limit=10",
            1,
            25,
        )
        .expect("search url should parse");

        assert_eq!(
            url.as_str(),
            "https://digitalgems.nus.edu.sg/browse/collection/31?q=must%2Cmetadata.Department.en.keyword%2Ccontains%2Ccomputing&limit=25"
        );
        assert!(url.query_pairs().all(|(key, _)| key != "page"));
    }

    #[test]
    fn builds_search_clause_for_contains_queries() {
        let clauses = build_query_clauses(&SearchCriterion {
            field: "metadata.CourseCode.en".to_string(),
            condition: "must".to_string(),
            operator: "contains".to_string(),
            value: Some("CS2030".to_string()),
            value2: None,
            values: None,
        })
        .expect("criterion should build");

        assert_eq!(
            clauses,
            vec!["must,metadata.CourseCode.en.keyword,contains,CS2030".to_string()]
        );
    }

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
        assert_eq!(extract_total_results(&facets, html, 10), Some(353));
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
}
