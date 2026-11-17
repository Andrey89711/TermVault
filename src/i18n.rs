//! UI-only locale selection. Never apply translations to vault contents.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub enum Language {
    #[default]
    #[serde(rename = "en")]
    English,
    #[serde(rename = "ru")]
    Russian,
}

impl Language {
    pub fn text(self, en: &'static str, ru: &'static str) -> &'static str {
        match self {
            Self::English => en,
            Self::Russian => ru,
        }
    }
}

/// Both format strings are static UI templates; arguments are never translated.
macro_rules! localized {
    ($language:expr, $en:literal, $ru:literal $(, $argument:expr)* $(,)?) => {
        match $language {
            crate::i18n::Language::English => format!($en $(, $argument)*),
            crate::i18n::Language::Russian => format!($ru $(, $argument)*),
        }
    };
}
pub(crate) use localized;

/// Translate only known, context-only errors from our own subsystems. Unknown
/// OS/library errors remain intact; this is not a general string translator.
pub fn error(language: Language, message: &str) -> String {
    if language == Language::English {
        return message.into();
    }
    let translated = match message {
        "Master passwords do not match" => "Мастер-пароли не совпадают",
        "New master passphrases do not match" => "Новые мастер-пароли не совпадают",
        "Title is required (maximum 256 bytes)" => "Укажите название (не более 256 байт)",
        "A field exceeds its size limit or contains unsupported control characters" => "Поле превышает допустимый размер или содержит недопустимые управляющие символы",
        "Use a master passphrase of at least 12 characters (maximum 1024 bytes)" => "Мастер-пароль должен содержать не менее 12 символов (не более 1024 байт)",
        "Choose a .kdbx file" | "Choose a .kdbx filename" => "Выберите файл с расширением .kdbx",
        "Vault not found; use --create to explicitly create it" => "Хранилище не найдено; для создания используйте --create",
        "Vault must be a regular file, not a symlink/reparse point" => "Хранилище должно быть обычным файлом, а не ссылкой",
        "Vault exceeds the 8 MiB MVP limit" | "Encrypted vault exceeds the 8 MiB MVP limit" => "Хранилище превышает ограничение MVP: 8 МиБ",
        "Destination already exists; refusing to overwrite it" => "Файл уже существует; перезапись запрещена",
        "Vault has no parent directory" | "Missing vault directory" | "Missing vault parent" => "Не найдена папка хранилища",
        "Missing vault filename" => "Не найдено имя файла хранилища",
        "Cannot create the vault directory" => "Не удалось создать папку хранилища",
        "Cannot create vault; destination may already exist" => "Не удалось создать хранилище; возможно, файл уже существует",
        "Master password exceeds the input limit" => "Мастер-пароль превышает допустимую длину",
        "Cannot unlock: wrong master password or damaged/unsupported vault" => "Не удалось разблокировать: неверный мастер-пароль или повреждённое/неподдерживаемое хранилище",
        "Vault exceeds MVP entry/group limits" => "Превышено ограничение MVP на число записей или папок",
        "Foreign vault is read-only" | "Foreign vault is read-only in this MVP" => "Стороннее хранилище доступно только для чтения",
        "Folder name must contain 1–128 bytes without slashes or control characters" => "Имя папки: 1–128 байт, без слешей и управляющих символов",
        "Vault limit: 100 folders including root" => "Ограничение: 100 папок, включая корневую",
        "Parent folder no longer exists" => "Родительская папка больше не существует",
        "A folder with this name already exists here" => "Папка с таким именем здесь уже существует",
        "Folder no longer exists" => "Папка больше не существует",
        "The root folder cannot be deleted" => "Корневую папку нельзя удалить",
        "Destination folder no longer exists" => "Папка назначения больше не существует",
        "Entry no longer exists" => "Запись больше не существует",
        "MVP limit: 2000 entries" => "Ограничение MVP: 2000 записей",
        "Invalid current master passphrase" => "Недопустимый текущий мастер-пароль",
        "Vault changed on disk; lock and reopen it before retrying" => "Хранилище изменено на диске; заблокируйте и откройте его заново",
        "Current master passphrase is incorrect" => "Неверный текущий мастер-пароль",
        "The new master passphrase must be different" => "Новый мастер-пароль должен отличаться от текущего",
        "Vault filename cannot be confirmed in this interface" => "В этом интерфейсе невозможно подтвердить имя файла",
        "The typed filename does not match the vault filename" => "Введённое имя не совпадает с именем файла хранилища",
        "Delete confirmation expired or no longer matches; authenticate again" => "Подтверждение удаления истекло или недействительно; введите пароль заново",
        "Vault changed on disk; deletion cancelled" => "Хранилище изменено на диске; удаление отменено",
        "System randomness unavailable" => "Системный генератор случайных чисел недоступен",
        "Cannot check encrypted backup destination; deletion cancelled" => "Не удалось проверить путь бэкапа; удаление отменено",
        "Cannot move vault to encrypted backup; deletion cancelled" => "Не удалось переместить хранилище в зашифрованный бэкап; удаление отменено",
        "Cannot allocate an encrypted backup name; deletion cancelled" => "Не удалось выбрать имя зашифрованного бэкапа; удаление отменено",
        "Vault changed on disk; lock and reopen it. Your draft is still in memory" => "Хранилище изменено на диске; заблокируйте и откройте его заново. Черновик пока в памяти",
        "Encrypted save verification failed" => "Не удалось проверить зашифрованный файл",
        "Vault changed on disk; no changes were written" => "Хранилище изменено на диске; изменения не записаны",
        "Cannot create encrypted backup; save cancelled" => "Не удалось создать зашифрованный бэкап; сохранение отменено",
        "Cannot atomically replace vault; draft retained" => "Не удалось безопасно заменить файл хранилища; черновик сохранён в памяти",
        "KDBX encryption failed; no changes written" => "Ошибка шифрования KDBX; изменения не записаны",
        "Cannot create encrypted temporary file" => "Не удалось создать зашифрованный временный файл",
        "Cannot write encrypted file" => "Не удалось записать зашифрованный файл",
        "Cannot flush encrypted file" => "Не удалось завершить запись зашифрованного файла",
        "Truncated KDBX header" => "Заголовок KDBX обрезан",
        "Invalid KDBX integer" => "Недопустимое число в KDBX",
        "Only KDBX4 vaults are supported" => "Поддерживаются только хранилища KDBX4",
        "KDBX header too large" => "Заголовок KDBX слишком большой",
        "KDBX header field too large" => "Поле заголовка KDBX слишком большое",
        "Duplicate KDBX header field" => "Повторяющееся поле заголовка KDBX",
        "Public custom header data is unsupported in this MVP" => "Пользовательские открытые данные заголовка не поддерживаются в MVP",
        "MVP supports uncompressed vaults only; do not import your main password database" => "MVP поддерживает только несжатые хранилища; не импортируйте основную базу паролей",
        "MVP supports AES-256 KDBX4 vaults only" => "MVP поддерживает только KDBX4 с AES-256",
        "Missing KDF parameters" | "Missing KDF parameter" => "Отсутствуют параметры KDF",
        "Unsupported KDF dictionary version" => "Неподдерживаемая версия словаря KDF",
        "Invalid KDF dictionary terminator" | "Invalid KDF dictionary" => "Некорректный словарь KDF",
        "KDF key too long" => "Слишком длинный ключ KDF",
        "Only Argon2id vaults are supported in the MVP" => "MVP поддерживает только хранилища с Argon2id",
        "KDF parameters exceed the MVP resource policy" => "Параметры KDF превышают ограничения ресурсов MVP",
        "Paste rejected: unsupported characters or input too large" => "Вставка отклонена: недопустимые символы или слишком длинное поле",
        "Paste rejected: maximum 128 bytes, no control characters." => "Вставка отклонена: не более 128 байт, без управляющих символов.",
        "GitHub response exceeds the size limit" => "Ответ GitHub превышает допустимый размер",
        "Invalid GitHub response" => "Некорректный ответ GitHub",
        "Expected a published stable release" => "Ожидался опубликованный стабильный релиз",
        "Invalid release tag" => "Некорректный тег релиза",
        "Invalid release version" => "Некорректная версия релиза",
        "Expected a stable vX.Y.Z tag" => "Ожидался стабильный тег vX.Y.Z",
        "Invalid application version" => "Некорректная версия приложения",
        "Cannot prepare HTTPS update check" => "Не удалось подготовить HTTPS-проверку",
        "Cannot reach GitHub. Check your network and retry." => "Не удалось связаться с GitHub. Проверьте сеть и повторите попытку.",
        "GitHub rejected the check. Try again later." => "GitHub отклонил проверку. Повторите позже.",
        "Cannot read GitHub response" => "Не удалось прочитать ответ GitHub",
        "Cannot start update check" => "Не удалось запустить проверку обновлений",
        "Update check was interrupted. Please retry." => "Проверка обновлений была прервана. Повторите попытку.",
        "Cannot access clipboard" => "Не удалось открыть буфер обмена",
        "Cannot copy to clipboard" => "Не удалось скопировать в буфер обмена",
        "System clipboard unavailable" => "Системный буфер обмена недоступен",
        "Cannot write to system clipboard" => "Не удалось записать в системный буфер обмена",
        "Cannot read system clipboard" => "Не удалось прочитать системный буфер обмена",
        "Cannot clear system clipboard" => "Не удалось очистить системный буфер обмена",
        "Password length must be 16–128" => "Длина пароля должна быть от 16 до 128 символов",
        "Operating-system randomness unavailable" => "Системный генератор случайных чисел недоступен",
        "Password generation failed; try again" => "Не удалось сгенерировать пароль; повторите попытку",
        "Cannot save language preference; existing settings were not changed." => "Не удалось сохранить язык; существующие настройки не изменены.",
        _ => message,
    };
    translated.into()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn default_is_english_and_unknown_data_is_never_translated() {
        assert_eq!(Language::default(), Language::English);
        assert_eq!(
            error(Language::Russian, "My notes: Title is required"),
            "My notes: Title is required"
        );
        assert_eq!(
            error(Language::Russian, "Title is required (maximum 256 bytes)"),
            "Укажите название (не более 256 байт)"
        );
    }
}
