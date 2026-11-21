use std::sync::atomic::{AtomicU64, Ordering};
use tezbar_extension_sdk::ExtensionContext;
static NEXT: AtomicU64 = AtomicU64::new(0);
struct AudioFile(std::path::PathBuf);
impl Drop for AudioFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
pub async fn speak(
    ctx: &ExtensionContext,
    text: &str,
    lang: &str,
    proxy: &str,
) -> Result<(), String> {
    // Google TTS accepts short chunks. Keep Unicode scalar boundaries intact.
    let chars: Vec<char> = text.chars().collect();
    std::fs::create_dir_all(&ctx.paths.temp_dir).map_err(|_| "Cannot create audio directory")?;
    for chunk in chars.chunks(180) {
        let text: String = chunk.iter().collect();
        let mut response = super::translation::client(proxy)?
            .get("https://translate.google.com/translate_tts")
            .query(&[
                ("ie", "UTF-8"),
                ("client", "tw-ob"),
                ("tl", lang),
                ("q", &text),
            ])
            .send()
            .await
            .map_err(|_| "TTS download failed")?;
        if !response.status().is_success() {
            return Err(format!("TTS returned HTTP {}", response.status().as_u16()));
        }
        let mut data = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| "TTS download interrupted")?
        {
            if data.len() + chunk.len() > 2 * 1024 * 1024 {
                return Err("TTS response too large".into());
            }
            data.extend_from_slice(&chunk);
        }
        if data.is_empty() {
            return Err("Empty TTS audio".into());
        }
        let path = ctx.paths.temp_dir.join(format!(
            "tts-{}-{}.mp3",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let file = AudioFile(path);
        std::fs::write(&file.0, data).map_err(|_| "Cannot write TTS audio")?;
        tokio::task::spawn_blocking(move || {
            let result = play(&file.0);
            drop(file);
            result
        })
        .await
        .map_err(|_| "Audio task failed")??;
    }
    Ok(())
}
#[cfg(windows)]
fn play(path: &std::path::Path) -> Result<(), String> {
    #[link(name = "winmm")]
    extern "system" {
        fn mciSendStringW(
            command: *const u16,
            result: *mut u16,
            length: u32,
            callback: isize,
        ) -> u32;
    }
    fn send(command: &str) -> Result<(), String> {
        let wide: Vec<u16> = command.encode_utf16().chain(Some(0)).collect();
        let code = unsafe { mciSendStringW(wide.as_ptr(), std::ptr::null_mut(), 0, 0) };
        if code == 0 {
            Ok(())
        } else {
            Err(format!("Native audio playback failed ({code})"))
        }
    }
    let alias = format!("tezbar{}", NEXT.fetch_add(1, Ordering::Relaxed));
    send(&format!(
        "open \"{}\" type mpegvideo alias {alias}",
        path.display()
    ))?;
    let result = send(&format!("play {alias} wait"));
    let _ = send(&format!("close {alias}"));
    result
}
#[cfg(target_os = "macos")]
fn play(path: &std::path::Path) -> Result<(), String> {
    let status = std::process::Command::new("/usr/bin/afplay")
        .arg(path)
        .status()
        .map_err(|_| "Cannot start native audio player")?;
    if status.success() {
        Ok(())
    } else {
        Err("Native audio playback failed".into())
    }
}
#[cfg(not(any(windows, target_os = "macos")))]
fn play(_: &std::path::Path) -> Result<(), String> {
    Err("Unsupported audio platform".into())
}
