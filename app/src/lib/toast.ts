// Real Windows notifications, not in-app popups.
//
// `@tauri-apps/plugin-notification` calls the OS toast API, so these appear over
// whatever the person is doing and land in Action Center. They fire while
// MatterLess runs in the background; when it is not running at all nothing can
// fire, because that needs the Mattermost Push Proxy, which a third-party client
// cannot stand in for.
//
// The decision to notify is *not* made here. Rust has already applied the whole
// policy -- own posts, system messages, do-not-disturb, muted channels, the
// desktop level, unfollowed threads, and the channel being read while the window
// is focused. Anything arriving here is meant to interrupt.
import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
} from "@tauri-apps/plugin-notification";
import * as api from "./api";
import * as log from "./log";

let allowed: boolean | undefined;

/** Asked once and remembered: the prompt itself is an interruption. */
async function permitted(): Promise<boolean> {
  if (allowed !== undefined) return allowed;
  try {
    allowed = await isPermissionGranted();
    if (!allowed) allowed = (await requestPermission()) === "granted";
  } catch (thrown) {
    log.failure("toast.permission.failed", thrown);
    allowed = false;
  }
  return allowed;
}

export async function raise(
  channelId: string,
  author: string,
  channel: string,
  preview: string,
  direct = false,
) {
  if (!(await permitted())) {
    log.warn("toast.suppressed", { reason: "permission" });
    return;
  }
  // Conversation in the title, author with the message: matches what
  // Mattermost's own desktop app shows, so the shape is already familiar. For a
  // DM the title says what kind of message it is, because naming the
  // conversation would only repeat the author -- and the `@` marks that the
  // person in the body is who wrote to you, not where it landed.
  const title = channel || "MatterLess";
  const who = direct ? `@${author}` : author;
  const body = author ? `${who}: ${preview}` : preview;
  try {
    // Raised in Rust so that clicking it reaches the app: the notification
    // plugin registers no activation handler, which made its toasts inert.
    await api.raiseToast(channelId, title, body);
  } catch (thrown) {
    // Falling back rather than going silent: a toast nobody can click still
    // says a message arrived, which is the point of it.
    log.failure("toast.native.failed", thrown, { channel: channelId });
    try {
      sendNotification({ title, body });
    } catch (second) {
      log.failure("toast.failed", second, { channel: channelId });
    }
  }
}
