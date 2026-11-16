import test from "node:test";
import assert from "node:assert/strict";

let onClick;
let scripts = [];
let messages = [];
let badges = [];
let failFill = false;
let allowedOrigin = "https://example.com";
let response;
let failure;
let titles = [];
globalThis.chrome = {
  action: {
    onClicked: { addListener: (handler) => { onClick = handler; } },
    setBadgeText: async (value) => { badges.push(value.text); },
    setTitle: async ({ title }) => { titles.push(title); },
  },
  scripting: { executeScript: async (request) => {
    scripts.push(request);
    if (failure?.stage === "PAGE" && request.func.name === "prepareFill") throw new Error(failure.message);
    if (request.func.name === "prepareFill") return [{ documentId: "original-document", result: { ok: true } }];
    if (request.func.name === "fillCredentials") return [{ result: !failFill }];
    return [{}];
  } },
  runtime: { sendNativeMessage: async (host, message) => {
    messages.push({ host, message });
    if (failure?.stage === "NATIVE") throw new Error(failure.message);
    response = { ok: true, credentials: { origin: allowedOrigin, username: "dummy-user", password: "dummy-password" } };
    return response;
  } },
};
await import("../extension/background.js");
function reset() { scripts = []; messages = []; badges = []; titles = []; failure = null; failFill = false; allowedOrigin = "https://example.com"; }

test("sends only origin, targets original document, clears response", async () => {
  reset();
  await onClick({ id: 1, url: "https://example.com/login?private=query" });
  assert.deepEqual(messages, [{ host: "com.termvault.browser", message: { action: "fill", url: "https://example.com" } }]);
  assert.deepEqual(scripts[1].target, { tabId: 1, documentIds: ["original-document"] });
  assert.equal(scripts[1].args[3], "dummy-password");
  assert.equal(response.credentials.password, "");
  assert.equal(scripts.at(-1).func.name, "clearFill");
  assert.equal(badges.at(-1), "✓");
});

test("HTTP and embedded credentials never reach native host", async () => {
  for (const url of ["http://example.com", "https://user@example.com", "chrome://settings"]) {
    reset();
    await onClick({ id: 1, url });
    assert.equal(messages.length, 0);
    assert.equal(scripts.length, 0);
    assert.equal(badges.at(-1), "!");
  }
});

test("denies mismatched native response and allows retry", async () => {
  reset();
  allowedOrigin = "https://evil.org";
  await onClick({ id: 1, url: "https://example.com" });
  assert.equal(scripts.some((s) => s.func.name === "fillCredentials"), false);
  assert.equal(badges.at(-1), "!");
  assert.equal(response.credentials.password, "");
  reset();
  await onClick({ id: 1, url: "https://example.com" });
  assert.equal(badges.at(-1), "✓");
});

test("does not claim success after navigation or form change", async () => {
  reset();
  failFill = true;
  await onClick({ id: 1, url: "https://example.com" });
  assert.equal(badges.at(-1), "!");
});

test("diagnostics distinguish page access from missing/forbidden native host without leaking errors", async () => {
  for (const [stage, message, code] of [
    ["PAGE", "Cannot access contents of url: PRIVATE_URL", "PAGE_ACCESS"],
    ["NATIVE", "Specified native messaging host not found.", "HOST_NOT_FOUND"],
    ["NATIVE", "Access to the specified native messaging host is forbidden.", "HOST_FORBIDDEN"],
    ["NATIVE", "Native host has exited.", "HOST_CONNECTION"],
    ["NATIVE", "PRIVATE_PASSWORD", "NATIVE_API"],
  ]) {
    reset();
    failure = { stage, message };
    await onClick({ id: 1, url: "https://example.com" });
    assert.ok(titles.at(-1).includes(`[${code}]`));
    assert.ok(!titles.at(-1).includes("PRIVATE_"));
    assert.equal(badges.at(-1), "!");
  }
});
