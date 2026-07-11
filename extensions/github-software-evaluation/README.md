# GitHub Software Evaluation extension

A zero-build Manifest V3 extension that adds a bounded analysis panel to metadata-confirmed public GitHub repository pages.

## Load for development

1. Run `cargo run --release --bin sevald -- serve` from the repository root; it listens on `127.0.0.1:7077` by default.
2. Enable developer mode on the browser's extension management page.
3. Choose **Load unpacked** and select this directory.

There is no build step, dependency, popup, or options page. The background service worker is the only network boundary. The extension never fetches or executes repository code.

## Production origin cutover

The development API origin appears in exactly two security-sensitive places: `host_permissions` in `manifest.json` and `apiOrigin` in `config.js`. Replace both in one atomic change with the same single HTTPS origin—for example, `https://analysis.example.com/*` in the manifest and `https://analysis.example.com` in config. Never deploy a revision where only one value changed, and never broaden either value to a wildcard host.

## Limits

Results describe one immutable default-branch snapshot and retain each analyzer's coverage and limitations. The five shape-trace rows use independent units and ranges; their lengths are not a shared scale, grade, ranking, or comparison. Analysis remains bounded by the service's archive, queue, cache, response-size, and time limits.
