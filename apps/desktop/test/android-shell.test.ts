import { describe, expect, it } from "vitest";
import type { ExportArtifact } from "@stiksound/core";
import { createAndroidShell, isAndroidTauri } from "../src/android-shell";

const mp4 = (bytes: number[] = [1, 2]): ExportArtifact => ({
  profile: "video-mp4",
  fileName: "s.mp4",
  mimeType: "video/mp4",
  bytes: new Uint8Array(bytes)
});

describe("android shell", () => {
  it("detects Android only inside Tauri", () => {
    expect(isAndroidTauri({ navigator: { userAgent: "Linux; Android 14" }, __TAURI_INTERNALS__: {} })).toBe(true);
    expect(isAndroidTauri({ navigator: { userAgent: "Linux; Android 14" } })).toBe(false);
    expect(isAndroidTauri({ navigator: { userAgent: "Windows NT 10.0" }, __TAURI_INTERNALS__: {} })).toBe(false);
  });

  it("shareArtifact invokes cmd_share_file with bytes array", async () => {
    const calls: unknown[] = [];
    const shell = createAndroidShell(async (c, a) => {
      calls.push([c, a]);
    });
    expect(await shell.shareArtifact!(mp4())).toBe("shared");
    expect(calls).toEqual([["cmd_share_file", { fileName: "s.mp4", mimeType: "video/mp4", bytes: [1, 2] }]]);
  });

  it("shareArtifact returns cancelled instead of throwing", async () => {
    const shell = createAndroidShell(async () => {
      throw new Error("cancelled");
    });
    expect(await shell.shareArtifact!(mp4([]))).toBe("cancelled");
  });

  it("saveArtifact and saveProjectZip invoke cmd_save_to_downloads", async () => {
    const calls: [string, Record<string, unknown>][] = [];
    const shell = createAndroidShell(async (c, a) => {
      calls.push([c, a]);
    });
    await shell.saveArtifact(mp4());
    await shell.saveProjectZip!(new Uint8Array([9]), "p.stiksound.zip");
    expect(calls).toEqual([
      ["cmd_save_to_downloads", { fileName: "s.mp4", mimeType: "video/mp4", bytes: [1, 2] }],
      ["cmd_save_to_downloads", { fileName: "p.stiksound.zip", mimeType: "application/zip", bytes: [9] }]
    ]);
  });
});
