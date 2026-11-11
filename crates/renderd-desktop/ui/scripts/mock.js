// A pretend backend for opening index.html straight in a browser.
//
//   index.html?role=viewer&phase=streaming&platform=windows
//
// Query parameters pick the starting state. Pressing start walks through a
// session on a timer, so the motion can be looked at too.

const STREAM = { codec: "hevc", width: 2560, height: 1440, fps: 60 };

function emptyStatus(role) {
  return {
    role,
    phase: "stopped",
    address: null,
    peer: null,
    stream: null,
    metrics: { fps: null, bitrate_kbps: null, rtt_ms: null, send_queue_kb: null, decode_ms: null, lost_frames: null },
    problem: null,
  };
}

const PROBLEMS = {
  port: {
    kind: "port_in_use",
    title: "Another app is already using this port",
    detail: "Failed to bind UDP socket on 0.0.0.0:4433: Address already in use (os error 48)",
  },
  crash: {
    kind: "crashed",
    title: "Renderd stopped unexpectedly",
    detail: "The process ended without an error message.",
  },
};

export function createMock(query) {
  const platform = query.get("platform") ?? "macos";
  const role = query.get("role") ?? (platform === "macos" ? "host" : "viewer");
  const listeners = { status: [], log: [] };
  let timers = [];

  let settings = {
    role,
    host: { mode: "extend", smooth: false, port: 4433, display_id: 0 },
    viewer: { fullscreen: false, codec: "auto", recent_hosts: ["192.168.1.20:4433", "10.0.0.7:5000"] },
  };
  let status = emptyStatus(role);

  const emit = (name, payload) => listeners[name].forEach((handler) => handler({ payload }));
  const set = (patch) => {
    status = { ...status, ...patch };
    emit("status", status);
  };
  const log = (message, level = "info") => emit("log", { level, message, target: "mock" });
  const later = (delay, run) => timers.push(setTimeout(run, delay));
  const clear = () => {
    timers.forEach(clearTimeout);
    timers = [];
  };

  function jump(phase) {
    const host = status.role === "host";
    const address = host ? "192.168.1.20:4433" : query.get("host");
    const live = { address, peer: host ? "192.168.1.34:53561" : "192.168.1.20:4433", stream: STREAM };
    const metrics = { ...status.metrics, fps: 59.8, bitrate_kbps: 8240, rtt_ms: 1.4, decode_ms: 2.1 };
    const states = {
      stopped: emptyStatus(status.role),
      starting: { phase },
      waiting: { phase, address },
      connecting: { phase, address, peer: host ? live.peer : null },
      reconnecting: { phase, address: live.peer },
      streaming: { phase, ...live, address: host ? address : live.peer, metrics },
      stopping: { phase },
      failed: { ...emptyStatus(status.role), phase, problem: PROBLEMS[query.get("problem") ?? "port"] },
    };
    set(states[phase] ?? {});
    if (phase === "streaming") pulse();
  }

  // Keeps the numbers moving while "streaming".
  function pulse() {
    const wobble = (base, spread) => base + (Math.random() - 0.5) * spread;
    const tick = () => {
      if (status.phase !== "streaming") return;
      set({
        metrics: {
          ...status.metrics,
          fps: wobble(59.6, 1.2),
          bitrate_kbps: Math.max(600, wobble(7800, 4200)),
          rtt_ms: wobble(1.6, 0.8),
          decode_ms: wobble(2.2, 0.6),
        },
      });
      later(1000, tick);
    };
    later(1000, tick);
  }

  function run() {
    clear();
    log("Starting (mock session)");
    jump("starting");
    if (status.role === "host") {
      later(700, () => jump("waiting"));
      later(4200, () => jump("connecting"));
      later(5200, () => jump("streaming"));
    } else {
      later(700, () => jump("connecting"));
      later(2600, () => jump("streaming"));
    }
  }

  const commands = {
    app_info: () => ({ version: "1.0.0", platform, roles: platform === "macos" ? ["host", "viewer"] : ["viewer"] }),
    get_settings: () => settings,
    save_settings: ({ settings: next }) => {
      settings = next;
      if (status.role !== next.role) set(emptyStatus(next.role));
      return settings;
    },
    get_status: () => status,
    get_logs: () => [],
    start: ({ host }) => {
      if (host && !/^[\d.:[\]a-f]+$/i.test(host.trim())) {
        throw `"${host.trim()}" is not an address. It should look like 192.168.1.20 or 192.168.1.20:4433.`;
      }
      run();
      return status;
    },
    stop: () => {
      clear();
      jump("stopping");
      later(500, () => jump("stopped"));
    },
    screen_access: () => query.get("access") ?? (platform === "macos" ? "granted" : "unneeded"),
    request_screen_access: () => "denied",
    open_screen_settings: () => null,
  };

  const initial = query.get("phase");
  if (initial) setTimeout(() => jump(initial), 0);

  return {
    invoke: async (name, args = {}) => commands[name](args),
    listen: async (name, handler) => {
      listeners[name].push(handler);
      return () => {};
    },
  };
}
