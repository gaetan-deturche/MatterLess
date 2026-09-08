// The frontend's half of the timeline.
//
// Rust knows what it returned; only the shell knows what it did next. Routing
// both into one file is what lets a session be debugged by reading text instead
// of squinting at a screenshot.
//
// Never pass message text: structure, ids, counts and outcomes only.
import { invoke } from "@tauri-apps/api/core";

type Level = "info" | "warn" | "error" | "debug";

function send(level: Level, event: string, detail?: Record<string, unknown>) {
  const flat = detail
    ? Object.entries(detail)
        .map(([key, value]) => `${key}=${String(value)}`)
        .join(" ")
    : undefined;
  // Fire and forget: logging must never be able to break a user action.
  void invoke("ui_log", { level, event, detail: flat ?? null }).catch(() => {});
}

export const info = (event: string, detail?: Record<string, unknown>) =>
  send("info", event, detail);
export const warn = (event: string, detail?: Record<string, unknown>) =>
  send("warn", event, detail);
export const error = (event: string, detail?: Record<string, unknown>) =>
  send("error", event, detail);
export const debug = (event: string, detail?: Record<string, unknown>) =>
  send("debug", event, detail);

export const logPath = () => invoke<string>("where_is_the_log");

/** Reports an unexpected throw with its shape, not just its message. */
export function failure(event: string, thrown: unknown, detail?: Record<string, unknown>) {
  send("error", event, { ...detail, error: String(thrown) });
}
