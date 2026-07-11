(() => {
  "use strict";
  const HOST_ID = "seval-github-panel-host";

  function createController(env = { document, location, chrome }) {
    let generation = 0;
    let panel = null;
    let port = null;
    let scheduled = false;
    let lastKey = "";

    function remove() {
      generation += 1;
      if (port) { try { port.disconnect(); } catch (_) {} port = null; }
      const existing = env.document.getElementById(HOST_ID);
      if (existing) existing.remove();
      panel = null;
      lastKey = "";
    }

    function progressMessage(response) {
      const state = String(response && response.status || "analyzing").replaceAll("_", " ");
      const done = Number(response && response.progress && response.progress.completed_instruments);
      const total = Number(response && response.progress && response.progress.total_instruments);
      return Number.isFinite(done) && Number.isFinite(total) ? `${state} · ${done}/${total} instruments` : `${state}…`;
    }

    function limitations(result) {
      const out = [];
      for (const item of Array.isArray(result && result.metrics) ? result.metrics : []) {
        for (const text of Array.isArray(item.limitations) ? item.limitations : []) {
          if (typeof text === "string" && out.length < 8) out.push(`${item.id}: ${text}`);
        }
      }
      if (!out.length) out.push("Results cover one immutable default-branch snapshot.", "Each trace uses its own basis; lengths are not comparable.");
      return out;
    }

    function snapshotText(result) {
      const snapshot = result && result.snapshot;
      if (!snapshot) return "Snapshot unavailable";
      const commit = typeof snapshot.commit === "string" ? snapshot.commit.slice(0, 12) : "unknown";
      const bundle = snapshot.cache && typeof snapshot.cache.bundle_version === "string" ? ` · analyzer ${snapshot.cache.bundle_version}` : "";
      return `commit ${commit}${bundle}`;
    }

    function start(identity) {
      const myGeneration = ++generation;
      const key = `${identity.owner}/${identity.repo}`;
      if (lastKey === key && panel) return;
      remove();
      generation = myGeneration;
      lastKey = key;
      const panelApi = env.SEVAL_PANEL || globalThis.SEVAL_PANEL;
      panel = panelApi.createPanel(env.document, () => { lastKey = ""; reconcile(); });
      env.document.body.append(panel.host);
      panel.update({ state: "loading", identity, message: "Requesting bounded snapshot analysis…" });
      port = env.chrome.runtime.connect({ name: "seval-analysis" });
      port.onMessage.addListener((message) => {
        if (myGeneration !== generation || !panel) return;
        const envelope = message.result || {};
        const result = envelope.result && typeof envelope.result === "object" ? envelope.result : envelope;
        if (message.type === "progress") {
          panel.update({ state: "progress", identity, message: progressMessage(envelope), rows: panelApi.model(result), limitations: limitations(result) });
        } else if (message.type === "result") {
          if (envelope.status === "failed") {
            panel.update({ state: "error", identity, message: "Analysis failed.", retryable: false, limitations: limitations(result) });
            return;
          }
          panel.update({
            state: "ready",
            identity,
            message: envelope.status === "completed_partial" ? "Snapshot analysis complete with partial coverage." : "Snapshot analysis complete.",
            snapshot: snapshotText(result),
            cache: Boolean(result.snapshot && result.snapshot.cache && result.snapshot.cache.hit),
            rows: panelApi.model(result),
            limitations: limitations(result)
          });
        } else if (message.type === "error") {
          panel.update({ state: "error", identity, message: String(message.message || "Analysis could not be completed."), retryable: Boolean(message.retryable) });
        }
      });
      port.onDisconnect.addListener(() => { if (myGeneration === generation) port = null; });
      port.postMessage({ type: "analyze", requestedIdentity: true, owner: identity.owner, repo: identity.repo });
    }

    function reconcile() {
      scheduled = false;
      const parser = env.SEVAL_REPO_IDENTITY || globalThis.SEVAL_REPO_IDENTITY;
      const identity = parser.identityFromDocument(env.document, env.location);
      if (!identity) { remove(); return; }
      const key = `${identity.owner}/${identity.repo}`;
      if (key !== lastKey || !env.document.getElementById(HOST_ID)) start(identity);
    }
    function schedule() { if (!scheduled) { scheduled = true; queueMicrotask(reconcile); } }
    function stale() { remove(); }
    return Object.freeze({ reconcile, schedule, stale, remove, get generation() { return generation; } });
  }

  if (globalThis.__SEVAL_EXTENSION_TEST__) globalThis.SEVAL_CONTENT_TEST = Object.freeze({ createController });
  if (!globalThis.__SEVAL_EXTENSION_TEST__) {
    const controller = createController();
    document.addEventListener("turbo:before-render", controller.stale, true);
    document.addEventListener("turbo:load", controller.schedule, true);
    addEventListener("popstate", controller.schedule, true);
    new MutationObserver(controller.schedule).observe(document.documentElement, { childList: true, subtree: true });
    controller.schedule();
  }
})();
