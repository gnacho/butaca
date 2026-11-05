// Browser HTTP transport for the control plane. Emscripten's JSPI integration suspends the
// calling Wasm thread while browser fetch() resolves, so worker requests do not block SDL's UI loop.
#include <emscripten/emscripten.h>
#include <stddef.h>
#include <stdint.h>
#include <stdlib.h>

EM_ASYNC_JS(int, browser_fetch_async,
            (const char *url, const char *method, const char *const *headers,
             const unsigned char *body, size_t body_len, uint32_t timeout_ms,
             int *status_out, unsigned char **body_out, size_t *body_len_out), {
  HEAP32[status_out >> 2] = 0;
  HEAPU32[body_out >> 2] = 0;
  HEAPU32[body_len_out >> 2] = 0;

  let timeout = null;
  try {
    const requestHeaders = new Headers();
    const pairs = headers >> 2;
    for (let i = 0; ; i += 2) {
      const namePtr = HEAPU32[pairs + i];
      if (!namePtr) break;
      const valuePtr = HEAPU32[pairs + i + 1];
      if (!valuePtr) return -4;
      requestHeaders.append(UTF8ToString(namePtr), UTF8ToString(valuePtr));
    }

    const controller = new AbortController();
    if (timeout_ms) timeout = setTimeout(() => controller.abort(), timeout_ms);
    const options = {
      method: UTF8ToString(method),
      headers: requestHeaders,
      mode: 'cors',
      credentials: 'omit',
      redirect: 'manual',
      cache: 'no-store',
      signal: controller.signal
    };
    if (body_len) options.body = HEAPU8.slice(body, body + body_len);

    const response = await fetch(UTF8ToString(url), options);
    if (response.type === 'opaqueredirect') return -2;
    if (!response.status) return 0;

    const data = new Uint8Array(await response.arrayBuffer());
    const dataPtr = _malloc(data.byteLength || 1);
    if (!dataPtr) return -3;
    // Publish ownership before copying: if a memory/view error throws, Rust still frees this
    // allocation after the failure return rather than leaking it on a worker thread.
    HEAP32[status_out >> 2] = response.status;
    HEAPU32[body_out >> 2] = dataPtr;
    HEAPU32[body_len_out >> 2] = data.byteLength;
    HEAPU8.set(data, dataPtr);
    return 1;
  } catch (error) {
    return error && error.name === 'AbortError' ? -1 : 0;
  } finally {
    if (timeout !== null) clearTimeout(timeout);
  }
});

int butaca_emscripten_http(const char *url, const char *method,
                           const char *const *headers, const unsigned char *body,
                           size_t body_len, uint32_t timeout_ms, int *status_out,
                           unsigned char **body_out, size_t *body_len_out) {
  return browser_fetch_async(url, method, headers, body, body_len, timeout_ms,
                             status_out, body_out, body_len_out);
}

void butaca_emscripten_http_free(void *body) {
  free(body);
}
