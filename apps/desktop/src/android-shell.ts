import type { ExportArtifact, LocaleCode } from "@stiksound/core";
import type { ShellBridge, UiLabels } from "@stiksound/ui";

type InvokeFn = (cmd: string, args: Record<string, unknown>) => Promise<unknown>;

export function isAndroidTauri(win: { navigator: { userAgent: string } } & object): boolean {
  return "__TAURI_INTERNALS__" in win && /Android/i.test(win.navigator.userAgent);
}

/** Android WebView has no navigator.share for files, so Share/Save go through native Rust+Kotlin commands. */
export function createAndroidShell(invokeFn: InvokeFn): ShellBridge {
  const save = (fileName: string, mimeType: string, bytes: Uint8Array) =>
    invokeFn("cmd_save_to_downloads", { fileName, mimeType, bytes: Array.from(bytes) });

  return {
    async shareArtifact(artifact: ExportArtifact) {
      try {
        await invokeFn("cmd_share_file", {
          fileName: artifact.fileName,
          mimeType: artifact.mimeType,
          bytes: Array.from(artifact.bytes)
        });
        return "shared";
      } catch (err) {
        if (/cancel/i.test(String(err))) return "cancelled";
        throw err;
      }
    },
    async saveArtifact(artifact) {
      await save(artifact.fileName, artifact.mimeType, artifact.bytes);
    },
    async saveProjectZip(zipBytes, suggestedName) {
      await save(suggestedName, "application/zip", zipBytes);
    }
  };
}

export const androidLabels: Record<LocaleCode, Partial<UiLabels>> = {
  es: {
    stickerDropTitle: "Toca para elegir tu sticker",
    stickerDropHint: "Foto, GIF o video corto desde la galería o Archivos",
    hoverCropHint: "Arrastra para encuadrar",
    exportTooltip: "Crear el video y compartirlo",
    copyAgainBtn: "Compartir",
    downloadTooltip: "Guardar",
    downloadBtnLabel: "Guardar",
    downloadChooseFormat: "Elige qué guardar:",
    downloadFmtMp4: "Video MP4 (5s / 10s)",
    exportSuccessVideo: "Listo. Guardado en Descargas/StikSound."
  },
  en: {
    stickerDropTitle: "Tap to choose your sticker",
    stickerDropHint: "Photo, GIF or short video from Gallery or Files",
    hoverCropHint: "Drag to frame",
    exportTooltip: "Make the video and share it",
    copyAgainBtn: "Share",
    downloadTooltip: "Save",
    downloadBtnLabel: "Save",
    downloadChooseFormat: "Choose what to save:",
    downloadFmtMp4: "MP4 video (5s / 10s)",
    exportSuccessVideo: "Done. Saved to Downloads/StikSound."
  }
};
