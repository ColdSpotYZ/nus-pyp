export type SearchField =
  | "metadata.title.en"
  | "metadata.Department.en"
  | "metadata.CourseCode.en"
  | "metadata.CourseName.en"
  | "metadata.YearOfExamination.en"
  | "metadata.AcademicLevel.en"
  | "metadata.Semester.en";

export type SearchCondition = "must" | "must_not" | "should";

export type SearchOperator =
  | "contains"
  | "term"
  | "terms"
  | "regexp"
  | "range"
  | "starts_with"
  | "ends_with";

export interface SearchCriterion {
  field: SearchField;
  condition: SearchCondition;
  operator: SearchOperator;
  value?: string;
  value2?: string;
  values?: string[];
}

export interface SearchRequest {
  criteria: SearchCriterion[];
  cursor?: string | null;
}

export interface ExamPaperResult {
  id: string;
  title: string;
  courseCode?: string;
  courseName?: string;
  year?: string;
  semester?: string;
  viewUrl: string;
  downloadUrl?: string;
  downloadable: boolean;
  unavailableReason?: string;
}

export interface SearchResponse {
  results: ExamPaperResult[];
  cursor?: string | null;
  hasMore: boolean;
}

export type DownloadJobState =
  | "queued"
  | "running"
  | "completed"
  | "failed"
  | "cancelled";

export interface DownloadJob {
  id: string;
  resultId: string;
  filename: string;
  destinationPath: string;
  state: DownloadJobState;
  bytesReceived: number;
  bytesTotal?: number;
  progressPercent?: number;
  errorMessage?: string;
  resultSnapshot: ExamPaperResult;
}

export interface DownloadProgressPayload {
  jobId: string;
  bytesReceived: number;
  bytesTotal?: number;
  progressPercent?: number;
}

export interface DownloadCompletedPayload {
  jobId: string;
  destinationPath: string;
}

export interface DownloadFailedPayload {
  jobId: string;
  message: string;
  cancelled?: boolean;
}

export interface AuthMessagePayload {
  message?: string;
}

export interface AppEventMap {
  "auth:login-ready": null;
  "auth:login-required": AuthMessagePayload;
  "search:page-loaded": SearchResponse;
  "download:progress": DownloadProgressPayload;
  "download:completed": DownloadCompletedPayload;
  "download:failed": DownloadFailedPayload;
}
