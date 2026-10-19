//! User-facing language: Italian when the session locale is Italian, English otherwise.
//!
//! Strings are written next to each other at the call site with [`tr!`](crate::tr), so a
//! message and its translation are reviewed together. The privileged helper runs under
//! pkexec with a cleaned environment, so the caller passes its language explicitly
//! (`droidnative priv --lang it …`) and the helper calls [`set_override`].

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Lang {
    It,
    En,
}

impl Lang {
    pub fn code(self) -> &'static str {
        match self {
            Self::It => "it",
            Self::En => "en",
        }
    }

    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            "it" => Some(Self::It),
            "en" => Some(Self::En),
            _ => None,
        }
    }
}

static LANG: OnceLock<Lang> = OnceLock::new();

/// Language of a POSIX locale value (`it_IT.UTF-8`, `it`, `en_US`, `C`, …).
pub fn lang_of_locale(value: &str) -> Option<Lang> {
    let v = value.trim();
    if v.is_empty() {
        return None;
    }
    Some(if v.starts_with("it") { Lang::It } else { Lang::En })
}

/// gettext precedence: LANGUAGE (first entry), LC_ALL, LC_MESSAGES, LANG.
pub fn detect_from(get: impl Fn(&str) -> Option<String>) -> Lang {
    if let Some(list) = get("LANGUAGE").filter(|s| !s.is_empty()) {
        if let Some(first) = list.split(':').find(|s| !s.is_empty()) {
            if let Some(l) = lang_of_locale(first) {
                return l;
            }
        }
    }
    for key in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Some(l) = get(key).as_deref().and_then(lang_of_locale) {
            return l;
        }
    }
    Lang::En
}

#[cfg(not(test))]
fn detect() -> Lang {
    detect_from(|k| std::env::var(k).ok())
}

/// Unit tests assert on the Italian texts; English is covered by `tr_in` tests.
#[cfg(test)]
fn detect() -> Lang {
    Lang::It
}

pub fn lang() -> Lang {
    *LANG.get_or_init(detect)
}

/// Force the language (the privileged helper does this from `--lang`). Has no effect
/// once a message has already been produced.
pub fn set_override(lang: Lang) {
    let _ = LANG.set(lang);
}

pub fn pick<'a>(it: &'a str, en: &'a str) -> &'a str {
    match lang() {
        Lang::It => it,
        Lang::En => en,
    }
}

/// `tr!("testo {x}", "text {x}")` → `String` in the current language; formatting works
/// like `format!`, including inline captured identifiers.
#[macro_export]
macro_rules! tr {
    ($it:literal, $en:literal $(,)?) => {
        match $crate::i18n::lang() {
            $crate::i18n::Lang::It => format!($it),
            $crate::i18n::Lang::En => format!($en),
        }
    };
    ($it:literal, $en:literal, $($arg:tt)+) => {
        match $crate::i18n::lang() {
            $crate::i18n::Lang::It => format!($it, $($arg)+),
            $crate::i18n::Lang::En => format!($en, $($arg)+),
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |k| pairs.iter().find(|(key, _)| *key == k).map(|(_, v)| v.to_string())
    }

    #[test]
    fn locale_detection_follows_gettext_order() {
        assert_eq!(detect_from(env(&[("LANG", "it_IT.UTF-8")])), Lang::It);
        assert_eq!(detect_from(env(&[("LANG", "de_DE.UTF-8")])), Lang::En);
        assert_eq!(detect_from(env(&[("LANG", "it_IT.UTF-8"), ("LC_ALL", "C")])), Lang::En);
        assert_eq!(detect_from(env(&[("LANGUAGE", "it:en"), ("LANG", "en_US.UTF-8")])), Lang::It);
        assert_eq!(detect_from(env(&[("LC_MESSAGES", "it_CH.UTF-8"), ("LANG", "fr_FR.UTF-8")])), Lang::It);
        assert_eq!(detect_from(env(&[])), Lang::En);
    }

    #[test]
    fn tr_formats_with_captures() {
        let n = 3;
        assert_eq!(tr!("{n} immagini", "{n} images"), "3 immagini");
        assert_eq!(tr!("{} volte", "{} times", n), "3 volte");
    }
}
