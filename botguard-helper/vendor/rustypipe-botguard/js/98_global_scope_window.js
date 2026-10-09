// Copyright 2018-2025 the Deno authors. MIT license.

import { core } from "ext:core/mod.js";

import * as location from "ext:deno_web/12_location.js";
import * as webidl from "ext:deno_webidl/00_webidl.js";
import * as globalInterfaces from "ext:deno_web/04_global_interfaces.js";

class Navigator {
  constructor() {
    webidl.illegalConstructor();
  }
}

const navigator = webidl.createBranded(Navigator);

const mainRuntimeGlobalProperties = {
  Location: location.locationConstructorDescriptor,
  location: {
    writable: true,
    configurable: true,
  },
  Window: globalInterfaces.windowConstructorDescriptor,
  self: core.propGetterOnly(() => globalThis),
  Navigator: core.propNonEnumerable(Navigator),
  navigator: core.propGetterOnly(() => navigator),
};

export { mainRuntimeGlobalProperties };
