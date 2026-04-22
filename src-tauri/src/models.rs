use serde::{Deserialize, Serialize};

pub const AUTH_WINDOW_URL: &str = "https://digitalgems.nus.edu.sg/browse/collection/31";
pub const SEARCH_LIMIT: usize = 10;
pub const APP_IDENTIFIER: &str = "com.coldspot.nuspyp";
pub const SESSION_STORE_FILE: &str = "digital-gems-session.json";

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchCriterion {
    pub field: String,
    pub condition: String,
    pub operator: String,
    pub value: Option<String>,
    pub value2: Option<String>,
    pub values: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchRequest {
    #[serde(default)]
    pub criteria: Vec<SearchCriterion>,
    pub search_url: Option<String>,
    pub raw_query_clauses: Option<Vec<String>>,
    pub facet_clauses: Option<Vec<String>>,
    pub cursor: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExamPaperResult {
    pub id: String,
    pub title: String,
    pub course_code: Option<String>,
    pub course_name: Option<String>,
    pub year: Option<String>,
    pub semester: Option<String>,
    pub view_url: String,
    pub download_url: Option<String>,
    pub downloadable: bool,
    pub unavailable_reason: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResponse {
    pub results: Vec<ExamPaperResult>,
    pub total_results: Option<usize>,
    pub facets: Vec<SearchFacetGroup>,
    pub search_url: Option<String>,
    pub raw_query_clauses: Vec<String>,
    pub cursor: Option<String>,
    pub has_more: bool,
    pub page: usize,
    pub page_size: usize,
    pub page_count: Option<usize>,
    pub session_ready: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchFacetGroup {
    pub id: String,
    pub title: String,
    pub values: Vec<SearchFacetValue>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchFacetValue {
    pub id: String,
    pub label: String,
    pub count: usize,
    pub href: String,
    pub query_clauses: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthSessionStatus {
    pub ready: bool,
    pub current_url: String,
    pub message: String,
}
