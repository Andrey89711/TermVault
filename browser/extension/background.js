import { prepareFill, fillCredentials, clearFill } from "./content.js";

const pending = new Set();

async function status(tabId, text, title) {
  await Promise.all([
    chrome.action.setBadgeText({ tabId, text }),
    chrome.action.setTitle({ tabId, title }),
  ]);
}

chrome.action.onClicked.addListener(async (tab) => {
  if (!Number.isInteger(tab.id) || pending.has(tab.id)) return;
  const tabId = tab.id;
  pending.add(tabId);
  let reply;
  let documentId;
  let stage = "PAGE";
  const token = crypto.randomUUID();
  try {
    const url = new URL(tab.url);
    if (url.protocol !== "https:" || url.username || url.password) {
      throw new Error("Разрешены только HTTPS-страницы без логина в адресе.");
    }
    await status(tabId, "…", "TermVault: подтвердите запрос в локальном окне (до 3 минут)");
    const [prepared] = await chrome.scripting.executeScript({
      target: { tabId, frameIds: [0] },
      func: prepareFill,
      args: [url.origin, token],
    });
    if (!prepared?.result?.ok || !prepared.documentId) {
      throw new Error("Не найдена однозначная форма входа. Выберите поле пароля и попробуйте снова.");
    }
    documentId = prepared.documentId;
    stage = "NATIVE";
    reply = await chrome.runtime.sendNativeMessage("com.termvault.browser", {
      action: "fill", url: url.origin,
    });
    if (!reply?.ok || reply.credentials?.origin !== url.origin
        || typeof reply.credentials.username !== "string" || typeof reply.credentials.password !== "string"
        || reply.credentials.username.length > 1024 || reply.credentials.password.length > 1024) {
      throw new Error("Запрос отменён или мост недоступен. Проверьте установку TermVault.");
    }
    // A document ID prevents a same-origin navigation from receiving an old approval.
    stage = "FILL";
    const [filled] = await chrome.scripting.executeScript({
      target: { tabId, documentIds: [documentId] },
      func: fillCredentials,
      args: [url.origin, token, reply.credentials.username, reply.credentials.password],
    });
    if (!filled?.result) throw new Error("Страница или форма изменилась. Запросите заполнение ещё раз.");
    await status(tabId, "✓", "TermVault: заполнено, отправьте форму самостоятельно");
  } catch (error) {
    // Never log the exception/native response; messages from APIs can contain payloads.
    let safe = error instanceof Error && [
      "Разрешены только HTTPS-страницы без логина в адресе.",
      "Не найдена однозначная форма входа. Выберите поле пароля и попробуйте снова.",
      "Запрос отменён или мост недоступен. Проверьте установку TermVault.",
      "Страница или форма изменилась. Запросите заполнение ещё раз.",
    ].includes(error.message) ? error.message : null;
    // Classify known API failures; never display/log arbitrary exception text or payloads.
    if (!safe) {
      const message = error instanceof Error ? error.message : "";
      if (stage === "NATIVE") {
        if (/native messaging host.*not found|native.*host.*not.*registered/i.test(message)) {
          safe = "[HOST_NOT_FOUND] Браузер не нашёл регистрацию локального моста.";
        } else if (/forbidden|not allowed|permission|access.*denied/i.test(message)) {
          safe = "[HOST_FORBIDDEN] Браузер запретил мост: проверьте ID расширения и политики.";
        } else if (/exited|communicat|disconnected|failed to start/i.test(message)) {
          safe = "[HOST_CONNECTION] Локальный мост не запустился или соединение закрылось.";
        } else {
          safe = "[NATIVE_API] Ошибка подключения Native Messaging. Уточните браузер и его версию.";
        }
      } else if (stage === "FILL") {
        safe = "[FILL_DOCUMENT] Исходная страница недоступна или изменилась. Повторите запрос.";
      } else {
        safe = "[PAGE_ACCESS] Не удалось проверить страницу. Откройте обычную HTTPS-вкладку, обновите её и разрешите расширению доступ к сайту.";
      }
    }
    await status(tabId, "!", `TermVault: ${safe}`).catch(() => {});
  } finally {
    if (reply?.credentials) {
      reply.credentials.username = "";
      reply.credentials.password = "";
    }
    if (documentId) {
      await chrome.scripting.executeScript({
        target: { tabId, documentIds: [documentId] }, func: clearFill, args: [token],
      }).catch(() => {});
    }
    pending.delete(tabId);
  }
});
