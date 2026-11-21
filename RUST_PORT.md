# Google Translate 0.1.1 for Tezbar beta.12

## Independent review follow-up (source only)

The populated List puts each translation in `List.Item.Detail.props.markdown`.
Quick results also show pronunciation, preferring target romanization and using
source pronunciation when the target slot is empty. Rows without pronunciation
omit its Copy action. Instant Copy, Paste and View take the foreground selection
exclusively. Quick Translate Results still honors explicitly forwarded text,
including an empty string.

Translation failures return a visible successful protocol reply so the SDK
commits empty results and action targets. The HTTP client uses the Cargo package
version in its user-agent. Encoded vendor URLs through 2048 characters use GET.
Longer requests keep the language and service options in the URL and send only
`q` in the form-encoded POST body.

The new `tests/loopback.rs` fixture drives the SDK worker over stdio with a
loopback HTTP recorder and a recording host. It covers populated two-row output,
row-two copy, pronunciation and language actions, stale action rejection after
response failures, instant selection precedence, and GET/POST boundary cases
including multibyte text.

These changes have source-level verification only. Compilation, execution of the
regressions, beta.12 decoder and GPUI acceptance, and platform packaging remain
unverified for this revision. The parent owns the source freeze and serialized
builds. The artifact hashes and runtime results below describe earlier snapshots.


The standalone `tezbar-google-translate` binary uses schema 3 / process protocol 1
with the immutable Rust SDK revision in Cargo.toml and Cargo.lock. No JavaScript,
Node, Bun, webview or Raycast process is used. Tezbar's GPUI renderer displays its
List, Form and Detail models. Original TypeScript stays in `src/` as an oracle.

The current candidate implements the seven command IDs in upstream commit
`dccfc8877dd0e658295cb4b6d29b4a0a1b79baf8`: Translate (forward/reverse for a single target, multi-translate for saved multi-target sets),
Translate Form, Quick Translate (multiple targets), Quick Translate Results, Instant Translate Copy,
Instant Translate Paste, Instant Translate View. The last returns a native Detail
view instead of repeatedly showing a transient HUD. View commands can read selected
text when `autoInput` is enabled; instant commands require a nonempty selection.
Quick Translate retains the upstream no-view declaration and opens its results
directly in the initiating native session. Tezbar accepts the returned view without
requiring a cross-command launch. Quick Translate Results also opens independently
and honors `launchContext.text`, including an explicitly empty string. A supplied
launch text takes precedence over an initiating search and prevents a second read
of the user's selection. Each new session replaces previous input; the upstream
React request-ID reset is unnecessary for separate native sessions.

Copy, paste, original-text copy, pronunciation copy and opening the encoded Google
Translate website use typed host capabilities and propagate host failures.

Source (`langFrom`), primary (`lang1`) and secondary (`lang2`) target preferences,
default copy/paste action, cross-language priority and HTTP(S)/SOCKS proxy are used.
The full upstream language list is preserved. Change Languages opens a native form;
additional target codes are comma-separated, with at most eight targets per set.
Save Language Set and Manage Language Sets provide persistent add/select/delete
operations (up to 64 sets) in the extension-private `language-sets/storage.json`.
The selected set is shared by interactive commands; instant commands use preferences.
Legacy Raycast cached language sets are not imported automatically.

Translation calls Google's public `translate_a/single` endpoint over Rust HTTP,
with a 5-second connection timeout, 15-second request timeout, 5000-character input
limit, 1 MiB response limit and explicit HTTP/network/rate-limit/malformed-response
errors. The online service can change or rate-limit; no API key is required.
Search has a 350 ms debounce; SDK cancellation fences obsolete search results.
Requests use the configured proxy; errors avoid printing credentials or input text.

TTS downloads bounded MP3 chunks in Rust and plays through WinMM on Windows or
the system afplay utility on macOS. Unique temporary files are cleaned after playback.
Toggle Full Text, Swap Languages and previous/next saved language sets are actions.
List-wide language controls and saved-set navigation remain reachable when a
result row is selected. Keyboard shortcuts are not reproduced. Language-set
selection is managed through actions/forms instead of the original Raycast dropdown.

## Local verification and packaging

```powershell
$env:CARGO_BUILD_JOBS='1'
cargo test --locked
cargo build --locked
python scripts/vendor-licenses.py
python scripts/package.py --target x86_64-pc-windows-msvc --binary target/debug/tezbar-google-translate.exe --output dist
```

The packaging script verifies the executable's native PE32+ x64 or ARM64 Mach-O
target, protocol identity, exact command
coverage, immutable SDK lock and all vendored dependency notices. It inventories
all package members, produces real ZIP/executable SHA256 sidecars, and uploads
nothing. `dist/` and `target/` are ignored. The locally generated Windows archive
is a **debug validation package**, not a signed release. Build the release binary
locally on each target before publishing. macOS and real GPUI desktop integration
remain separate acceptance gates; no GitHub Actions jobs were run or added.

## Source and current validation boundary

The original imported TypeScript oracle remains unchanged. `reference/LATEST.json`
pins the current 36-file upstream snapshot, including each exact Git blob,
SHA-256, size and mode. Its bytes live under `reference/<commit>/`.
The new launch-context, multi-target and selected-row action regressions must pass
on both native platforms before publication. Source formatting and Python syntax
checks have passed; final executable gates for 0.1.1 are still pending at this edit.
Historical six-command Windows logs do not establish seven-command acceptance.

Tests include mock HTTP translation requests, segmented response parsing,
pronunciation, limits and language validation; actual child-process JSONL handshake,
native list/form views, capability denial, selection errors, stale action rejection,
session disposal and persistent language-set CRUD; and fake-host copy/paste/open
effects using authoritative results. They do not send real clipboard or UI effects.

## Completion audit (2026-11-16)

Live GitHub source hashes and oracle comparisons: metadata/upstream-verification.json.
Explicit-source double-way translation translates the original input in both directions;
auto-detection back-translates the first result. Multi-target saved sets use ordered
multi-translation. Deleting the selected saved set clears its persisted selection.

UI adaptations: form translation uses an explicit Translate submit action; Instant
Translate View returns a persistent Detail instead of repeated HUDs. Language management
uses native actions/forms rather than Raycast dropdown focus shortcuts. Existing
Raycast cached language sets are not migrated. No dictionary feature exists in the
inspected upstream command UI. Audio playback, actual Google service, macOS, and
desktop GPUI are not acceptance-tested here.

Final local verification: CARGO_BUILD_JOBS=1 cargo test --locked (9 unit/parity +
5 child-process protocol tests), cargo build --locked, cargo fmt --check,
git diff --check passed. Preferences are structurally identical to package.json;
all six original command IDs are preserved. Vendor notice check passed (114 sets).
Production importer and installed binary --protocol-info passed using the existing
Tezbar extension_package_cli.exe with disposable profile; see
 dist/production-import-report.json. Application/SDK/catalog/STATUS were not edited.
Parent STATUS records its pnpm typecheck and test results; those app-wide gates
were not rerun by this scoped implementation agent.

Development Windows debug ZIP: dist/google-translate-x86_64-pc-windows-msvc.zip
SHA256: e02cb04551322335e3e94a002f82f046d4328e31c22d4b55279b987903dd8be4
Executable SHA256: 15401e2e85946052bc061b6adc0dc8dc23e2d2b581f9eff323deec7a59901f32
No release, catalog pins or Loved integration were published.


## Google Translate live request remediation ? 2026-11-16

Reproduced old UI failure and actual RPC rate-limit error. Root cause: port
endpoint/client drift (googleapis/gtx HTTP 429); restored upstream google.com/
dict-chrome-ex with real HTTP 200 results. Added visible failure diagnostics and
no stale copy/paste results. Final ZIP SHA256
`d63b96de0918617c94c0b749449495ad7e23d692552a2b5415b72035e0794e97`.
16 regression tests + 2 explicitly run live tests PASS; six commands exercised
against Google via recording protocol host. Final ZIP production import and real
GPUI form Auto?English and French?English PASS; actual copy/paste to owned target
PASS before format-only rebuild. TTS native playback returned success.
pnpm typecheck PASS; pnpm test 750 PASS/3 skipped. Exact clipboard formats/bytes
restored; disposable profiles and owned processes cleaned. App source untouched.
See app `docs/rust-ports-beta9/google-translate-fix/REPORT.md` and evidence there.
This supersedes unknown-cause Bonjour failure, not all-eight acceptance or full
six-command desktop parity. IDs/preferences/protocol/schema/SDK lock unchanged.
