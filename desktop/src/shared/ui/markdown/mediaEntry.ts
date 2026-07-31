/**
 * Pure classification of a markdown media URL: whether it should render as a
 * video, and whether it can be downloaded — two related but independent
 * questions.
 *
 * `isVideoMedia` decides the render path (video player vs. image block).
 * `isInlineableMediaUrl` decides whether a markdown *link* (`[…](url)` or a
 * bare autolink) should render as inline media instead of an external browser
 * link — required for private-pilot media GET auth (raw browser GETs 401).
 * `isRelayDownloadable` decides download eligibility, which is separate: an
 * off-relay video still renders as a video and can be Copy-Link'd, it just
 * can't be downloaded (the Rust `validate_download_url` SSRF gate accepts only
 * relay `/media/` origins, so offering Download for an external URL would only
 * produce an error).
 *
 * Kept DOM-free so the branch logic is unit-testable without a webview.
 */

/** Legacy video extensions, used only when an imeta MIME type is absent. */
const VIDEO_EXTENSIONS = ["mp4", "webm", "mov"] as const;

/** Image extensions for bare-link / autolink inline promotion (no imeta MIME). */
const IMAGE_EXTENSIONS = [
  "png",
  "jpg",
  "jpeg",
  "gif",
  "webp",
  "avif",
] as const;

/** The lowercased path extension of a URL, ignoring query strings and hashes. */
function urlPathExtension(src: string): string | undefined {
  let pathname: string;
  try {
    pathname = new URL(src).pathname;
  } catch {
    // Relative or malformed URL — strip query/hash by hand.
    pathname = src.split(/[?#]/, 1)[0];
  }
  const lastDot = pathname.lastIndexOf(".");
  if (lastDot < 0 || lastDot === pathname.length - 1) return undefined;
  return pathname.slice(lastDot + 1).toLowerCase();
}

/**
 * Whether `src` should render as a video.
 *
 * The imeta MIME type is authoritative when present (uploads tag every
 * attachment with `m`): a `video/*` MIME renders as video, and any other MIME
 * renders as an image regardless of the URL extension. Only when the MIME is
 * absent (legacy events that predate the tag) do we fall back to a path
 * extension check.
 */
export function isVideoMedia(src: string, imetaMime?: string): boolean {
  if (imetaMime) return imetaMime.toLowerCase().startsWith("video/");
  const ext = urlPathExtension(src);
  return (
    ext !== undefined && (VIDEO_EXTENSIONS as readonly string[]).includes(ext)
  );
}

/**
 * Whether a markdown link `href` should render as inline image/video instead
 * of an external OS-browser link.
 *
 * MIME wins when present (image/* and video/* only). Without MIME, known
 * image/video path extensions qualify. Non-media MIMEs (PDF, etc.) stay as
 * FileCard / external links.
 */
export function isInlineableMediaUrl(src: string, imetaMime?: string): boolean {
  if (imetaMime) {
    const mime = imetaMime.toLowerCase();
    return mime.startsWith("image/") || mime.startsWith("video/");
  }
  const ext = urlPathExtension(src);
  if (!ext) return false;
  return (
    (IMAGE_EXTENSIONS as readonly string[]).includes(ext) ||
    (VIDEO_EXTENSIONS as readonly string[]).includes(ext)
  );
}

/**
 * Whether `src` is a relay-hosted `/media/` URL on `relayOrigin`.
 *
 * This mirrors the Rust `validate_download_url` origin+path check for UX
 * purposes only — the Rust gate remains the authoritative SSRF boundary.
 *
 * Fails closed when the relay origin has not resolved yet (`relayOrigin`
 * absent): an unresolved origin means we can't distinguish a relay `/media/`
 * URL from an off-relay one, so we do NOT offer Download (offering it for an
 * off-relay URL would only produce an error, and the Rust gate would reject
 * it anyway). Callers must read `relayOrigin` from a reactive source
 * (`useRelayOrigin`) so eligibility recomputes — and Download appears for a
 * genuine relay URL — the moment the origin resolves.
 */
export function isRelayDownloadable(
  src: string,
  relayOrigin?: string,
): boolean {
  if (!relayOrigin) return false;
  let parsed: URL;
  try {
    parsed = new URL(src);
  } catch {
    return false;
  }
  if (!parsed.pathname.startsWith("/media/")) return false;
  return parsed.origin === relayOrigin;
}
