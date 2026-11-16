import test from "node:test";
import assert from "node:assert/strict";
import { checkConnection } from "../extension/options.js";

test("ping sends no URL or credentials and reports the compatible host version", async () => {
  const result = await checkConnection({ sendNativeMessage: async (host, request) => {
    assert.equal(host, "com.termvault.browser");
    assert.deepEqual(request, { action: "ping" });
    return { ok: true, kind: "pong", protocol: 1, version: "0.1.0" };
  } });
  assert.ok(result.includes("0.1.0"));
});

test("ping ignores incompatible responses and never renders arbitrary native errors", async () => {
  for (const reply of [{ ok: false, error: "PRIVATE_SECRET" }, { ok: true, kind: "pong", protocol: 2, version: "0.1.0" }]) {
    const result = await checkConnection({ sendNativeMessage: async () => reply });
    assert.ok(!result.includes("PRIVATE_SECRET"));
    assert.ok(result.includes("протокол"));
  }
  const result = await checkConnection({ sendNativeMessage: async () => { throw new Error("PRIVATE_SECRET"); } });
  assert.ok(!result.includes("PRIVATE_SECRET"));
});
