import { spawn } from "child_process";

export function audioPlaybackCommand(filename: string, platform = process.platform) {
  if (platform === "darwin") return { executable: "afplay", args: [filename] };
  if (platform !== "win32") throw new Error(`Audio playback is not supported on ${platform}`);

  // WPF's MediaPlayer decodes MP3 on stock Windows without an extra player.
  // Pump the STA dispatcher so MediaOpened/MediaFailed events are delivered.
  const escapedFilename = filename.replace(/'/g, "''");
  const script = `
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName PresentationCore
$player = New-Object System.Windows.Media.MediaPlayer
$dispatcher = [System.Windows.Threading.Dispatcher]::CurrentDispatcher
$script:playbackError = $null
$player.add_MediaFailed({ $script:playbackError = $args[1].ErrorException })
try {
  $player.Open([Uri]::new('${escapedFilename}', [UriKind]::Absolute))
  $deadline = [DateTime]::UtcNow.AddSeconds(15)
  while (-not $player.NaturalDuration.HasTimeSpan) {
    $dispatcher.Invoke([Action]{}, [System.Windows.Threading.DispatcherPriority]::Background)
    if ($script:playbackError) { throw $script:playbackError }
    if ([DateTime]::UtcNow -gt $deadline) { throw 'Audio player timed out opening the MP3' }
    Start-Sleep -Milliseconds 50
  }
  $player.Play()
  $deadline = [DateTime]::UtcNow.AddSeconds(120)
  while ($player.Position -lt $player.NaturalDuration.TimeSpan) {
    $dispatcher.Invoke([Action]{}, [System.Windows.Threading.DispatcherPriority]::Background)
    if ($script:playbackError) { throw $script:playbackError }
    if ([DateTime]::UtcNow -gt $deadline) { throw 'Audio playback timed out' }
    Start-Sleep -Milliseconds 50
  }
} finally {
  $player.Close()
}
`;
  return {
    executable: "powershell.exe",
    args: [
      "-NoLogo",
      "-NoProfile",
      "-NonInteractive",
      "-STA",
      "-EncodedCommand",
      Buffer.from(script, "utf16le").toString("base64"),
    ],
  };
}

export async function playAudioFile(filename: string): Promise<void> {
  const command = audioPlaybackCommand(filename);
  await new Promise<void>((resolve, reject) => {
    const child = spawn(command.executable, command.args, { windowsHide: true, stdio: ["ignore", "ignore", "pipe"] });
    let stderr = "";
    child.stderr?.on("data", (chunk: Buffer) => {
      stderr = (stderr + chunk.toString("utf8")).slice(-4000);
    });
    child.once("error", reject);
    child.once("close", (code) => {
      if (code === 0) resolve();
      else reject(new Error(`Audio playback failed (${code}): ${stderr}`));
    });
  });
}
