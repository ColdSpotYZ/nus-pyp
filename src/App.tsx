import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import "./App.css";
import {
  DEFAULT_CRITERION,
  FIELD_OPTIONS,
  OPERATOR_OPTIONS,
  CONDITION_OPTIONS,
} from "./constants";
import type {
  AppEventMap,
  DownloadJob,
  DownloadJobState,
  ExamPaperResult,
  SearchCriterion,
  SearchResponse,
} from "./types";

const MAX_CONCURRENT_DOWNLOADS = 3;

interface AuthSessionStatus {
  ready: boolean;
  currentUrl: string;
  message: string;
}

function makeCriterion(): SearchCriterion {
  return {
    ...DEFAULT_CRITERION,
    values: [],
  };
}

function App() {
  const [criteria, setCriteria] = useState<SearchCriterion[]>([makeCriterion()]);
  const [results, setResults] = useState<ExamPaperResult[]>([]);
  const [selectedResultIds, setSelectedResultIds] = useState<string[]>([]);
  const [downloadJobs, setDownloadJobs] = useState<DownloadJob[]>([]);
  const [authState, setAuthState] = useState<"unknown" | "required" | "ready">(
    "unknown",
  );
  const [isOpeningAuth, setIsOpeningAuth] = useState(false);
  const [isCheckingAuth, setIsCheckingAuth] = useState(false);
  const [isSearching, setIsSearching] = useState(false);
  const [hasMoreResults, setHasMoreResults] = useState(false);
  const [searchCursor, setSearchCursor] = useState<string | null>(null);
  const [statusMessage, setStatusMessage] = useState(
    "Checking for an existing Digital Gems session.",
  );
  const [resultError, setResultError] = useState<string | null>(null);
  const searchTimeoutRef = useRef<number | null>(null);
  const runningJobsRef = useRef<Set<string>>(new Set());
  const downloadDestinationRef = useRef<string | null>(null);
  const authPollRef = useRef<number | null>(null);

  const allLoadedSelected =
    results.length > 0 && results.every((result) => selectedResultIds.includes(result.id));

  const selectedResults = useMemo(
    () => results.filter((result) => selectedResultIds.includes(result.id)),
    [results, selectedResultIds],
  );

  useEffect(() => {
    void bootstrapAuthSession();

    const unlisteners: Promise<UnlistenFn>[] = [
      listen<AppEventMap["auth:login-ready"]>("auth:login-ready", () => {
        setAuthState("ready");
        setIsOpeningAuth(false);
        setIsCheckingAuth(false);
        setStatusMessage(
          "Authenticated session detected. Search and downloads now run through a hidden background session.",
        );
      }),
      listen<AppEventMap["auth:login-required"]>("auth:login-required", (event) => {
        setAuthState("required");
        setIsOpeningAuth(false);
        setIsCheckingAuth(false);
        setStatusMessage(
          event.payload?.message ??
            "The auth window is not logged in yet. Complete the Digital Gems sign-in flow and confirm again.",
        );
      }),
      listen<AppEventMap["search:page-loaded"]>("search:page-loaded", (event) => {
        const payload = event.payload as SearchResponse;
        clearSearchTimeout();
        setIsSearching(false);
        setResultError(null);
        setStatusMessage(
          payload.results.length > 0
            ? `Loaded ${payload.results.length} result${payload.results.length === 1 ? "" : "s"} from Digital Gems.`
            : "Search completed but no examination papers were found for the current filters.",
        );
        setResults((current) => mergeResults(current, payload.results));
        setHasMoreResults(payload.hasMore);
        setSearchCursor(payload.cursor ?? null);
      }),
      listen<AppEventMap["download:progress"]>("download:progress", (event) => {
        const payload = event.payload;
        setDownloadJobs((current) =>
          current.map((job) =>
            job.id === payload.jobId
              ? {
                  ...job,
                  state: "running",
                  bytesReceived: payload.bytesReceived,
                  bytesTotal: payload.bytesTotal,
                  progressPercent: payload.progressPercent,
                  errorMessage: undefined,
                }
              : job,
          ),
        );
      }),
      listen<AppEventMap["download:completed"]>("download:completed", (event) => {
        const payload = event.payload;
        runningJobsRef.current.delete(payload.jobId);
        setDownloadJobs((current) =>
          current.map((job) =>
            job.id === payload.jobId
              ? {
                  ...job,
                  state: "completed",
                  progressPercent: 100,
                  bytesReceived: job.bytesTotal ?? job.bytesReceived,
                  destinationPath: payload.destinationPath,
                  errorMessage: undefined,
                }
              : job,
          ),
        );
      }),
      listen<AppEventMap["download:failed"]>("download:failed", (event) => {
        const payload = event.payload;
        runningJobsRef.current.delete(payload.jobId);
        setDownloadJobs((current) =>
          current.map((job) =>
            job.id === payload.jobId
              ? {
                  ...job,
                  state: payload.cancelled ? "cancelled" : "failed",
                  errorMessage: payload.message,
                }
              : job,
          ),
        );
      }),
    ];

    return () => {
      void Promise.all(unlisteners).then((resolved) => {
        resolved.forEach((unlisten) => unlisten());
      });
      clearSearchTimeout();
      stopAuthPolling();
    };
  }, []);

  useEffect(() => {
    if (authState !== "ready") {
      return;
    }

    const runningCount = downloadJobs.filter((job) => job.state === "running").length;
    const availableSlots = Math.max(0, MAX_CONCURRENT_DOWNLOADS - runningCount);
    if (availableSlots === 0) {
      return;
    }

    const pendingJobs = downloadJobs.filter(
      (job) => job.state === "queued" && !runningJobsRef.current.has(job.id),
    );
    if (pendingJobs.length === 0) {
      return;
    }

    pendingJobs.slice(0, availableSlots).forEach((job) => {
      runningJobsRef.current.add(job.id);
      setDownloadJobs((current) =>
        current.map((entry) =>
          entry.id === job.id
            ? {
                ...entry,
                state: "running",
                progressPercent: 0,
                errorMessage: undefined,
              }
            : entry,
        ),
      );
      void invoke("start_download", {
        request: {
          jobId: job.id,
          destinationDirectory: job.destinationPath,
          requestedName: job.filename,
          viewUrl: job.resultSnapshot.viewUrl,
          downloadUrl: job.resultSnapshot.downloadUrl,
        },
      }).catch((error) => {
        runningJobsRef.current.delete(job.id);
        setDownloadJobs((current) =>
          current.map((entry) =>
            entry.id === job.id
              ? {
                  ...entry,
                  state: "failed",
                  errorMessage: formatError(error),
                }
              : entry,
          ),
        );
      });
    });
  }, [authState, downloadJobs, results]);

  function clearSearchTimeout() {
    if (searchTimeoutRef.current !== null) {
      window.clearTimeout(searchTimeoutRef.current);
      searchTimeoutRef.current = null;
    }
  }

  function scheduleSearchTimeout() {
    clearSearchTimeout();
    searchTimeoutRef.current = window.setTimeout(() => {
      setIsSearching(false);
      setResultError(
        "Digital Gems did not answer in time. Confirm the auth window is logged in and visible, then try again.",
      );
    }, 20000);
  }

  function stopAuthPolling() {
    if (authPollRef.current !== null) {
      window.clearInterval(authPollRef.current);
      authPollRef.current = null;
    }
  }

  async function bootstrapAuthSession() {
    setIsCheckingAuth(true);
    try {
      const status = await invoke<AuthSessionStatus>("bootstrap_auth_session");
      setAuthState(status.ready ? "ready" : "required");
      setStatusMessage(status.message);
      setResultError(null);
    } catch (error) {
      setAuthState("required");
      setResultError(formatError(error));
    } finally {
      setIsCheckingAuth(false);
    }
  }

  async function openAuthWindow() {
    setIsOpeningAuth(true);
    setResultError(null);
    try {
      await invoke("open_auth_window");
      setStatusMessage(
        "Sign in through the Digital Gems window. The app will detect success automatically and return to the workspace.",
      );
      stopAuthPolling();
      authPollRef.current = window.setInterval(() => {
        void confirmSession(false, true);
      }, 1500);
    } catch (error) {
      setResultError(formatError(error));
    } finally {
      setIsOpeningAuth(false);
    }
  }

  async function confirmSession(autoClose = true, silent = false) {
    if (!silent) {
      setIsCheckingAuth(true);
      setResultError(null);
    }
    try {
      const status = await invoke<AuthSessionStatus>("confirm_auth_session", {
        autoClose,
      });
      setAuthState(status.ready ? "ready" : "required");
      setStatusMessage(status.message);
      if (status.ready) {
        stopAuthPolling();
        if (!autoClose) {
          await invoke("hide_auth_window");
        }
        setResultError(null);
      } else if (!silent) {
        setResultError(null);
      }
    } catch (error) {
      if (!silent) {
        setResultError(formatError(error));
      }
    } finally {
      if (!silent) {
        setIsCheckingAuth(false);
      }
    }
  }

  async function openPaper(result: ExamPaperResult) {
    setResultError(null);
    try {
      await invoke("show_auth_window", {
        url: result.viewUrl,
      });
      setStatusMessage(`Opened ${result.title} in the Digital Gems session window.`);
    } catch (error) {
      setResultError(formatError(error));
    }
  }

  function updateCriterion(index: number, next: Partial<SearchCriterion>) {
    setCriteria((current) =>
      current.map((criterion, criterionIndex) =>
        criterionIndex === index ? { ...criterion, ...next } : criterion,
      ),
    );
  }

  function handleOperatorChange(index: number, operator: SearchCriterion["operator"]) {
    if (operator === "range") {
      updateCriterion(index, { operator, value: "", value2: "", values: [] });
      return;
    }
    if (operator === "terms") {
      updateCriterion(index, { operator, value: "", value2: "", values: [""] });
      return;
    }
    updateCriterion(index, { operator, value2: "", values: [] });
  }

  function addCriterion() {
    setCriteria((current) => [...current, makeCriterion()]);
  }

  function removeCriterion(index: number) {
    setCriteria((current) => current.filter((_, criterionIndex) => criterionIndex !== index));
  }

  function resetSearch() {
    setCriteria([makeCriterion()]);
    setResults([]);
    setSelectedResultIds([]);
    setHasMoreResults(false);
    setSearchCursor(null);
    setResultError(null);
    setStatusMessage("Search form cleared.");
  }

  function validateCriteria(input: SearchCriterion[]): string | null {
    for (const criterion of input) {
      if (criterion.operator === "range") {
        if (!criterion.value?.trim() || !criterion.value2?.trim()) {
          return "Every range search needs both minimum and maximum values.";
        }
        continue;
      }
      if (criterion.operator === "terms") {
        if (!criterion.values || criterion.values.every((value) => !value.trim())) {
          return "Every multi-value search needs at least one value.";
        }
        continue;
      }
      if (!criterion.value?.trim()) {
        return "Every search rule needs a value.";
      }
    }
    return null;
  }

  async function runSearch(mode: "replace" | "append") {
    const validationError = validateCriteria(criteria);
    if (validationError) {
      setResultError(validationError);
      return;
    }
    if (authState !== "ready") {
      setResultError("Confirm the authenticated Digital Gems session before searching.");
      return;
    }

    const requestCursor = mode === "append" ? searchCursor : null;
    if (mode === "replace") {
      setResults([]);
      setSelectedResultIds([]);
      setHasMoreResults(false);
      setSearchCursor(null);
    }

    setIsSearching(true);
    setResultError(null);
    scheduleSearchTimeout();

    try {
      const response = await invoke<SearchResponse>("search_exam_papers", {
        criteria,
        cursor: requestCursor,
      });
      clearSearchTimeout();
      setIsSearching(false);
      setResultError(null);
      setStatusMessage(
        response.results.length > 0
          ? `Loaded ${response.results.length} result${response.results.length === 1 ? "" : "s"} from Digital Gems.`
          : "Search completed but no examination papers were found for the current filters.",
      );
      setResults((current) => mergeResults(current, response.results));
      setHasMoreResults(response.hasMore);
      setSearchCursor(response.cursor ?? null);
    } catch (error) {
      clearSearchTimeout();
      setIsSearching(false);
      setResultError(formatError(error));
    }
  }

  function toggleResultSelection(resultId: string) {
    setSelectedResultIds((current) =>
      current.includes(resultId)
        ? current.filter((id) => id !== resultId)
        : [...current, resultId],
    );
  }

  function toggleSelectAllLoaded() {
    if (allLoadedSelected) {
      setSelectedResultIds([]);
      return;
    }
    setSelectedResultIds(results.map((result) => result.id));
  }

  async function enqueueDownloads(items: ExamPaperResult[]) {
    const downloadableItems = items.filter((item) => item.downloadable);
    if (downloadableItems.length === 0) {
      setResultError("Select at least one examination paper before starting a batch download.");
      return;
    }
    if (authState !== "ready") {
      setResultError("Confirm the authenticated session before downloading.");
      return;
    }

    const destination = await open({
      directory: true,
      multiple: false,
      title: "Choose a destination folder for exam papers",
    });

    if (!destination || Array.isArray(destination)) {
      return;
    }

    downloadDestinationRef.current = destination;
    const jobs = downloadableItems.map<DownloadJob>((result) => ({
      id: crypto.randomUUID(),
      resultId: result.id,
      destinationPath: destination,
      filename: buildFilename(result),
      state: "queued",
      bytesReceived: 0,
      progressPercent: 0,
      resultSnapshot: result,
    }));

    setDownloadJobs((current) => [...jobs, ...current]);
    setStatusMessage(
      `Queued ${jobs.length} download${jobs.length === 1 ? "" : "s"} to ${destination}.`,
    );
  }

  async function cancelDownload(job: DownloadJob) {
    if (job.state === "queued") {
      setDownloadJobs((current) =>
        current.map((entry) =>
          entry.id === job.id ? { ...entry, state: "cancelled" } : entry,
        ),
      );
      return;
    }

    if (job.state !== "running") {
      return;
    }

    try {
      await invoke("cancel_download", {
        jobId: job.id,
      });
    } catch (error) {
      setDownloadJobs((current) =>
        current.map((entry) =>
          entry.id === job.id
            ? {
                ...entry,
                errorMessage: formatError(error),
              }
            : entry,
        ),
      );
    }
  }

  function retryDownload(job: DownloadJob) {
    runningJobsRef.current.delete(job.id);
    setDownloadJobs((current) =>
      current.map((entry) =>
        entry.id === job.id
          ? {
              ...entry,
              state: "queued",
              bytesReceived: 0,
              bytesTotal: undefined,
              progressPercent: 0,
              errorMessage: undefined,
            }
          : entry,
      ),
    );
  }

  return (
    <main className="app-shell">
      <section className="workspace">
        <div className="masthead">
          <div>
            <p className="eyebrow">NUS Libraries</p>
            <h1>Exam Papers Workspace</h1>
            <p className="lede">
              Native advanced search, structured results, and queued downloads backed by an authenticated Digital Gems session.
            </p>
          </div>
          <div className="status-stack">
            <span className={`status-pill ${authState}`}>Session: {authState}</span>
            <p className="status-copy">{statusMessage}</p>
            {resultError ? <p className="error-copy">{resultError}</p> : null}
          </div>
        </div>

        {authState !== "ready" ? (
          <section className="panel login-gate">
            <div className="panel-header">
              <div>
                <p className="section-kicker">Authentication</p>
                <h2>Sign in to Digital Gems</h2>
              </div>
              <div className="action-row">
                <button
                  type="button"
                  className="primary-button"
                  onClick={() => void openAuthWindow()}
                  disabled={isOpeningAuth || isCheckingAuth}
                >
                  {isOpeningAuth ? "Opening..." : "Sign in"}
                </button>
                <button
                  type="button"
                  className="ghost-button"
                  onClick={() => void bootstrapAuthSession()}
                  disabled={isCheckingAuth}
                >
                  {isCheckingAuth ? "Checking..." : "Recheck saved session"}
                </button>
              </div>
            </div>
            <p className="panel-note">
              The app keeps a hidden background Digital Gems session and reuses it on future launches when the cookies are still valid.
            </p>
          </section>
        ) : null}

        {authState === "ready" ? (
        <div className="panel-stack">
          <section className="panel search-panel">
            <div className="panel-header">
              <div>
                <p className="section-kicker">Advanced search</p>
                <h2>Search rules</h2>
              </div>
              <div className="action-row">
                <button type="button" className="ghost-button" onClick={resetSearch}>
                  Clear rules
                </button>
                <button type="button" className="primary-button" onClick={() => void runSearch("replace")} disabled={isSearching}>
                  {isSearching ? "Searching..." : "Run search"}
                </button>
              </div>
            </div>

            <div className="criteria-list">
              {criteria.map((criterion, index) => (
                <article className="criterion-card" key={`criterion-${index}`}>
                  <div className="criterion-grid">
                    <label>
                      <span>Field</span>
                      <select
                        value={criterion.field}
                        onChange={(event) =>
                          updateCriterion(index, {
                            field: event.currentTarget.value as SearchCriterion["field"],
                          })
                        }
                      >
                        {FIELD_OPTIONS.map((option) => (
                          <option key={option.value} value={option.value}>
                            {option.label}
                          </option>
                        ))}
                      </select>
                    </label>

                    <label>
                      <span>Condition</span>
                      <select
                        value={criterion.condition}
                        onChange={(event) =>
                          updateCriterion(index, {
                            condition: event.currentTarget.value as SearchCriterion["condition"],
                          })
                        }
                      >
                        {CONDITION_OPTIONS.map((option) => (
                          <option key={option.value} value={option.value}>
                            {option.label}
                          </option>
                        ))}
                      </select>
                    </label>

                    <label>
                      <span>Type of search</span>
                      <select
                        value={criterion.operator}
                        onChange={(event) =>
                          handleOperatorChange(
                            index,
                            event.currentTarget.value as SearchCriterion["operator"],
                          )
                        }
                      >
                        {OPERATOR_OPTIONS.map((option) => (
                          <option key={option.value} value={option.value}>
                            {option.label}
                          </option>
                        ))}
                      </select>
                    </label>

                    {criterion.operator === "range" ? (
                      <div className="range-row">
                        <label>
                          <span>Minimum value</span>
                          <input
                            type="text"
                            value={criterion.value ?? ""}
                            onChange={(event) =>
                              updateCriterion(index, { value: event.currentTarget.value })
                            }
                          />
                        </label>
                        <label>
                          <span>Maximum value</span>
                          <input
                            type="text"
                            value={criterion.value2 ?? ""}
                            onChange={(event) =>
                              updateCriterion(index, { value2: event.currentTarget.value })
                            }
                          />
                        </label>
                      </div>
                    ) : criterion.operator === "terms" ? (
                      <label className="full-span">
                        <span>Multi values</span>
                        <textarea
                          rows={3}
                          value={(criterion.values ?? []).join("\n")}
                          onChange={(event) =>
                            updateCriterion(index, {
                              values: event.currentTarget.value
                                .split("\n")
                                .map((value) => value.trim())
                                .filter(Boolean),
                            })
                          }
                          placeholder="One value per line"
                        />
                      </label>
                    ) : (
                      <label className="full-span">
                        <span>Value</span>
                        <input
                          type="text"
                          value={criterion.value ?? ""}
                          onChange={(event) =>
                            updateCriterion(index, { value: event.currentTarget.value })
                          }
                        />
                      </label>
                    )}
                  </div>

                  <div className="criterion-footer">
                    <span>Rule {index + 1}</span>
                    <button
                      type="button"
                      className="ghost-button"
                      onClick={() => removeCriterion(index)}
                      disabled={criteria.length === 1}
                    >
                      Remove
                    </button>
                  </div>
                </article>
              ))}
            </div>

            <div className="search-footer">
              <button type="button" className="secondary-button" onClick={addCriterion}>
                Add more criteria
              </button>
              {hasMoreResults ? (
                <button
                  type="button"
                  className="ghost-button"
                  onClick={() => void runSearch("append")}
                  disabled={isSearching}
                >
                  Load more
                </button>
              ) : null}
            </div>
          </section>
        </div>
        ) : null}

        {authState === "ready" ? (
        <section className="panel results-panel">
          <div className="panel-header">
            <div>
              <p className="section-kicker">Search results</p>
              <h2>Loaded papers</h2>
            </div>
            <div className="action-row">
              <button
                type="button"
                className="ghost-button"
                onClick={toggleSelectAllLoaded}
                disabled={results.length === 0}
              >
                {allLoadedSelected ? "Clear loaded selection" : "Select all loaded"}
              </button>
              <button
                type="button"
                className="primary-button"
                onClick={() => void enqueueDownloads(selectedResults)}
                disabled={selectedResults.length === 0}
              >
                Download selected
              </button>
            </div>
          </div>

          <div className="result-table">
            <div className="result-table-header">
              <span />
              <span>Paper</span>
              <span>Course</span>
              <span>Exam session</span>
              <span>Status</span>
              <span />
            </div>
            {results.length === 0 ? (
              <div className="empty-state">
                <p>No loaded results yet.</p>
                <span>Run the advanced search after confirming the Digital Gems session.</span>
              </div>
            ) : (
              results.map((result) => (
                <article className="result-row" key={result.id}>
                  <label className="checkbox-cell">
                    <input
                      type="checkbox"
                      checked={selectedResultIds.includes(result.id)}
                      onChange={() => toggleResultSelection(result.id)}
                    />
                  </label>
                  <div>
                    <strong>
                      <button
                        type="button"
                        className="link-button"
                        onClick={() => void openPaper(result)}
                      >
                        {result.title}
                      </button>
                    </strong>
                    <p>Open in Digital Gems</p>
                  </div>
                  <div>
                    <strong>{result.courseCode ?? "Unknown"}</strong>
                    <p>{result.courseName ?? "Course name unavailable"}</p>
                  </div>
                  <div>
                    <strong>{result.year ?? "Unknown year"}</strong>
                    <p>{result.semester ?? "Semester unavailable"}</p>
                  </div>
                  <div>
                    <span className={`availability ${result.downloadable ? "ready" : "blocked"}`}>
                      {result.downloadable ? "Downloadable" : "Restricted"}
                    </span>
                    {!result.downloadable && result.unavailableReason ? (
                      <p>{result.unavailableReason}</p>
                    ) : null}
                  </div>
                  <div className="row-actions">
                    <button
                      type="button"
                      className="ghost-button"
                      disabled={!result.downloadable}
                      onClick={() => void enqueueDownloads([result])}
                    >
                      Download
                    </button>
                  </div>
                </article>
              ))
            )}
          </div>
        </section>
        ) : null}

        {authState === "ready" ? (
        <section className="panel downloads-panel">
          <div className="panel-header">
            <div>
              <p className="section-kicker">Downloads</p>
              <h2>Queue monitor</h2>
            </div>
            <p className="panel-note">
              Max concurrency: {MAX_CONCURRENT_DOWNLOADS}. Destination folder:{" "}
              {downloadDestinationRef.current ?? "Not chosen yet"}.
            </p>
          </div>

          {downloadJobs.length === 0 ? (
            <div className="empty-state compact">
              <p>No downloads queued.</p>
              <span>Select one or more papers, then start a batch.</span>
            </div>
          ) : (
            <div className="download-list">
              {downloadJobs.map((job) => (
                <article className="download-row" key={job.id}>
                  <div className="download-topline">
                    <div>
                      <strong>{job.filename}</strong>
                      <p>{describeJobState(job.state)}</p>
                    </div>
                    <div className="row-actions">
                      {job.state === "running" || job.state === "queued" ? (
                        <button
                          type="button"
                          className="ghost-button"
                          onClick={() => void cancelDownload(job)}
                        >
                          Cancel
                        </button>
                      ) : null}
                      {job.state === "failed" || job.state === "cancelled" ? (
                        <button
                          type="button"
                          className="ghost-button"
                          onClick={() => retryDownload(job)}
                        >
                          Retry
                        </button>
                      ) : null}
                    </div>
                  </div>
                  <div className="progress-track">
                    <div
                      className={`progress-fill ${job.state}`}
                      style={{ width: `${job.progressPercent ?? 0}%` }}
                    />
                  </div>
                  <div className="download-meta">
                    <span>{formatProgress(job.bytesReceived, job.bytesTotal)}</span>
                    {job.destinationPath ? <span>{job.destinationPath}</span> : null}
                  </div>
                  {job.errorMessage ? <p className="error-copy">{job.errorMessage}</p> : null}
                </article>
              ))}
            </div>
          )}
        </section>
        ) : null}
      </section>
    </main>
  );
}

function mergeResults(current: ExamPaperResult[], incoming: ExamPaperResult[]) {
  const seen = new Map(current.map((result) => [result.id, result]));
  for (const result of incoming) {
    seen.set(result.id, result);
  }
  return Array.from(seen.values());
}

function buildFilename(result: ExamPaperResult) {
  const segments = [
    result.courseCode,
    result.year,
    result.semester,
    result.title,
  ].filter(Boolean);
  return `${segments.join(" - ") || "exam-paper"}.pdf`;
}

function formatProgress(bytesReceived: number, bytesTotal?: number) {
  const received = formatBytes(bytesReceived);
  const total = bytesTotal ? formatBytes(bytesTotal) : "Unknown size";
  return `${received} / ${total}`;
}

function formatBytes(input: number) {
  if (input === 0) {
    return "0 B";
  }
  const units = ["B", "KB", "MB", "GB"];
  const exponent = Math.min(Math.floor(Math.log(input) / Math.log(1024)), units.length - 1);
  const value = input / 1024 ** exponent;
  return `${value.toFixed(value >= 10 || exponent === 0 ? 0 : 1)} ${units[exponent]}`;
}

function describeJobState(state: DownloadJobState) {
  switch (state) {
    case "queued":
      return "Queued";
    case "running":
      return "Downloading";
    case "completed":
      return "Completed";
    case "failed":
      return "Failed";
    case "cancelled":
      return "Cancelled";
  }
}

function formatError(error: unknown) {
  if (typeof error === "string") {
    return error;
  }
  if (error instanceof Error) {
    return error.message;
  }
  return "Unexpected error";
}

export default App;
