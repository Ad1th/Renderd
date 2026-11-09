// Turning raw numbers and addresses into what the window prints.

const BLANK = "--";

/** Frames per second as a whole number. */
export function fps(value) {
  return Number.isFinite(value) ? String(Math.round(value)) : BLANK;
}
