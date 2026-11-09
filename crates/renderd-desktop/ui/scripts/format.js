// Turning raw numbers and addresses into what the window prints.

const BLANK = "--";

/** Frames per second as a whole number. */
export function fps(value) {
  return Number.isFinite(value) ? String(Math.round(value)) : BLANK;
}

/** A bitrate given in kbit/s, shown in Mbit/s with one decimal. */
export function mbps(kbps) {
  if (!Number.isFinite(kbps)) return BLANK;
  const value = kbps / 1000;
  return value >= 100 ? String(Math.round(value)) : value.toFixed(1);
}

/** Milliseconds: one decimal while small, whole numbers once it matters less. */
export function ms(value) {
  if (!Number.isFinite(value)) return BLANK;
  return value < 10 ? value.toFixed(1) : String(Math.round(value));
}

/** The picture size, like 2560x1440 with a proper multiplication sign. */
export function size(stream) {
  return stream ? `${stream.width}×${stream.height}` : "";
}

/** Codec names the way people write them. */
export function codec(name) {
  if (name === "hevc") return "HEVC";
  if (name === "h264") return "H.264";
  return name ? name.toUpperCase() : "";
}

/** Drops the port from `ip:port`, including the bracketed IPv6 form. */
export function hostOnly(address) {
  if (!address) return "";
  const bracketed = address.match(/^\[(.+)\]:\d+$/);
  if (bracketed) return bracketed[1];
  const parts = address.split(":");
  return parts.length === 2 ? parts[0] : address;
}
