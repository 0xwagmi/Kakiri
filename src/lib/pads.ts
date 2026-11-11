/**
 * Pads: where they are stored and how their hotkeys get registered.
 *
 * This lives outside the Pads component because two places need it. The grid needs
 * to save edits, and the app needs the hotkeys armed from launch — not from the
 * first time someone opens that tab, which is how it used to work and why a hotkey
 * did nothing until you visited Pads.
 */
import { invoke } from "@tauri-apps/api/core";

export type Pad = {
  id: number;
  soundPath: string | null;
  name: string;
  hotkey: string;
  /** what the engine actually registered, e.g. `control+alt+Digit3` */
  canonical: string | null;
  error: boolean;
};

export const PAD_COUNT = 16;
const LS_PADS = "sb.pads";

export function defaultPads(): Pad[] {
  return Array.from({ length: PAD_COUNT }, (_, i) => ({
    id: i,
    soundPath: null,
    name: "",
    hotkey: "",
    canonical: null,
    error: false,
  }));
}

export function readPads(): Pad[] {
  try {
    const raw = localStorage.getItem(LS_PADS);
    const parsed = raw ? JSON.parse(raw) : null;
    if (Array.isArray(parsed) && parsed.length) return parsed as Pad[];
  } catch {
    /* a corrupt entry should not take the app down */
  }
  return defaultPads();
}

export function writePads(list: Pad[]) {
  try {
    localStorage.setItem(LS_PADS, JSON.stringify(list));
  } catch {
    /* storage full or blocked: the grid still works for this session */
  }
}

/**
 * Hand the accelerators to the engine and record what it accepted. An empty or
 * unparseable combo comes back as `canonical: null`, which the grid shows as a red
 * outline so the reason is visible instead of a key that silently never fires.
 */
export async function registerHotkeys(list: Pad[]): Promise<Pad[]> {
  try {
    const res = await invoke<string[]>("set_hotkeys", {
      accelerators: list.map((p) => p.hotkey || ""),
    });
    const next = list.map((p, i) => ({
      ...p,
      canonical: res[i] || null,
      error: !!p.hotkey.trim() && !res[i],
    }));
    writePads(next);
    return next;
  } catch {
    // no backend (browser preview) or the plugin refused wholesale
    writePads(list);
    return list;
  }
}

/** The pad bound to a registered accelerator, or -1. */
export function padForAccelerator(list: Pad[], accel: string): number {
  return list.findIndex((p) => !!p.canonical && p.canonical === accel);
}
