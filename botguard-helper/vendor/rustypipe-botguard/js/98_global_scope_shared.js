// Copyright 2018-2025 the Deno authors. MIT license.

import { core } from "ext:core/mod.js";

import * as event from "ext:deno_web/02_event.js";
import * as timers from "ext:deno_web/02_timers.js";
import * as base64 from "ext:deno_web/05_base64.js";
import * as encoding from "ext:deno_web/08_text_encoding.js";
import * as console from "ext:deno_console/01_console.js";
import * as compression from "ext:deno_web/14_compression.js";
import * as performance from "ext:deno_web/15_performance.js";
import * as url from "ext:deno_url/00_url.js";
import * as urlPattern from "ext:deno_url/01_urlpattern.js";
import * as streams from "ext:deno_web/06_streams.js";
import * as fileReader from "ext:deno_web/10_filereader.js";
import * as file from "ext:deno_web/09_file.js";
import * as messagePort from "ext:deno_web/13_message_port.js";
import * as webidl from "ext:deno_webidl/00_webidl.js";
import { DOMException } from "ext:deno_web/01_dom_exception.js";
import * as abortSignal from "ext:deno_web/03_abort_signal.js";
import * as imageData from "ext:deno_web/16_image_data.js";

// https://developer.mozilla.org/en-US/docs/Web/API/WindowOrWorkerGlobalScope
const windowOrWorkerGlobalScope = {
  AbortController: core.propNonEnumerable(abortSignal.AbortController),
  AbortSignal: core.propNonEnumerable(abortSignal.AbortSignal),
  Blob: core.propNonEnumerable(file.Blob),
  ByteLengthQueuingStrategy: core.propNonEnumerable(streams.ByteLengthQueuingStrategy),
  CloseEvent: core.propNonEnumerable(event.CloseEvent),
  CountQueuingStrategy: core.propNonEnumerable(streams.CountQueuingStrategy),
  CustomEvent: core.propNonEnumerable(event.CustomEvent),
  DecompressionStream: core.propNonEnumerable(compression.DecompressionStream),
  DOMException: core.propNonEnumerable(DOMException),
  ErrorEvent: core.propNonEnumerable(event.ErrorEvent),
  Event: core.propNonEnumerable(event.Event),
  EventTarget: core.propNonEnumerable(event.EventTarget),
  File: core.propNonEnumerable(file.File),
  FileReader: core.propNonEnumerable(fileReader.FileReader),
  ImageData: core.propNonEnumerable(imageData.ImageData),
  MessageEvent: core.propNonEnumerable(event.MessageEvent),
  Performance: core.propNonEnumerable(performance.Performance),
  PerformanceEntry: core.propNonEnumerable(performance.PerformanceEntry),
  PerformanceMark: core.propNonEnumerable(performance.PerformanceMark),
  PerformanceMeasure: core.propNonEnumerable(performance.PerformanceMeasure),
  PromiseRejectionEvent: core.propNonEnumerable(event.PromiseRejectionEvent),
  ProgressEvent: core.propNonEnumerable(event.ProgressEvent),
  ReadableStream: core.propNonEnumerable(streams.ReadableStream),
  ReadableStreamDefaultReader: core.propNonEnumerable(
    streams.ReadableStreamDefaultReader
  ),
  TextDecoder: core.propNonEnumerable(encoding.TextDecoder),
  TextEncoder: core.propNonEnumerable(encoding.TextEncoder),
  TextDecoderStream: core.propNonEnumerable(encoding.TextDecoderStream),
  TextEncoderStream: core.propNonEnumerable(encoding.TextEncoderStream),
  TransformStream: core.propNonEnumerable(streams.TransformStream),
  URL: core.propNonEnumerable(url.URL),
  URLPattern: core.propNonEnumerable(urlPattern.URLPattern),
  URLSearchParams: core.propNonEnumerable(url.URLSearchParams),
  MessageChannel: core.propNonEnumerable(messagePort.MessageChannel),
  MessagePort: core.propNonEnumerable(messagePort.MessagePort),
  WritableStream: core.propNonEnumerable(streams.WritableStream),
  WritableStreamDefaultWriter: core.propNonEnumerable(
    streams.WritableStreamDefaultWriter
  ),
  WritableStreamDefaultController: core.propNonEnumerable(
    streams.WritableStreamDefaultController
  ),
  ReadableByteStreamController: core.propNonEnumerable(
    streams.ReadableByteStreamController
  ),
  ReadableStreamBYOBReader: core.propNonEnumerable(streams.ReadableStreamBYOBReader),
  ReadableStreamBYOBRequest: core.propNonEnumerable(streams.ReadableStreamBYOBRequest),
  ReadableStreamDefaultController: core.propNonEnumerable(
    streams.ReadableStreamDefaultController
  ),
  TransformStreamDefaultController: core.propNonEnumerable(
    streams.TransformStreamDefaultController
  ),
  atob: core.propWritable(base64.atob),
  btoa: core.propWritable(base64.btoa),
  clearInterval: core.propWritable(timers.clearInterval),
  clearTimeout: core.propWritable(timers.clearTimeout),
  console: core.propNonEnumerable(
    new console.Console((msg) => core.print(msg, true))
  ),
  performance: core.propWritable(performance.performance),
  reportError: core.propWritable(event.reportError),
  setInterval: core.propWritable(timers.setInterval),
  setTimeout: core.propWritable(timers.setTimeout),
  structuredClone: core.propWritable(messagePort.structuredClone),
  setImmediate: core.propWritable(timers.setImmediate),
  clearImmediate: core.propWritable(timers.clearTimeout),
  process: {},
  global: {
    enumerable: true,
    configurable: true,
    get: () => globalThis,
  },
  // Branding as a WebIDL object
  [webidl.brand]: core.propNonEnumerable(webidl.brand),
};

export { windowOrWorkerGlobalScope };
