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
