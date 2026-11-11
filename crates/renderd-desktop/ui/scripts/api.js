// The bridge to the Rust side. In a browser, where there is no Rust side, a
// small stand-in takes its place so the window can be designed and previewed.

import { createMock } from "./mock.js";

const tauri = window.__TAURI__;
const backend = tauri
  ? { invoke: tauri.core.invoke, listen: tauri.event.listen }
  : createMock(new URLSearchParams(window.location.search));

/** Whether the real app is behind this page. */
export const native = Boolean(tauri);

export const appInfo = () => backend.invoke("app_info");
export const getSettings = () => backend.invoke("get_settings");
export const saveSettings = (settings) => backend.invoke("save_settings", { settings });
export const getStatus = () => backend.invoke("get_status");
export const getLogs = () => backend.invoke("get_logs");
export const start = (host) => backend.invoke("start", { host: host ?? null });
export const stop = () => backend.invoke("stop");
export const screenAccess = () => backend.invoke("screen_access");
export const requestScreenAccess = () => backend.invoke("request_screen_access");
export const openScreenSettings = () => backend.invoke("open_screen_settings");

/** Calls `handler` with every new status. */
export const onStatus = (handler) => backend.listen("status", (event) => handler(event.payload));

/** Calls `handler` with every new log line. */
export const onLog = (handler) => backend.listen("log", (event) => handler(event.payload));
