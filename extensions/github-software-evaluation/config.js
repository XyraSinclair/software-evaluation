(() => {
  "use strict";
  const config = Object.freeze({
    apiOrigin: "http://127.0.0.1:7077",
    requestTimeoutMs: 12_000,
    pollTimeoutMs: 120_000,
    pollMinDelayMs: 750,
    pollMaxDelayMs: 5_000,
    maxResponseBytes: 262_144,
    maxPollAttempts: 80
  });
  Object.defineProperty(globalThis, "SEVAL_CONFIG", { value: config, enumerable: true, configurable: false, writable: false });
})();
