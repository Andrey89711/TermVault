export async function checkConnection(runtime) {
  try {
    const response = await runtime.sendNativeMessage("com.termvault.browser", { action: "ping" });
    if (response?.ok === true && response.kind === "pong" && response.protocol === 1
        && typeof response.version === "string" && /^\d+\.\d+\.\d+$/.test(response.version)) {
      return `Подключено. Локальный TermVault ${response.version}. Хранилище не открывалось.`;
    }
    return "Мост не подтвердил совместимый протокол. Обновите программу через setup.ps1 и повторите проверку.";
  } catch (error) {
    // Only fixed diagnostics, never render a native payload or an arbitrary API exception.
    const message = error instanceof Error ? error.message : "";
    if (/native messaging host.*not found|native.*host.*not.*registered/i.test(message)) {
      return "Мост не найден. Запустите setup.ps1, выберите этот браузер и укажите ID выше.";
    }
    if (/forbidden|not allowed|permission|access.*denied/i.test(message)) {
      return "Браузер запретил подключение. Проверьте ID в setup.ps1 и корпоративные политики.";
    }
    return "Не удалось подключиться. Проверьте установку и повторите попытку. Мастер-пароль для проверки не нужен.";
  }
}

if (typeof document !== "undefined") {
  document.getElementById("extension-id").textContent = chrome.runtime.id;
  const button = document.getElementById("check");
  const status = document.getElementById("status");
  button.addEventListener("click", async () => {
    button.disabled = true;
    status.textContent = "Проверка…";
    try { status.textContent = await checkConnection(chrome.runtime); }
    finally { button.disabled = false; }
  });
}
