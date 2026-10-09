import { primordials } from "ext:core/mod.js";
const { ObjectDefineProperties, ObjectSetPrototypeOf } = primordials;

import { windowOrWorkerGlobalScope } from "ext:runtime/98_global_scope_shared.js";
import { mainRuntimeGlobalProperties } from "ext:runtime/98_global_scope_window.js";
import * as event from "ext:deno_web/02_event.js";

// Set up global properties shared by main and worker runtime.
ObjectDefineProperties(globalThis, windowOrWorkerGlobalScope);
ObjectDefineProperties(globalThis, mainRuntimeGlobalProperties);

ObjectSetPrototypeOf(globalThis, Window.prototype);

event.defineEventHandler(globalThis, "error");
event.defineEventHandler(globalThis, "load");
event.defineEventHandler(globalThis, "beforeunload");
event.defineEventHandler(globalThis, "unload");

event.setEventTargetData(globalThis);
event.saveGlobalThisReference(globalThis);
event.defineEventHandler(globalThis, "unhandledrejection");

globalThis.__crabytubeInstallYtcfg = (rawConfig) => {
  const config = JSON.parse(rawConfig);
  globalThis.yt = { config_: config };
  if (globalThis.window) globalThis.window.yt = globalThis.yt;
};
