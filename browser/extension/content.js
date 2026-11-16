// These functions run only in the main frame's ISOLATED extension world.
// They must be self-contained: executeScript serializes the function, not its imports.
export function prepareFill(origin, token) {
  delete globalThis.__termvaultFill;
  if (window.top !== window || location.origin !== origin || location.protocol !== "https:") return { ok: false };
  const visible = (el) => el instanceof HTMLInputElement && el.isConnected && !el.disabled && !el.readOnly
    && el.getClientRects().length > 0 && getComputedStyle(el).visibility === "visible"
    && getComputedStyle(el).display !== "none";
  const safeForm = (form) => {
    if (!form) return false;
    try {
      const action = new URL(form.action || location.href, location.href);
      return action.protocol === "https:" && action.origin === origin && !action.username && !action.password;
    } catch { return false; }
  };
  const passwords = [...document.querySelectorAll('input[type="password"]')].filter((el) =>
    visible(el) && el.autocomplete !== "new-password" && safeForm(el.form)
    && el.form.querySelectorAll('input[type="password"]').length === 1);
  const focused = passwords.filter((el) => el === document.activeElement || el.form === document.activeElement?.form);
  const password = focused.length === 1 ? focused[0] : passwords.length === 1 ? passwords[0] : null;
  if (!password) return { ok: false };
  const users = [...password.form.elements].filter((el) => visible(el) && ["text", "email", "tel"].includes(el.type));
  const preferred = users.filter((el) => ["username", "email"].includes(el.autocomplete));
  const username = preferred.length === 1 ? preferred[0] : users.length === 1 ? users[0] : null;
  // Ambiguous usernames are not guessed. A password-only step is supported.
  if (users.length > 0 && !username) return { ok: false };
  globalThis.__termvaultFill = { token, origin, password, username, form: password.form, created: Date.now() };
  return { ok: true };
}

export function fillCredentials(origin, token, username, password) {
  const state = globalThis.__termvaultFill;
  delete globalThis.__termvaultFill;
  if (!state || state.token !== token || state.origin !== origin || location.origin !== origin
      || location.protocol !== "https:" || window.top !== window || Date.now() - state.created > 180_000) return false;
  const visible = (el) => el instanceof HTMLInputElement && el.isConnected && !el.disabled && !el.readOnly
    && el.getClientRects().length > 0 && getComputedStyle(el).visibility === "visible"
    && getComputedStyle(el).display !== "none";
  const valid = () => {
    try {
      const action = new URL(state.form.action || location.href, location.href);
      return location.origin === origin && state.form.isConnected
        && action.protocol === "https:" && action.origin === origin && !action.username && !action.password
        && visible(state.password) && state.password.type === "password" && state.password.autocomplete !== "new-password"
        && state.password.form === state.form && state.form.querySelectorAll('input[type="password"]').length === 1
        && (!state.username || (visible(state.username) && ["text", "email", "tel"].includes(state.username.type)
          && state.username.form === state.form));
    } catch { return false; }
  };
  if (!valid()) return false;
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value").set;
  // Validate again after username events: a page may synchronously replace the form.
  if (state.username) {
    setter.call(state.username, username);
    state.username.dispatchEvent(new Event("input", { bubbles: true }));
    state.username.dispatchEvent(new Event("change", { bubbles: true }));
  }
  if (!valid()) return false;
  setter.call(state.password, password);
  state.password.dispatchEvent(new Event("input", { bubbles: true }));
  state.password.dispatchEvent(new Event("change", { bubbles: true }));
  // Never submit or click page buttons.
  return true;
}

export function clearFill(token) {
  if (globalThis.__termvaultFill?.token === token) delete globalThis.__termvaultFill;
}
