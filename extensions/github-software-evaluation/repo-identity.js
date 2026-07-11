(() => {
  "use strict";
  const OWNER = /^[A-Za-z0-9](?:[A-Za-z0-9-]{0,37}[A-Za-z0-9])?$/;
  const REPO = /^(?!\.{1,2}$)[A-Za-z0-9._-]{1,100}$/;
  const RESERVED = new Set(["about","account","apps","codespaces","collections","contact","customer-stories","events","explore","features","issues","login","marketplace","new","notifications","organizations","orgs","pricing","pulls","search","security","settings","site","sponsors","topics","trending","users"]);
  function parseNwo(value) {
    if (typeof value !== "string" || value.length > 140) return null;
    const p = value.split("/");
    if (p.length !== 2 || !OWNER.test(p[0]) || !REPO.test(p[1]) || RESERVED.has(p[0].toLowerCase())) return null;
    return Object.freeze({ owner:p[0], repo:p[1] });
  }
  function identityFromDocument(doc, loc) {
    if (!doc || !loc || loc.protocol !== "https:" || loc.hostname !== "github.com" || loc.search) return null;
    const p = String(loc.pathname || "").split("/").filter(Boolean);
    const route = p.length >= 2 ? parseNwo(`${p[0]}/${p[1]}`) : null;
    const meta = doc.querySelector('meta[name="octolytics-dimension-repository_nwo"]');
    const confirmed = parseNwo(meta && meta.getAttribute("content"));
    if (!route || !confirmed || confirmed.owner.toLowerCase() !== route.owner.toLowerCase() || confirmed.repo.toLowerCase() !== route.repo.toLowerCase()) return null;
    return Object.freeze({owner:confirmed.owner.toLowerCase(),repo:confirmed.repo.toLowerCase()});
  }
  globalThis.SEVAL_REPO_IDENTITY = Object.freeze({parseNwo,identityFromDocument});
})();
