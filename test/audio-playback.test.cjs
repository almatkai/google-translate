const { test } = require("node:test");
const assert = require("node:assert/strict");
const { readFileSync } = require("node:fs");
const { join } = require("node:path");
const { EventEmitter } = require("node:events");
const Module = require("node:module");
const ts = require("typescript");

// Use the extension's existing TypeScript compiler: no test-only dependencies.
function loadPlayback(spawn) {
  const filename = join(__dirname, "../src/audio-playback.ts");
  const source = ts.transpileModule(readFileSync(filename, "utf8"), {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 },
  }).outputText;
  const mod = new Module(filename, module);
  const originalRequire = mod.require.bind(mod);
  mod.require = (name) => (name === "child_process" ? { spawn } : originalRequire(name));
  mod._compile(source, filename);
  return mod.exports;
}

const { audioPlaybackCommand } = loadPlayback(() => {
  throw Error("unexpected spawn");
});

test("macOS uses afplay with a separate unquoted path argument", () => {
  assert.deepEqual(audioPlaybackCommand("/tmp/a b.mp3", "darwin"), { executable: "afplay", args: ["/tmp/a b.mp3"] });
});

test("Windows uses an encoded STA MediaPlayer script and safely quotes filenames", () => {
  const filename = "C:\\Users\\O'Brien\\音声 $file.mp3";
  const command = audioPlaybackCommand(filename, "win32");
  assert.equal(command.executable, "powershell.exe");
  assert.ok(command.args.includes("-STA"));
  const script = Buffer.from(command.args.at(-1), "base64").toString("utf16le");
  assert.ok(script.includes("O''Brien"));
  assert.ok(script.includes("音声 $file.mp3"));
  assert.ok(script.includes("System.Windows.Media.MediaPlayer"));
  assert.ok(script.includes("$player.Close()"));
  assert.ok(script.includes("add_MediaEnded"));
  assert.ok(script.includes("while (-not $script:playbackFinished)"));
});

test("unsupported platforms fail explicitly", () => {
  assert.throws(() => audioPlaybackCommand("/tmp/test.mp3", "unknown"), /not supported/);
});

function mockPlayer(event, value) {
  return () => {
    const child = new EventEmitter();
    child.stderr = new EventEmitter();
    queueMicrotask(() => child.emit(event, value));
    return child;
  };
}

test("playback resolves only after a successful player close", async () => {
  await loadPlayback(mockPlayer("close", 0)).playAudioFile("/tmp/fixture.mp3");
});

test("missing player rejects instead of emitting an unhandled error", async () => {
  await assert.rejects(
    loadPlayback(mockPlayer("error", new Error("ENOENT"))).playAudioFile("/tmp/fixture.mp3"),
    /ENOENT/,
  );
});

test("failed playback rejects rather than silently succeeding", async () => {
  await assert.rejects(loadPlayback(mockPlayer("close", 1)).playAudioFile("/tmp/fixture.mp3"), /Audio playback failed/);
});
