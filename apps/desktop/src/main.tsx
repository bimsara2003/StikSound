import React from "react";
import { createRoot } from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";
import type {
  AudioProbeInfo,
  ExportArtifact,
  ImageProbeInfo,
  MediaEngine
} from "@stiksound/core";
import { WebMediaEngine } from "@stiksound/media-web";
import { MyInstantsProvider, type FetchLike } from "@stiksound/providers";
import { StikSoundMinimalApp, type ShellBridge } from "@stiksound/ui";
import "../../../packages/ui/src/styles.css";
import { androidLabels, createAndroidShell, isAndroidTauri } from "./android-shell";

function isTauriRuntime(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

const desktopFetch: FetchLike = async (input, init) => {
  const url = typeof input === "string" ? input : input instanceof URL ? input.toString() : input.url;
  if (!isTauriRuntime()) {
    return globalThis.fetch(input, init);
  }
  try {
    const bytes = await invoke<number[]>("cmd_fetch_myinstants", { url });
    return new Response(new Uint8Array(bytes), { status: 200 });
  } catch (err) {
    const msg = String(err ?? "");
    if (msg.includes("file-too-large")) {
      return new Response(null, { status: 413 });
    }
    if (msg.includes("url-rejected")) {
      return new Response(null, { status: 403 });
    }
    throw new Error(msg || "remote-unavailable");
  }
};

/**
 * Self-contained Desktop engine: uses Rust sniffing in Tauri and WebCodecs/MediaRecorder
 * inside the OS WebView so zero external FFmpeg installation is required.
 */
class DesktopMediaEngine extends WebMediaEngine implements MediaEngine {
  public override async probeImage(bytes: Uint8Array): Promise<ImageProbeInfo> {
    if (!isTauriRuntime()) return super.probeImage(bytes);
    return invoke<ImageProbeInfo>("cmd_probe_image", { bytes: Array.from(bytes) });
  }

  public override async probeAudio(bytes: Uint8Array): Promise<AudioProbeInfo> {
    if (!isTauriRuntime()) return super.probeAudio(bytes);
    return invoke<AudioProbeInfo>("cmd_probe_audio", { bytes: Array.from(bytes) });
  }
}

const shell: ShellBridge = {
  async saveArtifact(artifact: ExportArtifact) {
    if (isTauriRuntime()) {
      await invoke("cmd_save_export_to_downloads", {
        fileName: artifact.fileName,
        bytes: Array.from(artifact.bytes)
      });
      return;
    }
    const blob = new Blob([artifact.bytes.slice().buffer], { type: artifact.mimeType });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = artifact.fileName;
    a.click();
    setTimeout(() => URL.revokeObjectURL(url), 5000);
  },

  async copyArtifactToClipboard(artifact: ExportArtifact) {
    if (isTauriRuntime()) {
      await invoke("cmd_copy_artifact_to_clipboard", {
        fileName: artifact.fileName,
        bytes: Array.from(artifact.bytes)
      });
      return;
    }
    if (navigator.clipboard?.write && typeof ClipboardItem !== "undefined") {
      try {
        const blob = new Blob([artifact.bytes.slice().buffer], { type: artifact.mimeType });
        await navigator.clipboard.write([new ClipboardItem({ [artifact.mimeType]: blob })]);
      } catch {
        // Browser may restrict video MIME clipboard writes outside Tauri
      }
    }
  }
};

const isAndroid = isAndroidTauri(window);
const locale = navigator.language.toLowerCase().startsWith("es") ? "es" : "en";

createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <StikSoundMinimalApp
      engine={new DesktopMediaEngine()}
      providers={[new MyInstantsProvider(desktopFetch)]}
      shell={isAndroid ? createAndroidShell(invoke) : shell}
      initialLocale={locale}
      {...(isAndroid && {
        platform: { transparentVideo: false, myInstantsUrl: true, videoStickers: true },
        labelOverrides: androidLabels
      })}
    />
  </React.StrictMode>
);
