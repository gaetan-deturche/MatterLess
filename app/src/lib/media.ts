// URLs for the authenticated `mmedia` scheme.
//
// The form differs by platform, and getting it wrong fails *silently*: nothing
// logs, no request is made, and every avatar quietly falls back to initials.
// That is exactly what happened -- `mmedia://localhost/...` reaches the handler
// on macOS and Linux but not on Windows, where Tauri maps a custom scheme onto
// `http://<scheme>.localhost/...`.
//
// `convertFileSrc` from the Tauri API is not used because it encodes the whole
// path with `encodeURIComponent`, which would turn the separators and the query
// into `%2F` and `%3F`.

const WINDOWS = navigator.userAgent.includes("Windows");

function url(path: string): string {
  return WINDOWS ? `http://mmedia.localhost/${path}` : `mmedia://localhost/${path}`;
}

/** A user's avatar.
 *
 *  `version` is the server's `last_picture_update`, and it is part of the URL
 *  rather than a header: a new picture arrives under the same user id, so
 *  without it the old face would stay cached for the life of the process.
 */
export const avatar = (userId: string, version = 0) =>
  url(`avatar/${encodeURIComponent(userId)}?v=${version}`);

/** A team's icon. Optional on the server, so a 404 here is ordinary. */
export const team = (teamId: string) => url(`team/${encodeURIComponent(teamId)}`);

/** A custom emoji's image. */
export const emoji = (emojiId: string) => url(`emoji/${encodeURIComponent(emojiId)}`);

/** An attachment, as uploaded. Cached on disk by the handler, so a second look
 *  costs nothing. */
export const file = (fileId: string) => url(`file/${encodeURIComponent(fileId)}`);

/** The server's JPEG thumbnail -- 20 KB against a 474 KB original, measured --
 *  which is what a message list should be asking for. */
export const thumb = (fileId: string) => url(`thumb/${encodeURIComponent(fileId)}`);

/** The server's larger JPEG, for a full-size look without the original's
 *  weight. */
export const preview = (fileId: string) => url(`preview/${encodeURIComponent(fileId)}`);
