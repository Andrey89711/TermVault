import test from "node:test";
import assert from "node:assert/strict";
import { prepareFill, fillCredentials, clearFill } from "../extension/content.js";

const origin = "https://example.com";
class Input {
  constructor(type, form, autocomplete = "") {
    Object.assign(this, { type, form, autocomplete, isConnected: true, disabled: false, readOnly: false, visible: true, events: [] });
    this._value = "";
  }
  get value() { return this._value; }
  set value(value) { this._value = value; }
  getClientRects() { return this.visible ? [{}] : []; }
  dispatchEvent(event) { this.events.push(event.type); this.onEvent?.(event); }
}
function setup() {
  globalThis.HTMLInputElement = Input;
  globalThis.getComputedStyle = () => ({ visibility: "visible", display: "block" });
  globalThis.location = { origin, protocol: "https:", href: `${origin}/login` };
  globalThis.window = {};
  window.top = window;
  const form = { action: `${origin}/session`, isConnected: true, elements: [] };
  const username = new Input("email", form, "username");
  const password = new Input("password", form, "current-password");
  form.elements = [username, password];
  form.querySelectorAll = () => form.elements.filter((el) => el.type === "password");
  globalThis.document = { querySelectorAll: () => [password], activeElement: null };
  delete globalThis.__termvaultFill;
  return { form, username, password };
}

test("fills once, emits events, never submits", () => {
  const { username, password } = setup();
  assert.equal(prepareFill(origin, "token").ok, true);
  assert.equal(fillCredentials(origin, "token", "test-user", "test-password"), true);
  assert.equal(username.value, "test-user");
  assert.equal(password.value, "test-password");
  assert.deepEqual(password.events, ["input", "change"]);
  assert.equal(fillCredentials(origin, "token", "x", "y"), false);
});

test("denies HTTP, changed origins and iframes", () => {
  setup(); location.protocol = "http:";
  assert.equal(prepareFill(origin, "t").ok, false);
  setup(); location.origin = "https://evil.org";
  assert.equal(prepareFill(origin, "t").ok, false);
  setup(); window.top = {};
  assert.equal(prepareFill(origin, "t").ok, false);
});

test("denies hidden, disabled, read-only and new-password fields", () => {
  for (const [key, value] of [["visible", false], ["disabled", true], ["readOnly", true], ["autocomplete", "new-password"]]) {
    const { password } = setup();
    password[key] = value;
    assert.equal(prepareFill(origin, "t").ok, false, key);
  }
});

test("denies unsafe form actions before and after approval", () => {
  for (const action of ["https://evil.org", "http://example.com", "https://user@example.com"]) {
    const { form, password } = setup();
    assert.equal(prepareFill(origin, "t").ok, true);
    form.action = action;
    assert.equal(fillCredentials(origin, "t", "u", "p"), false);
    assert.equal(password.value, "");
    assert.equal(prepareFill(origin, "t").ok, false);
  }
});

test("rejects replaced fields, changed field types, expired approvals and wrong tokens", () => {
  for (const mutate of [
    ({ password }) => { password.isConnected = false; },
    ({ password }) => { password.type = "text"; },
    () => { globalThis.__termvaultFill.created -= 181_000; },
    () => { globalThis.__termvaultFill.token = "other"; },
    () => { location.origin = "https://evil.org"; },
  ]) {
    const state = setup();
    assert.equal(prepareFill(origin, "t").ok, true);
    mutate(state);
    assert.equal(fillCredentials(origin, "t", "u", "p"), false);
    assert.equal(state.password.value, "");
  }
});

test("rechecks after username input handlers mutate the form", () => {
  const { username, password, form } = setup();
  prepareFill(origin, "t");
  username.onEvent = () => { form.action = "https://evil.org"; };
  assert.equal(fillCredentials(origin, "t", "u", "p"), false);
  assert.equal(password.value, "");
});

test("supports password-only steps and cancellation", () => {
  const { form, password } = setup();
  form.elements = [password];
  assert.equal(prepareFill(origin, "t").ok, true);
  clearFill("other");
  assert.ok(globalThis.__termvaultFill);
  clearFill("t");
  assert.equal(fillCredentials(origin, "t", "u", "p"), false);
});

test("rejects multiple passwords and ambiguous forms; allows focused form", () => {
  const { form, password } = setup();
  form.elements.push(new Input("password", form));
  assert.equal(prepareFill(origin, "t").ok, false);
  form.elements.pop();
  const secondForm = { action: `${origin}/other`, isConnected: true, elements: [] };
  const second = new Input("password", secondForm);
  secondForm.elements = [second];
  secondForm.querySelectorAll = () => [second];
  document.querySelectorAll = () => [password, second];
  assert.equal(prepareFill(origin, "t").ok, false);
  document.activeElement = second;
  assert.equal(prepareFill(origin, "t").ok, true);
  assert.equal(globalThis.__termvaultFill.password, second);
});
