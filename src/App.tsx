import { useEffect, useMemo, useRef, useState, type KeyboardEvent as ReactKeyboardEvent } from "react";
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
  SearchFacetGroup,
  SearchFacetValue,
  SearchCriterion,
  SearchResponse,
} from "./types";

const MAX_CONCURRENT_DOWNLOADS = 3;
const PAGE_SIZE = 10;
const THEME_STORAGE_KEY = "nus-pyp-theme";
type ThemePreference = "system" | "light" | "dark";
type EffectiveTheme = "light" | "dark";
type WorkspaceView = "results" | "refine" | "downloads";

interface AuthSessionStatus {
  ready: boolean;
  currentUrl: string;
  message: string;
}

interface CliToolStatus {
  platform: string;
  commandName: string;
  bundled: boolean;
  bundledPath?: string;
  installActionAvailable: boolean;
  installPath?: string;
  pathManagedByInstaller: boolean;
  inPath: boolean;
  message: string;
}

interface SearchSnapshot {
  title: string;
  criteria: SearchCriterion[];
  facetClauses: string[];
  searchUrl: string | null;
  results: ExamPaperResult[];
  selectedResultIds: string[];
  facets: SearchFacetGroup[];
  totalResults: number | null;
  hasMoreResults: boolean;
  searchCursor: string | null;
  currentPage: number;
}

function makeCriterion(): SearchCriterion {
  return {
    ...DEFAULT_CRITERION,
    values: [],
  };
}

function getStoredThemePreference(): ThemePreference {
  if (typeof window === "undefined") {
    return "system";
  }

  const storedPreference = window.localStorage.getItem(THEME_STORAGE_KEY);
  if (
    storedPreference === "system" ||
    storedPreference === "light" ||
    storedPreference === "dark"
  ) {
    return storedPreference;
  }

  return "system";
}

function getSystemTheme(): EffectiveTheme {
  if (typeof window === "undefined") {
    return "light";
  }

  return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}

function cloneCriteria(input: SearchCriterion[]) {
  return input.map((criterion) => ({
    ...criterion,
    values: [...(criterion.values ?? [])],
  }));
}

function formatResultStatusMessage(response: SearchResponse, mode: "replace" | "append") {
  if (response.results.length === 0) {
    return "Search finished with no matching papers.";
  }

  const totalCopy = response.totalResults
    ? ` ${response.totalResults} matching paper${response.totalResults === 1 ? "" : "s"} available.`
    : "";

  if (mode === "append") {
    return `Loaded ${response.results.length} more result${response.results.length === 1 ? "" : "s"}.${totalCopy}`;
  }

  return `Loaded ${response.results.length} result${response.results.length === 1 ? "" : "s"}.${totalCopy}`;
}

function describeSnapshotPath(history: SearchSnapshot[]) {
  if (history.length === 0) {
    return "Base search";
  }

  return history.map((snapshot) => snapshot.title).join(" / ");
}

function getPageFromSearchUrl(searchUrl?: string | null) {
  if (!searchUrl) {
    return 1;
  }

  try {
    const parsed = new URL(searchUrl);
    const page = Number.parseInt(parsed.searchParams.get("page") ?? "1", 10);
    return Number.isFinite(page) && page > 0 ? page : 1;
  } catch {
    return 1;
  }
}

function getCursorForPage(page: number) {
  return page > 1 ? String((page - 1) * PAGE_SIZE) : null;
}

function getVisiblePageNumbers(currentPage: number, totalPages: number) {
  if (totalPages <= 1) {
    return [];
  }

  const maxVisible = 5;
  const halfWindow = Math.floor(maxVisible / 2);
  let start = Math.max(1, currentPage - halfWindow);
  let end = Math.min(totalPages, start + maxVisible - 1);

  if (end - start + 1 < maxVisible) {
    start = Math.max(1, end - maxVisible + 1);
  }

  return Array.from({ length: end - start + 1 }, (_, index) => start + index);
}

function App() {
  const [themePreference, setThemePreference] =
    useState<ThemePreference>(getStoredThemePreference);
  const [systemTheme, setSystemTheme] = useState<EffectiveTheme>(getSystemTheme);
  const [criteria, setCriteria] = useState<SearchCriterion[]>([makeCriterion()]);
  const [results, setResults] = useState<ExamPaperResult[]>([]);
  const [selectedResultIds, setSelectedResultIds] = useState<string[]>([]);
  const [downloadJobs, setDownloadJobs] = useState<DownloadJob[]>([]);
  const [workspaceView, setWorkspaceView] = useState<WorkspaceView>("results");
  const [isSearchEditorOpen, setIsSearchEditorOpen] = useState(true);
  const [authState, setAuthState] = useState<"unknown" | "required" | "ready">(
    "unknown",
  );
  const [isOpeningAuth, setIsOpeningAuth] = useState(false);
  const [isCheckingAuth, setIsCheckingAuth] = useState(false);
  const [isSearching, setIsSearching] = useState(false);
  const [hasMoreResults, setHasMoreResults] = useState(false);
  const [searchCursor, setSearchCursor] = useState<string | null>(null);
  const [currentPage, setCurrentPage] = useState(1);
  const [activeSearchUrl, setActiveSearchUrl] = useState<string | null>(null);
  const [totalResults, setTotalResults] = useState<number | null>(null);
  const [facets, setFacets] = useState<SearchFacetGroup[]>([]);
  const [isRefinementCollapsed, setIsRefinementCollapsed] = useState(true);
  const [activeFacetClauses, setActiveFacetClauses] = useState<string[]>([]);
  const [searchHistory, setSearchHistory] = useState<SearchSnapshot[]>([]);
  const [statusMessage, setStatusMessage] = useState(
    "Checking saved Digital Gems session.",
  );
  const [resultError, setResultError] = useState<string | null>(null);
  const [cliToolStatus, setCliToolStatus] = useState<CliToolStatus | null>(null);
  const [isInstallingCli, setIsInstallingCli] = useState(false);
  const searchTimeoutRef = useRef<number | null>(null);
  const runningJobsRef = useRef<Set<string>>(new Set());
  const downloadDestinationRef = useRef<string | null>(null);
  const authPollRef = useRef<number | null>(null);
  const resultsPanelRef = useRef<HTMLElement | null>(null);
  const pendingResultsFocusRef = useRef<"panel" | "tab" | null>(null);

  const allLoadedSelected =
    results.length > 0 && results.every((result) => selectedResultIds.includes(result.id));

  const effectiveTheme = themePreference === "system" ? systemTheme : themePreference;
  const selectedResults = useMemo(
    () => results.filter((result) => selectedResultIds.includes(result.id)),
    [results, selectedResultIds],
  );
  const loadedResultCount = results.length;
  const activeDownloadCount = useMemo(
    () => downloadJobs.filter((job) => job.state === "queued" || job.state === "running").length,
    [downloadJobs],
  );
  const searchPath = useMemo(() => describeSnapshotPath(searchHistory), [searchHistory]);
  const totalPages = useMemo(() => {
    if (typeof totalResults === "number" && totalResults > 0) {
      return Math.max(1, Math.ceil(totalResults / PAGE_SIZE));
    }

    if (hasMoreResults) {
      return Math.max(2, currentPage + 1);
    }

    return results.length > 0 ? 1 : 0;
  }, [currentPage, hasMoreResults, results.length, totalResults]);
  const visiblePageNumbers = useMemo(
    () => getVisiblePageNumbers(currentPage, totalPages),
    [currentPage, totalPages],
  );
  const pageSummary = useMemo(() => {
    if (results.length === 0) {
      return "No results loaded";
    }

    const start = (currentPage - 1) * PAGE_SIZE + 1;
    const end = start + results.length - 1;
    if (typeof totalResults === "number" && totalResults > 0) {
      return `Showing ${start}-${end} of ${totalResults} matching paper${totalResults === 1 ? "" : "s"}`;
    }

    return `Showing ${start}-${end}`;
  }, [currentPage, results.length, totalResults]);
  const searchRuleSummary = useMemo(() => {
    if (criteria.length === 0) {
      return "No search rules yet";
    }

    return criteria
      .map((criterion) => {
        const fieldLabel =
          FIELD_OPTIONS.find((option) => option.value === criterion.field)?.label ?? criterion.field;
        const valueLabel =
          criterion.operator === "terms"
            ? `${criterion.values?.length ?? 0} values`
            : criterion.operator === "range"
              ? `${criterion.value ?? ""} to ${criterion.value2 ?? ""}`
              : criterion.value ?? "";

        return `${fieldLabel}: ${valueLabel}`.trim();
      })
      .filter(Boolean)
      .join(" • ");
  }, [criteria]);
  const totalFacetValues = useMemo(
    () => facets.reduce((sum, group) => sum + group.values.length, 0),
    [facets],
  );
  const workspaceTabs = [
    { id: "results" as const, label: "Results" },
    { id: "refine" as const, label: `Refine${facets.length ? ` (${facets.length})` : ""}` },
    {
      id: "downloads" as const,
      label: `Downloads${downloadJobs.length ? ` (${downloadJobs.length})` : ""}`,
    },
  ];

  useEffect(() => {
    const mediaQuery = window.matchMedia("(prefers-color-scheme: dark)");
    const handleThemeChange = (event: MediaQueryListEvent) => {
      setSystemTheme(event.matches ? "dark" : "light");
    };

    setSystemTheme(mediaQuery.matches ? "dark" : "light");
    mediaQuery.addEventListener("change", handleThemeChange);

    return () => {
      mediaQuery.removeEventListener("change", handleThemeChange);
    };
  }, []);

  useEffect(() => {
    window.localStorage.setItem(THEME_STORAGE_KEY, themePreference);
  }, [themePreference]);

  useEffect(() => {
    document.documentElement.dataset.theme = effectiveTheme;
    document.documentElement.style.colorScheme = effectiveTheme;
  }, [effectiveTheme]);

  useEffect(() => {
    void bootstrapAuthSession();
    void refreshCliToolStatus();

    const unlisteners: Promise<UnlistenFn>[] = [
      listen<AppEventMap["auth:login-ready"]>("auth:login-ready", () => {
        setAuthState("ready");
        setIsOpeningAuth(false);
        setIsCheckingAuth(false);
        setStatusMessage("Session ready. Search and downloads use the hidden Digital Gems window.");
      }),
      listen<AppEventMap["auth:login-required"]>("auth:login-required", (event) => {
        setAuthState("required");
        setIsOpeningAuth(false);
        setIsCheckingAuth(false);
        setStatusMessage(
          event.payload?.message ??
            "Digital Gems is not signed in yet. Finish the sign-in flow and check again.",
        );
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

  useEffect(() => {
    if (workspaceView !== "results") {
      return;
    }

    const activeElement = document.activeElement;
    const focusIsInHiddenUtilityPanel =
      activeElement instanceof HTMLElement &&
      (["refine", "downloads"] as const).some((view) => {
        const panel = document.getElementById(`workspace-panel-${view}`);
        return panel instanceof HTMLElement && panel.contains(activeElement);
      });

    const nextFocusTarget = pendingResultsFocusRef.current;
    if (!focusIsInHiddenUtilityPanel && nextFocusTarget === null) {
      return;
    }

    pendingResultsFocusRef.current = null;
    window.requestAnimationFrame(() => {
      if (nextFocusTarget === "tab") {
        const resultsTab = document.getElementById("workspace-tab-results");
        if (resultsTab instanceof HTMLButtonElement) {
          resultsTab.focus();
          return;
        }
      }

      resultsPanelRef.current?.focus();
    });
  }, [workspaceView]);

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
        "Digital Gems timed out. Confirm the session is signed in, then try again.",
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

  async function refreshCliToolStatus() {
    try {
      const status = await invoke<CliToolStatus>("get_cli_tool_status");
      setCliToolStatus(status);
    } catch {
      setCliToolStatus(null);
    }
  }

  async function installCliTool() {
    setIsInstallingCli(true);
    try {
      const status = await invoke<CliToolStatus>("install_cli_tool");
      setCliToolStatus(status);
      setResultError(null);
      setStatusMessage(status.message);
    } catch (error) {
      setResultError(formatError(error));
    } finally {
      setIsInstallingCli(false);
    }
  }

  async function openAuthWindow() {
    setIsOpeningAuth(true);
    setResultError(null);
    try {
      await invoke("open_auth_window");
      setStatusMessage("Finish signing in through Digital Gems. The workspace reconnects automatically.");
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
    setFacets([]);
    setActiveFacetClauses([]);
    setActiveSearchUrl(null);
    setTotalResults(null);
    setHasMoreResults(false);
    setSearchCursor(null);
    setCurrentPage(1);
    setIsRefinementCollapsed(true);
    showResultsView();
    if (!isSearchEditorOpen) {
      setIsSearchEditorOpen(true);
    }
    resetSearchHistory();
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

  async function runSearch(
    mode: "replace" | "append",
    options?: {
      criteriaOverride?: SearchCriterion[];
      searchUrlOverride?: string;
      rawQueryClausesOverride?: string[];
      facetClausesOverride?: string[];
      clearHistory?: boolean;
      pageOverride?: number;
    },
  ) {
    const activeCriteria = options?.criteriaOverride ?? criteria;
    const requestedPage = options?.pageOverride ?? (mode === "append" ? currentPage + 1 : 1);
    const searchUrlOverride =
      options?.searchUrlOverride ??
      (mode === "append" || requestedPage > 1 ? activeSearchUrl ?? undefined : undefined);
    const rawQueryClauses = options?.rawQueryClausesOverride;
    const facetClauses = options?.facetClausesOverride ?? activeFacetClauses;
    const previousReplaceState =
      mode === "replace"
        ? {
            criteria: cloneCriteria(criteria),
            results: [...results],
            selectedResultIds: [...selectedResultIds],
            facets: facets.map((group) => ({
              ...group,
              values: group.values.map((value) => ({ ...value })),
            })),
            activeFacetClauses: [...activeFacetClauses],
            activeSearchUrl,
            totalResults,
            hasMoreResults,
            searchCursor,
            currentPage,
            isRefinementCollapsed,
            searchHistory: [...searchHistory],
            workspaceView,
            isSearchEditorOpen,
          }
        : null;
    if (!rawQueryClauses) {
      const validationError = validateCriteria(activeCriteria);
      if (validationError) {
        setResultError(validationError);
        return false;
      }
    }
    if (authState !== "ready") {
      setResultError("Confirm the authenticated Digital Gems session before searching.");
      return false;
    }

    const requestCursor =
      options?.pageOverride !== undefined
        ? getCursorForPage(options.pageOverride)
        : mode === "append"
          ? searchCursor
          : null;
    if (mode === "replace") {
      setCriteria(cloneCriteria(activeCriteria));
      setActiveFacetClauses([...facetClauses]);
      setActiveSearchUrl(searchUrlOverride ?? null);
      setResults([]);
      setSelectedResultIds([]);
      setFacets([]);
      setTotalResults(null);
      setHasMoreResults(false);
      setSearchCursor(null);
      setCurrentPage(requestedPage);
      setIsRefinementCollapsed(true);
      if (options?.clearHistory ?? true) {
        resetSearchHistory();
      }
    }

    setIsSearching(true);
    setResultError(null);
    scheduleSearchTimeout();

    try {
      const response = await invoke<SearchResponse>("search_exam_papers", {
        criteria: activeCriteria,
        searchUrl: searchUrlOverride,
        rawQueryClauses,
        facetClauses,
        cursor: requestCursor,
      });
      clearSearchTimeout();
      setIsSearching(false);
      applySearchResponse(response, mode);
      return true;
    } catch (error) {
      clearSearchTimeout();
      setIsSearching(false);
      setResultError(formatError(error));
      if (previousReplaceState) {
        setCriteria(previousReplaceState.criteria);
        setResults(previousReplaceState.results);
        setSelectedResultIds(previousReplaceState.selectedResultIds);
        setFacets(previousReplaceState.facets);
        setActiveFacetClauses(previousReplaceState.activeFacetClauses);
        setActiveSearchUrl(previousReplaceState.activeSearchUrl);
        setTotalResults(previousReplaceState.totalResults);
        setHasMoreResults(previousReplaceState.hasMoreResults);
        setSearchCursor(previousReplaceState.searchCursor);
        setCurrentPage(previousReplaceState.currentPage);
        setIsRefinementCollapsed(previousReplaceState.isRefinementCollapsed);
        setSearchHistory(previousReplaceState.searchHistory);
        setWorkspaceView(previousReplaceState.workspaceView);
        setIsSearchEditorOpen(previousReplaceState.isSearchEditorOpen);
      }
      return false;
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

  function createSearchSnapshot(title: string): SearchSnapshot {
    const snapshotTitle = title.trim() || searchRuleSummary;
    return {
      title: snapshotTitle,
      criteria: cloneCriteria(criteria),
      facetClauses: [...activeFacetClauses],
      searchUrl: activeSearchUrl,
      results: [...results],
      selectedResultIds: [...selectedResultIds],
      facets: facets.map((group) => ({
        ...group,
        values: group.values.map((value) => ({ ...value })),
      })),
      totalResults,
      hasMoreResults,
      searchCursor,
      currentPage,
    };
  }

  function shouldRestoreFocusToResultsPanel() {
    if (workspaceView === "results") {
      return false;
    }

    const activePanel = document.getElementById(`workspace-panel-${workspaceView}`);
    const activeElement = document.activeElement;
    return (
      activePanel instanceof HTMLElement &&
      activeElement instanceof HTMLElement &&
      activePanel.contains(activeElement)
    );
  }

  function showResultsView(options?: { focusTab?: boolean }) {
    pendingResultsFocusRef.current = options?.focusTab
      ? "tab"
      : shouldRestoreFocusToResultsPanel()
        ? "panel"
        : null;
    setWorkspaceView("results");
  }

  function applySearchResponse(response: SearchResponse, mode: "replace" | "append") {
    setResultError(null);
    setStatusMessage(formatResultStatusMessage(response, mode));
    setResults(response.results);
    setHasMoreResults(response.hasMore);
    setSearchCursor(response.cursor ?? null);
    setActiveSearchUrl(response.searchUrl ?? null);
    setTotalResults(response.totalResults ?? null);
    setFacets(response.facets);
    setCurrentPage(getPageFromSearchUrl(response.searchUrl));
    showResultsView();
    if (mode === "replace") {
      setSelectedResultIds([]);
      setIsSearchEditorOpen(response.results.length === 0);
    }
  }

  async function applyFacet(facetGroup: SearchFacetGroup, facetValue: SearchFacetValue) {
    const nextFacetClauses = [...facetValue.queryClauses];
    if (
      nextFacetClauses.length === activeFacetClauses.length &&
      nextFacetClauses.every((clause, index) => clause === activeFacetClauses[index])
    ) {
      setStatusMessage(`${facetGroup.title}: ${facetValue.label} is already applied.`);
      return;
    }

    setSearchHistory((current) => [
      ...current,
      createSearchSnapshot(`${facetGroup.title}: ${facetValue.label}`),
    ]);
    const success = await runSearch("replace", {
      searchUrlOverride: facetValue.href,
      facetClausesOverride: nextFacetClauses,
      clearHistory: false,
    });
    if (!success) {
      setSearchHistory((current) => current.slice(0, -1));
    }
  }

  function restorePreviousSearch() {
    if (searchHistory.length === 0) {
      return;
    }

    const previous = searchHistory[searchHistory.length - 1];
    setCriteria(cloneCriteria(previous.criteria));
    setActiveFacetClauses(previous.facetClauses);
    setActiveSearchUrl(previous.searchUrl);
    setResults(previous.results);
    setSelectedResultIds(previous.selectedResultIds);
    setFacets(previous.facets);
    setTotalResults(previous.totalResults);
    setHasMoreResults(previous.hasMoreResults);
    setSearchCursor(previous.searchCursor);
    setCurrentPage(previous.currentPage);
    setIsRefinementCollapsed(true);
    showResultsView();
    setSearchHistory(searchHistory.slice(0, -1));
    setResultError(null);
    setStatusMessage(`Returned to ${previous.title.toLowerCase()}.`);
  }

  function resetSearchHistory() {
    setSearchHistory([]);
  }

  function restoreBaseSearch() {
    if (searchHistory.length === 0) {
      return;
    }

    const rootSnapshot = searchHistory[0];
    setCriteria(cloneCriteria(rootSnapshot.criteria));
    setActiveFacetClauses(rootSnapshot.facetClauses);
    setActiveSearchUrl(rootSnapshot.searchUrl);
    setResults(rootSnapshot.results);
    setSelectedResultIds(rootSnapshot.selectedResultIds);
    setFacets(rootSnapshot.facets);
    setTotalResults(rootSnapshot.totalResults);
    setHasMoreResults(rootSnapshot.hasMoreResults);
    setSearchCursor(rootSnapshot.searchCursor);
    setCurrentPage(rootSnapshot.currentPage);
    setIsRefinementCollapsed(true);
    showResultsView();
    setSearchHistory([]);
    setResultError(null);
    setStatusMessage("Returned to the base search.");
  }

  async function goToPage(page: number) {
    if (page < 1 || page === currentPage || isSearching) {
      return;
    }

    await runSearch("replace", {
      searchUrlOverride: activeSearchUrl ?? undefined,
      pageOverride: page,
      clearHistory: false,
    });
  }

  function focusWorkspaceTab(view: WorkspaceView) {
    window.requestAnimationFrame(() => {
      const nextTab = document.getElementById(`workspace-tab-${view}`);
      if (nextTab instanceof HTMLButtonElement) {
        nextTab.focus();
      }
    });
  }

  function handleWorkspaceTabKeyDown(
    event: ReactKeyboardEvent<HTMLButtonElement>,
    index: number,
  ) {
    if (
      event.key !== "ArrowRight" &&
      event.key !== "ArrowLeft" &&
      event.key !== "Home" &&
      event.key !== "End"
    ) {
      return;
    }

    event.preventDefault();
    const lastIndex = workspaceTabs.length - 1;
    const nextIndex =
      event.key === "Home"
        ? 0
        : event.key === "End"
          ? lastIndex
          : event.key === "ArrowRight"
            ? (index + 1) % workspaceTabs.length
            : (index - 1 + workspaceTabs.length) % workspaceTabs.length;
    const nextView = workspaceTabs[nextIndex]?.id;

    if (!nextView) {
      return;
    }

    setWorkspaceView(nextView);
    focusWorkspaceTab(nextView);
  }

  return (
    <main className={`app-shell ${authState === "ready" ? "is-authenticated" : ""}`}>
      <section className="workspace">
        <header className="workspace-bar">
          <div className="workspace-bar-primary">
            <div className="workspace-identity">
              <p className="section-kicker">NUS Libraries</p>
              <h1>Exam Papers Workspace</h1>
            </div>
            <div className="workspace-metrics" aria-label="Workspace metrics">
              <span className="metric-pill">
                Session: {authState === "ready" ? "Connected" : "Required"}
              </span>
              <span className="metric-pill">
                Results:{" "}
                {typeof totalResults === "number"
                  ? `${loadedResultCount}/${totalResults}`
                  : loadedResultCount}
              </span>
              <span className="metric-pill">Downloads: {activeDownloadCount}</span>
            </div>
          </div>
          <div className="workspace-bar-secondary">
            <div className="theme-control compact" role="group" aria-label="Theme switcher">
              <span className="theme-label">Theme</span>
              <div className="theme-toggle">
                {(["system", "light", "dark"] as ThemePreference[]).map((option) => (
                  <button
                    key={option}
                    type="button"
                    className={`theme-option ${themePreference === option ? "active" : ""}`}
                    onClick={() => setThemePreference(option)}
                    aria-pressed={themePreference === option}
                  >
                    {option === "system"
                      ? "System"
                      : option.charAt(0).toUpperCase() + option.slice(1)}
                  </button>
                ))}
              </div>
            </div>
            <div className="status-stack compact">
              <p className="status-copy">{statusMessage}</p>
              {cliToolStatus ? (
                <div className="cli-inline">
                  <span className="cli-copy">{cliToolStatus.message}</span>
                  {cliToolStatus.installActionAvailable ? (
                    <button
                      type="button"
                      className="ghost-button compact-button"
                      onClick={() => void installCliTool()}
                      disabled={isInstallingCli || !cliToolStatus.bundled}
                    >
                      {isInstallingCli ? "Installing CLI..." : "Install CLI"}
                    </button>
                  ) : cliToolStatus.pathManagedByInstaller ? (
                    <span className="metric-pill cli-pill">
                      Command: {cliToolStatus.commandName}
                    </span>
                  ) : null}
                </div>
              ) : null}
              {resultError ? <p className="error-copy">{resultError}</p> : null}
            </div>
          </div>
        </header>

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
              Saved cookies are reused when still valid.
            </p>
          </section>
        ) : null}

        {authState === "ready" ? (
          <div className="panel-stack">
            <section className="panel command-bar">
              <div className="command-bar-row">
                <div className="command-bar-summary">
                  <p className="section-kicker">Search</p>
                  <strong>{searchRuleSummary}</strong>
                  <span>{searchPath}</span>
                </div>
                <div className="action-row">
                  <button
                    type="button"
                    className="ghost-button"
                    onClick={() => setIsSearchEditorOpen((current) => !current)}
                  >
                    {isSearchEditorOpen ? "Hide editor" : "Edit search"}
                  </button>
                  <button type="button" className="ghost-button" onClick={resetSearch}>
                    Clear
                  </button>
                  <button
                    type="button"
                    className="primary-button"
                    onClick={() => void runSearch("replace")}
                    disabled={isSearching}
                  >
                    {isSearching ? "Searching..." : "Run search"}
                  </button>
                </div>
              </div>

              {isSearchEditorOpen ? (
                <div className="command-bar-editor">
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
                                  condition:
                                    event.currentTarget.value as SearchCriterion["condition"],
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
                  </div>
                </div>
              ) : null}
            </section>
            <div
              className="workspace-tabs"
              role="tablist"
              aria-label="Workspace views"
              aria-orientation="horizontal"
            >
              {workspaceTabs.map((view, index) => (
                <button
                  key={view.id}
                  id={`workspace-tab-${view.id}`}
                  type="button"
                  role="tab"
                  aria-selected={workspaceView === view.id}
                  aria-controls={`workspace-panel-${view.id}`}
                  tabIndex={workspaceView === view.id ? 0 : -1}
                  className={`workspace-tab ${workspaceView === view.id ? "active" : ""}`}
                  onClick={() => setWorkspaceView(view.id)}
                  onKeyDown={(event) => handleWorkspaceTabKeyDown(event, index)}
                >
                  {view.label}
                </button>
              ))}
            </div>

            <div className="workspace-frame">
              <section
                id="workspace-panel-results"
                role="tabpanel"
                aria-labelledby="workspace-tab-results"
                hidden={workspaceView !== "results"}
                ref={resultsPanelRef}
                tabIndex={-1}
                className="panel results-panel"
              >
                  <div className="results-topbar">
                    <div className="panel-header">
                      <div>
                        <p className="section-kicker">Search results</p>
                        <h2>Loaded papers</h2>
                        <p className="panel-note">Review papers and queue downloads.</p>
                      </div>
                      <div className="action-row">
                        {searchHistory.length > 0 ? (
                          <button
                            type="button"
                            className="ghost-button"
                            onClick={restorePreviousSearch}
                            disabled={isSearching}
                          >
                            Back to previous results
                          </button>
                        ) : null}
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

                    <div className="search-summary-bar">
                      <div className="search-summary-copy">
                        <strong>{pageSummary}</strong>
                        <span>{searchPath}</span>
                      </div>
                      <div className="search-summary-actions">
                        {totalPages > 1 ? (
                          <span className="page-indicator">
                            Page {currentPage} of {totalPages}
                          </span>
                        ) : null}
                        {searchHistory.length > 0 ? (
                          <button
                            type="button"
                            className="secondary-button"
                            onClick={restoreBaseSearch}
                            disabled={isSearching}
                          >
                            Clear drill-down
                          </button>
                        ) : null}
                      </div>
                    </div>
                  </div>

                  <div className="results-body">
                    <div className="result-table">
                      <div className="result-table-header">
                        <label className="table-select-all" aria-label="Select all papers on this page">
                          <input
                            type="checkbox"
                            checked={allLoadedSelected}
                            onChange={toggleSelectAllLoaded}
                            disabled={results.length === 0}
                          />
                          <span className="sr-only">
                            {allLoadedSelected ? "Clear page selection" : "Select this page"}
                          </span>
                        </label>
                        <span>Paper</span>
                        <span>Course</span>
                        <span>Exam session</span>
                        <span>Status</span>
                        <span>Action</span>
                      </div>
                      {results.length === 0 ? (
                        <div className="empty-state">
                          <p>No loaded results yet.</p>
                          <span>Run a search after confirming the session.</span>
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
                              <span
                                className={`availability ${result.downloadable ? "ready" : "blocked"}`}
                              >
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
                  </div>

                  {totalPages > 1 ? (
                    <nav className="pagination-nav" aria-label="Search results pagination">
                      <button
                        type="button"
                        className="ghost-button"
                        onClick={() => void goToPage(currentPage - 1)}
                        disabled={isSearching || currentPage <= 1}
                      >
                        Previous
                      </button>
                      <div className="pagination-pages">
                        {visiblePageNumbers[0] > 1 ? (
                          <>
                            <button
                              type="button"
                              className="page-chip"
                              onClick={() => void goToPage(1)}
                              disabled={isSearching}
                            >
                              1
                            </button>
                            {visiblePageNumbers[0] > 2 ? (
                              <span className="pagination-ellipsis">...</span>
                            ) : null}
                          </>
                        ) : null}
                        {visiblePageNumbers.map((page) => (
                          <button
                            type="button"
                            key={page}
                            className={`page-chip ${page === currentPage ? "active" : ""}`}
                            onClick={() => void goToPage(page)}
                            disabled={isSearching}
                            aria-current={page === currentPage ? "page" : undefined}
                          >
                            {page}
                          </button>
                        ))}
                        {visiblePageNumbers[visiblePageNumbers.length - 1] < totalPages ? (
                          <>
                            {visiblePageNumbers[visiblePageNumbers.length - 1] <
                            totalPages - 1 ? (
                              <span className="pagination-ellipsis">...</span>
                            ) : null}
                            <button
                              type="button"
                              className="page-chip"
                              onClick={() => void goToPage(totalPages)}
                              disabled={isSearching}
                            >
                              {totalPages}
                            </button>
                          </>
                        ) : null}
                      </div>
                      <button
                        type="button"
                        className="ghost-button"
                        onClick={() => void goToPage(currentPage + 1)}
                        disabled={isSearching || currentPage >= totalPages}
                      >
                        Next
                      </button>
                    </nav>
                  ) : null}
              </section>

              <section
                id="workspace-panel-refine"
                role="tabpanel"
                aria-labelledby="workspace-tab-refine"
                hidden={workspaceView !== "refine"}
                className="panel utility-panel"
              >
                  <div className="panel-header utility-panel-header">
                    <div className="facet-sidebar-header">
                      <p className="section-kicker">Refine</p>
                      <h2>Refine results</h2>
                      <p className="panel-note">Apply filters, then return to results.</p>
                    </div>
                    <div className="utility-panel-actions">
                      {facets.length > 0 ? (
                        <button
                          type="button"
                          className="ghost-button refinement-toggle"
                          onClick={() => setIsRefinementCollapsed((current) => !current)}
                          aria-expanded={!isRefinementCollapsed}
                        >
                          {isRefinementCollapsed ? "Show filters" : "Hide filters"}
                        </button>
                      ) : null}
                      <button
                        type="button"
                        className="ghost-button"
                        onClick={() => showResultsView({ focusTab: true })}
                      >
                        Back to results
                      </button>
                    </div>
                  </div>

                  {facets.length === 0 ? (
                    <div className="empty-state compact">
                      <p>No refinement filters yet.</p>
                      <span>Run a search to load facets.</span>
                    </div>
                  ) : isRefinementCollapsed ? (
                    <div className="refinement-collapsed">
                      <div className="refinement-summary">
                        <strong>
                          {facets.length} filter group{facets.length === 1 ? "" : "s"} ready
                        </strong>
                        <span>
                          {totalFacetValues} option{totalFacetValues === 1 ? "" : "s"} available.
                        </span>
                      </div>
                      <div className="refinement-preview-list" aria-hidden="true">
                        {facets.slice(0, 4).map((group) => (
                          <span className="refinement-preview-pill" key={group.id}>
                            {group.title}
                          </span>
                        ))}
                      </div>
                    </div>
                  ) : (
                    <div className="facet-groups facet-groups-inline">
                      {facets.map((group) => (
                        <section className="facet-group" key={group.id}>
                          <div className="facet-group-header">
                            <h3>{group.title}</h3>
                          </div>
                          <div className="facet-values facet-values-inline">
                            {group.values.map((value) => (
                              <button
                                type="button"
                                key={value.id}
                                className="facet-value"
                                onClick={() => void applyFacet(group, value)}
                                disabled={isSearching}
                              >
                                <span className="facet-value-label">{value.label}</span>
                                <span className="facet-value-count">{value.count}</span>
                              </button>
                            ))}
                          </div>
                        </section>
                      ))}
                    </div>
                  )}
              </section>

              <section
                id="workspace-panel-downloads"
                role="tabpanel"
                aria-labelledby="workspace-tab-downloads"
                hidden={workspaceView !== "downloads"}
                className="panel utility-panel"
              >
                  <div className="panel-header utility-panel-header">
                    <div>
                      <p className="section-kicker">Downloads</p>
                      <h2>Queue monitor</h2>
                      <p className="panel-note">Track progress and retry failures.</p>
                    </div>
                    <div className="utility-panel-actions">
                      <p className="panel-note utility-panel-meta">
                        Max {MAX_CONCURRENT_DOWNLOADS} at once. Folder:{" "}
                        {downloadDestinationRef.current ?? "Not chosen yet"}.
                      </p>
                      <button
                        type="button"
                        className="ghost-button"
                        onClick={() => showResultsView({ focusTab: true })}
                      >
                        Back to results
                      </button>
                    </div>
                  </div>

                  {downloadJobs.length === 0 ? (
                    <div className="empty-state compact">
                      <p>No downloads queued.</p>
                      <span>Select papers to start a batch.</span>
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
            </div>
        </div>
        ) : null}
      </section>
    </main>
  );
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
