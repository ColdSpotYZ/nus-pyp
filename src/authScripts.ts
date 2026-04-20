import type { ExamPaperResult, SearchRequest } from "./types";

const COLLECTION_URL = "https://digitalgems.nus.edu.sg/browse/collection/31";

interface DownloadScriptInput {
  jobId: string;
  destinationDirectory: string;
  requestedName: string;
  result?: ExamPaperResult;
}

function serialize<T>(value: T) {
  return JSON.stringify(value).replace(/</g, "\\u003c");
}

export function buildSearchScript(request: SearchRequest) {
  return `
    (async () => {
      const request = ${serialize(request)};
      const collectionUrl = ${serialize(COLLECTION_URL)};
      const emit = async (event, payload) => {
        if (!window.__TAURI__?.event?.emit) {
          throw new Error("Tauri global API is unavailable in the auth window.");
        }
        await window.__TAURI__.event.emit(event, payload);
      };

      const wait = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
      const textOf = (node) => (node?.textContent || "").replace(/\\s+/g, " ").trim();
      const waitForSelector = async (selector, attempts = 40, interval = 500) => {
        for (let attempt = 0; attempt < attempts; attempt += 1) {
          const element = document.querySelector(selector);
          if (element) {
            return element;
          }
          await wait(interval);
        }
        return null;
      };
      const clickByText = (tags, text) => {
        const wanted = text.toLowerCase();
        const elements = Array.from(document.querySelectorAll(tags.join(",")));
        const match = elements.find((element) => textOf(element).toLowerCase() === wanted);
        if (match instanceof HTMLElement) {
          match.click();
          return true;
        }
        return false;
      };
      const dispatch = (element) => {
        element.dispatchEvent(new Event("input", { bubbles: true }));
        element.dispatchEvent(new Event("change", { bubbles: true }));
      };
      const waitForResults = async () => {
        for (let attempt = 0; attempt < 40; attempt += 1) {
          const anchors = Array.from(document.querySelectorAll("a[href*='/view/']"));
          const noResults = document.body.innerText.toLowerCase().includes("no results");
          if (anchors.length > 0 || noResults) {
            return;
          }
          await wait(500);
        }
      };
      const parseField = (text, label) => {
        const pattern = new RegExp(label + "\\\\s*:?\\\\s*([^\\\\n]+)", "i");
        return text.match(pattern)?.[1]?.trim();
      };
      const scrapeResults = () => {
        const anchors = Array.from(document.querySelectorAll("a[href*='/view/']")).filter(
          (anchor, index, array) =>
            array.findIndex((candidate) => candidate.getAttribute("href") === anchor.getAttribute("href")) === index,
        );

        const results = anchors.map((anchor) => {
          const card =
            anchor.closest("article, li, .card, .list-group-item, .row, .search-result-item") ||
            anchor.parentElement ||
            anchor;
          const cardText = textOf(card);
          const title = textOf(anchor) || parseField(cardText, "Title") || "Untitled paper";
          const href = anchor.href;
          const downloadAnchor = Array.from(card.querySelectorAll("a, button")).find((element) => {
            const hrefValue = element.getAttribute("href") || "";
            const label = textOf(element).toLowerCase();
            return (
              hrefValue.toLowerCase().includes("download") ||
              hrefValue.toLowerCase().endsWith(".pdf") ||
              label.includes("download")
            );
          });
          const unavailableReason = cardText.includes("Paper not released by department")
            ? "Paper not released by department."
            : undefined;

          return {
            id: href || title + "-" + Math.random().toString(36).slice(2),
            title,
            courseCode: parseField(cardText, "Course Code"),
            courseName: parseField(cardText, "Course Name"),
            year: parseField(cardText, "Year of Examination"),
            semester: parseField(cardText, "Semester"),
            viewUrl: href,
            downloadUrl:
              downloadAnchor?.getAttribute("href")
                ? new URL(downloadAnchor.getAttribute("href"), location.origin).toString()
                : undefined,
            downloadable: !unavailableReason,
            unavailableReason,
          };
        });

        const nextButton = Array.from(document.querySelectorAll("a, button")).find((element) => {
          const label = textOf(element).toLowerCase();
          const disabled =
            element.hasAttribute("disabled") ||
            element.getAttribute("aria-disabled") === "true" ||
            element.classList.contains("disabled");
          return !disabled && (label === "next" || label === "load more");
        });

        return {
          results,
          hasMore: Boolean(nextButton),
          cursor: nextButton ? "next" : null,
        };
      };

      if (!location.hostname.includes("digitalgems.nus.edu.sg")) {
        await emit("auth:login-required", {
          message: "The background session is no longer on Digital Gems. Re-open login and authenticate again.",
        });
        return;
      }

      if (location.pathname !== "/browse/collection/31") {
        location.href = collectionUrl;
        const ready = await waitForSelector(".search-field, button, a");
        if (!ready) {
          throw new Error("Could not load the Examination Papers collection page.");
        }
        await wait(400);
      }

      if (request.cursor === "next") {
        const nextButton = Array.from(document.querySelectorAll("a, button")).find((element) => {
          const label = textOf(element).toLowerCase();
          const disabled =
            element.hasAttribute("disabled") ||
            element.getAttribute("aria-disabled") === "true" ||
            element.classList.contains("disabled");
          return !disabled && (label === "next" || label === "load more");
        });
        if (!(nextButton instanceof HTMLElement)) {
          await emit("search:page-loaded", { results: [], hasMore: false, cursor: null });
          return;
        }
        nextButton.click();
        await waitForResults();
        await emit("search:page-loaded", scrapeResults());
        return;
      }

      clickByText(["a", "button"], "Advanced");
      await wait(300);
      clickByText(["a", "button"], "Clear");
      await wait(300);

      await waitForSelector(".search-field");

      for (let index = 1; index < request.criteria.length; index += 1) {
        const addMoreButton = Array.from(document.querySelectorAll("a, button")).find(
          (element) => textOf(element).toLowerCase() === "add more criteria",
        );
        if (addMoreButton instanceof HTMLElement) {
          addMoreButton.click();
          await wait(150);
        }
      }

      const rows = Array.from(document.querySelectorAll(".search-field"));
      for (let index = 0; index < request.criteria.length; index += 1) {
        const criterion = request.criteria[index];
        const row = rows[index];
        if (!row) {
          continue;
        }
        const fieldSelect = row.querySelector("select[name='search-field-metadata[]']");
        const conditionSelect = row.querySelector("select[name='search-operator[]']");
        const operatorSelect = row.querySelector("select[name='search-field-operator[]']");
        if (fieldSelect instanceof HTMLSelectElement) {
          fieldSelect.value = criterion.field;
          dispatch(fieldSelect);
        }
        if (conditionSelect instanceof HTMLSelectElement) {
          conditionSelect.value = criterion.condition;
          dispatch(conditionSelect);
        }
        if (operatorSelect instanceof HTMLSelectElement) {
          operatorSelect.value = criterion.operator;
          dispatch(operatorSelect);
        }

        await wait(100);

        if (criterion.operator === "range") {
          const inputs = row.querySelectorAll("input[name='search-field-value[]'], input[name='search-field-value2[]']");
          const minInput = inputs[0];
          const maxInput = inputs[1];
          if (minInput instanceof HTMLInputElement) {
            minInput.disabled = false;
            minInput.value = criterion.value || "";
            dispatch(minInput);
          }
          if (maxInput instanceof HTMLInputElement) {
            maxInput.disabled = false;
            maxInput.value = criterion.value2 || "";
            dispatch(maxInput);
          }
          continue;
        }

        if (criterion.operator === "terms") {
          const input = row.querySelector("input.multiValue, input[name='search-field-value[]']");
          if (input instanceof HTMLInputElement) {
            input.disabled = false;
            input.value = (criterion.values || []).join(",");
            dispatch(input);
          }
          continue;
        }

        const valueInput = row.querySelector("input[name='search-field-value[]']");
        if (valueInput instanceof HTMLInputElement) {
          valueInput.disabled = false;
          valueInput.value = criterion.value || "";
          dispatch(valueInput);
        }
      }

      const searchButton = Array.from(document.querySelectorAll("a, button")).find(
        (element) => textOf(element).toLowerCase() === "search",
      );

      if (!(searchButton instanceof HTMLElement)) {
        throw new Error("Could not find the Digital Gems search button.");
      }

      searchButton.click();
      await waitForResults();
      await emit("search:page-loaded", scrapeResults());
    })();
  `;
}

export function buildDownloadScript(input: DownloadScriptInput) {
  return `
    (async () => {
      const payload = ${serialize(input)};
      const emit = async (event, data) => {
        if (!window.__TAURI__?.event?.emit) {
          throw new Error("Tauri global API is unavailable in the auth window.");
        }
        await window.__TAURI__.event.emit(event, data);
      };
      const invoke = async (command, args) => {
        if (!window.__TAURI__?.core?.invoke) {
          throw new Error("Tauri invoke API is unavailable in the auth window.");
        }
        return window.__TAURI__.core.invoke(command, args);
      };

      window.__NUS_PYP_CANCELLED__ = window.__NUS_PYP_CANCELLED__ || {};
      window.__NUS_PYP_CANCELLED__[payload.jobId] = false;
      const isCancelled = () => Boolean(window.__NUS_PYP_CANCELLED__[payload.jobId]);

      const fail = async (message, cancelled = false) => {
        await emit("download:failed", {
          jobId: payload.jobId,
          message,
          cancelled,
        });
      };

      try {
        if (!payload.result?.viewUrl) {
          throw new Error("Missing result metadata for the requested download.");
        }

        const resolveDownloadUrl = async () => {
          if (payload.result.downloadUrl) {
            return payload.result.downloadUrl;
          }

          const parseHtml = async (url) => {
            const response = await fetch(url, { credentials: "include" });
            if (!response.ok) {
              throw new Error("Unable to open the Digital Gems viewer page.");
            }
            const contentType = response.headers.get("content-type") || "";
            if (contentType.toLowerCase().includes("application/pdf")) {
              return { directPdfUrl: url, doc: null };
            }
            const html = await response.text();
            const parser = new DOMParser();
            return {
              directPdfUrl: null,
              doc: parser.parseFromString(html, "text/html"),
            };
          };

          const viewPage = await parseHtml(payload.result.viewUrl);
          if (viewPage.directPdfUrl) {
            return viewPage.directPdfUrl;
          }

          const iframe = viewPage.doc?.querySelector(
            "iframe.viewer-iframe, iframe#First-Iframe, .container-iframe iframe",
          );
          const iframeSrc = iframe?.getAttribute("src");
          if (!iframeSrc) {
            throw new Error("Viewer iframe not found on the Digital Gems paper page.");
          }

          const viewerUrl = new URL(iframeSrc, payload.result.viewUrl).toString();
          const viewerPage = await parseHtml(viewerUrl);
          if (viewerPage.directPdfUrl) {
            return viewerPage.directPdfUrl;
          }

          const nestedCandidate = viewerPage.doc?.querySelector(
            "iframe[src], embed[src], object[data], a[href$='.pdf'], a[href*='download']",
          );
          const nestedSrc =
            nestedCandidate?.getAttribute("src") ||
            nestedCandidate?.getAttribute("data") ||
            nestedCandidate?.getAttribute("href");

          if (!nestedSrc) {
            // The viewer endpoint itself is often the PDF source even when it serves inline.
            return viewerUrl;
          }

          return new URL(nestedSrc, viewerUrl).toString();
        };

        const downloadUrl = await resolveDownloadUrl();
        const response = await fetch(downloadUrl, { credentials: "include" });
        if (!response.ok || !response.body) {
          throw new Error("Digital Gems did not return a downloadable file.");
        }

        const contentLength = Number(response.headers.get("content-length") || "0") || undefined;
        const reader = response.body.getReader();
        const chunks = [];
        let bytesReceived = 0;

        while (true) {
          if (isCancelled()) {
            await fail("Download cancelled.", true);
            return;
          }

          const { done, value } = await reader.read();
          if (done) {
            break;
          }
          if (!value) {
            continue;
          }
          chunks.push(value);
          bytesReceived += value.length;
          await emit("download:progress", {
            jobId: payload.jobId,
            bytesReceived,
            bytesTotal: contentLength,
            progressPercent: contentLength
              ? Math.round((bytesReceived / contentLength) * 100)
              : undefined,
          });
        }

        const totalLength = chunks.reduce((sum, chunk) => sum + chunk.length, 0);
        const fileBytes = new Uint8Array(totalLength);
        let offset = 0;
        for (const chunk of chunks) {
          fileBytes.set(chunk, offset);
          offset += chunk.length;
        }

        const finalPath = await invoke("prepare_download_path", {
          directory: payload.destinationDirectory,
          requestedName: payload.requestedName,
        });

        await invoke("write_binary_file", {
          path: finalPath,
          bytes: Array.from(fileBytes),
        });

        await emit("download:completed", {
          jobId: payload.jobId,
          destinationPath: finalPath,
        });
      } catch (error) {
        await fail(error instanceof Error ? error.message : "Unexpected download failure.");
      }
    })();
  `;
}

export function buildCancelDownloadScript(jobId: string) {
  return `
    (() => {
      window.__NUS_PYP_CANCELLED__ = window.__NUS_PYP_CANCELLED__ || {};
      window.__NUS_PYP_CANCELLED__[${serialize(jobId)}] = true;
    })();
  `;
}
