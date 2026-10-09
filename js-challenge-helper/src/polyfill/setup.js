/*
 * Copied unmodified from ytdlp-ejs (https://github.com/ahaoboy/ytdlp-ejs),
 * commit de624cbd4d8e7cf29f0dcb9b00b410991570d011, src/builtin/polyfill/.
 *
 * MIT License
 *
 * Copyright (c) 2026 ahaoboy
 *
 * Permission is hereby granted, free of charge, to any person obtaining a copy
 * of this software and associated documentation files (the "Software"), to deal
 * in the Software without restriction, including without limitation the rights
 * to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
 * copies of the Software, and to permit persons to whom the Software is
 * furnished to do so, subject to the following conditions:
 *
 * The above copyright notice and this permission notice shall be included in all
 * copies or substantial portions of the Software.
 *
 * THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
 * IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
 * FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
 * AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
 * LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
 * OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
 * SOFTWARE.
 */

// Browser environment simulation for YouTube player execution
if (typeof globalThis.XMLHttpRequest === "undefined") {
  globalThis.XMLHttpRequest = { prototype: {} };
}
if (typeof URL === "undefined") {
  globalThis.location = {
    hash: "",
    host: "www.youtube.com",
    hostname: "www.youtube.com",
    href: "https://www.youtube.com/watch?v=yt-dlp-wins",
    origin: "https://www.youtube.com",
    password: "",
    pathname: "/watch",
    port: "",
    protocol: "https:",
    search: "?v=yt-dlp-wins",
    username: "",
  };
} else {
  globalThis.location = new URL("https://www.youtube.com/watch?v=yt-dlp-wins");
}
if (typeof globalThis.document === "undefined") {
  globalThis.document = Object.create(null);
}
if (typeof globalThis.navigator === "undefined") {
  globalThis.navigator = Object.create(null);
}
if (typeof globalThis.self === "undefined") {
  globalThis.self = globalThis;
}
if (typeof globalThis.window === "undefined") {
  globalThis.window = globalThis;
}
// TV player uses `g` as a global without declaring it (no `var g`).
// Standard players define `g` as an IIFE parameter, but TV players don't.
if (typeof globalThis.g === "undefined") {
  globalThis.g = {};
}
