// A small stepped line of recent bitrates.

const WIDTH = 320;
const HEIGHT = 32;
const PAD = 5;

/** How many samples the line holds. One arrives per second. */
export const CAPACITY = 60;

/**
 * Builds the `points` attribute for a polyline from `samples`.
 *
 * The line is stepped rather than smoothed: each sample holds its value until
 * the next one, which is what the encoder actually does.
 */
export function points(samples) {
  if (samples.length < 2) return "";
  const peak = Math.max(...samples, 1);
  const step = WIDTH / (CAPACITY - 1);
  const offset = (CAPACITY - samples.length) * step;
  const y = (value) => (HEIGHT - PAD - (value / peak) * (HEIGHT - 2 * PAD)).toFixed(1);

  const out = [];
  samples.forEach((value, index) => {
    const x = (offset + index * step).toFixed(1);
    if (index > 0) out.push(`${x},${y(samples[index - 1])}`);
    out.push(`${x},${y(value)}`);
  });
  return out.join(" ");
}
