//! The words the app shows, in the language chosen: gettext's behaviour
//! over the tables `tools/make_translations.py` writes from the
//! reference's catalogs (`src/api/i18n/<language>.json`).
//!
//! The C++ application shows every message id through gettext, English
//! included: English loads the `en` catalog, which words 520 of them
//! otherwise than the source does ("Advance" reads "Advanced"). So here:
//! a message id is written as the C++ source writes it, and [`tr`] gives
//! the current language's translation of it, or the message id itself
//! when the table has none. [`trc`] is the same for a message id with a
//! context (`L_CONTEXT`), [`trn`] for one with a plural (`_L_PLURAL`).
//! Placeholders (`%1%`, `%s`, `%d`, `{...}`) are left as they are; [`fill`]
//! puts the values in after.
//!
//! The current language is the session's. A call of the page runs with it
//! ([`with`], from `Session::call`), so what a call composes is in that
//! language; anything else (a test, a thread of its own) is in English, as
//! the C++ shows it.
//!
//! Which languages there are and how one is chosen follows
//! `GUI_App::load_language` and `PreferencesDialog::create_general_page`
//! (Preferences.cpp l.1925–1966): English and the 19 the reference ships a
//! catalog for, listed by their English names (wxWidgets' descriptions)
//! and shown by the names Preferences gives them.

use std::{
    cell::RefCell,
    collections::HashMap,
    fmt::Display,
    sync::{Arc, OnceLock},
};

use serde_json::Value;

/// A language the app offers: the name it is kept under (wxWidgets'
/// canonical name, as the reference keeps `language` in its
/// `app_config`), the folder of its catalog, and the name the list shows.
pub struct Language {
    pub code: &'static str,
    pub catalog: &'static str,
    pub name: &'static str,
    table: &'static str,
}

macro_rules! language {
    ($code:literal, $catalog:literal, $name:literal) => {
        Language {
            code: $code,
            catalog: $catalog,
            name: $name,
            table: include_str!(concat!("i18n/", $catalog, ".json")),
        }
    };
}

/// In the reference's order: sorted by wxWidgets' English description
/// ("Catalan", "Chinese", "Chinese (Simplified)", "Czech", ...), shown by
/// the names `create_item_language_combobox` gives them (the ones it does
/// not rename keep their description). English's canonical name is
/// wxWidgets' `wxLANGUAGE_ENGLISH`, "en_GB"; its catalog is `en`, which
/// wxWidgets loads for any English but the message ids' own (US).
pub static LANGUAGES: [Language; 20] = [
    language!("ca_ES", "ca", "Catalan"),
    language!("zh_TW", "zh_TW", "中文(繁体)"),
    language!("zh_CN", "zh_CN", "中文(简体)"),
    language!("cs_CZ", "cs", "Czech"),
    language!("nl_NL", "nl", "Nederlands"),
    language!("en_GB", "en", "English"),
    language!("fr_FR", "fr", "Français"),
    language!("de_DE", "de", "Deutsch"),
    language!("hu_HU", "hu", "Magyar"),
    language!("it_IT", "it", "italiano"),
    language!("ja_JP", "ja", "日本語"),
    language!("ko_KR", "ko", "한국어"),
    language!("lt_LT", "lt", "Lietuvių"),
    language!("pl_PL", "pl", "Polski"),
    language!("pt_BR", "pt_BR", "Português (Brasil)"),
    language!("ru_RU", "ru", "Русский"),
    language!("es_ES", "es", "Español"),
    language!("sv_SE", "sv", "Svenska"),
    language!("tr_TR", "tr", "Turkish"),
    language!("uk_UA", "uk", "Ukrainian"),
];

/// English, the language when none can be found.
pub const ENGLISH: &str = "en_GB";

/// One language's table, read.
#[derive(Debug)]
pub struct Catalog {
    pub code: &'static str,
    messages: HashMap<String, String>,
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the session composes nothing with a context or a plural yet; the page does"
        )
    )]
    plurals: HashMap<String, Vec<String>>,
    /// The plural form for n = 0 to 199; past that, that of
    /// 100 + n % 100 (see the tool).
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the session composes nothing with a context or a plural yet; the page does"
        )
    )]
    forms: Vec<u8>,
}

impl Catalog {
    fn read(language: &Language) -> Self {
        let table: Value =
            serde_json::from_str(language.table).expect("a table the tool wrote is JSON");
        let texts = |key: &str| -> HashMap<String, String> {
            table[key]
                .as_object()
                .into_iter()
                .flatten()
                .filter_map(|(id, text)| Some((id.clone(), text.as_str()?.to_owned())))
                .collect()
        };
        let plurals = table["plurals"]
            .as_object()
            .into_iter()
            .flatten()
            .map(|(id, forms)| {
                let forms = forms.as_array().into_iter().flatten();
                let forms = forms.filter_map(|form| form.as_str().map(str::to_owned));
                (id.clone(), forms.collect())
            })
            .collect();
        let forms = table["plural"]
            .as_str()
            .unwrap_or("")
            .bytes()
            .map(|digit| digit.saturating_sub(b'0'))
            .collect();
        Self {
            code: language.code,
            messages: texts("messages"),
            plurals,
            forms,
        }
    }

    /// The translation of `msgid`, or `msgid` itself.
    pub fn tr<'a>(&'a self, msgid: &'a str) -> &'a str {
        self.messages.get(msgid).map_or(msgid, String::as_str)
    }

    /// The translation of `msgid` in `context` (gettext's key: the
    /// context, U+0004, the message id), or `msgid`.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the session composes nothing with a context or a plural yet; the page does"
        )
    )]
    pub fn trc(&self, context: &str, msgid: &str) -> String {
        let key = format!("{context}\u{4}{msgid}");
        self.messages
            .get(&key)
            .cloned()
            .unwrap_or_else(|| msgid.to_owned())
    }

    /// The form of `singular` for `n`, or `singular` (n = 1) or `plural`
    /// as they are when the table has none, as gettext gives them.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the session composes nothing with a context or a plural yet; the page does"
        )
    )]
    pub fn trn(&self, singular: &str, plural: &str, n: u64) -> String {
        if let Some(forms) = self.plurals.get(singular) {
            let at = if n < 200 { n } else { 100 + n % 100 } as usize;
            let form = self.forms.get(at).copied().unwrap_or(0) as usize;
            if let Some(text) = forms.get(form) {
                return text.clone();
            }
        }
        if n == 1 { singular } else { plural }.to_owned()
    }
}

/// The language kept under `code`, or under its language alone
/// ("de_AT" finds German, as wxWidgets looks for `de_AT` and then `de`).
pub fn find(code: &str) -> Option<&'static Language> {
    let code = code.replace('-', "_");
    LANGUAGES
        .iter()
        .find(|language| language.code.eq_ignore_ascii_case(&code))
        .or_else(|| {
            LANGUAGES
                .iter()
                .find(|language| language.catalog.eq_ignore_ascii_case(&code))
        })
        .or_else(|| {
            let base = code.split('_').next()?;
            // The two Chinese are told apart by their region or script,
            // never by the language alone.
            (base != "zh")
                .then(|| {
                    LANGUAGES
                        .iter()
                        .find(|language| language.catalog.eq_ignore_ascii_case(base))
                })
                .flatten()
        })
}

/// The language of the system's language `tag` (a BCP 47 tag such as
/// "de-AT", "zh-Hans-CN", "pt-BR", or a locale such as "de_DE.UTF-8"),
/// when the reference has a catalog for it. Chinese is told by its
/// script, else its region (Taiwan, Hong Kong and Macao read Traditional);
/// Portuguese other than Brazil's has none, as in the reference, which
/// looks for `pt_PT` and then `pt`.
pub fn of_system(tag: &str) -> Option<&'static Language> {
    let tag = tag.split(['.', '@']).next()?.replace('-', "_");
    let parts: Vec<String> = tag.split('_').map(str::to_ascii_lowercase).collect();
    let base = parts.first()?.as_str();
    match base {
        "zh" => {
            let traditional = parts
                .iter()
                .skip(1)
                .any(|part| matches!(part.as_str(), "hant" | "tw" | "hk" | "mo"));
            find(if traditional { "zh_TW" } else { "zh_CN" })
        }
        "pt" => parts
            .get(1)
            .filter(|region| region.as_str() == "br")
            .and_then(|_| find("pt_BR")),
        _ => find(base),
    }
}

/// The table of `language`, read the first time it is asked for.
pub fn catalog(language: &'static Language) -> Arc<Catalog> {
    static READ: OnceLock<Vec<OnceLock<Arc<Catalog>>>> = OnceLock::new();
    let read = READ.get_or_init(|| LANGUAGES.iter().map(|_| OnceLock::new()).collect());
    let at = LANGUAGES
        .iter()
        .position(|other| std::ptr::eq(other, language))
        .expect("a language of the list");
    read[at]
        .get_or_init(|| Arc::new(Catalog::read(language)))
        .clone()
}

/// English, as the C++ shows it: the `en` catalog over the message ids.
pub fn english() -> Arc<Catalog> {
    catalog(find(ENGLISH).expect("English is in the list"))
}

/// The table as the page is handed it: the tool's JSON, untouched.
pub fn table_json(language: &Language) -> Value {
    serde_json::from_str(language.table).expect("a table the tool wrote is JSON")
}

thread_local! {
    static CURRENT: RefCell<Option<Arc<Catalog>>> = const { RefCell::new(None) };
}

/// Runs `work` with `catalog` as the language of what it composes.
pub fn with<R>(catalog: Arc<Catalog>, work: impl FnOnce() -> R) -> R {
    let before = CURRENT.with(|current| current.replace(Some(catalog)));
    struct Restore(Option<Arc<Catalog>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            let before = self.0.take();
            CURRENT.with(|current| *current.borrow_mut() = before);
        }
    }
    let _restore = Restore(before);
    work()
}

/// The language of what is composed now: the call's, else English.
pub fn current() -> Arc<Catalog> {
    CURRENT
        .with(|current| current.borrow().clone())
        .unwrap_or_else(english)
}

/// The translation of `msgid` in the current language, or `msgid`.
pub fn tr(msgid: &str) -> String {
    current().tr(msgid).to_owned()
}

/// The same for a message id with a context.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the session composes nothing with a context or a plural yet; the page does"
    )
)]
pub fn trc(context: &str, msgid: &str) -> String {
    current().trc(context, msgid)
}

/// The same for a message id with a plural, for `n`.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the session composes nothing with a context or a plural yet; the page does"
    )
)]
pub fn trn(singular: &str, plural: &str, n: u64) -> String {
    current().trn(singular, plural, n)
}

/// `text` with `values` put in its placeholders, as the C++ formats it:
/// boost::format's `%1%`, printf's `%1$d` (by position) and `%s`, `%d`,
/// `%.2f` (in order; a precision is kept for a number), `%%` a percent
/// sign. A placeholder with no value is left as it is.
pub fn fill(text: &str, values: &[&dyn Display]) -> String {
    let mut out = String::with_capacity(text.len() + 16);
    let mut next = 0;
    let mut rest = text;
    while let Some(at) = rest.find('%') {
        out.push_str(&rest[..at]);
        let tail = &rest[at + 1..];
        if let Some(stripped) = tail.strip_prefix('%') {
            out.push('%');
            rest = stripped;
            continue;
        }
        match placeholder(tail) {
            Some((length, position, precision)) => {
                let index = position.unwrap_or_else(|| {
                    next += 1;
                    next
                });
                match values.get(index.wrapping_sub(1)) {
                    Some(value) => {
                        let shown = value.to_string();
                        match (precision, shown.parse::<f64>()) {
                            (Some(digits), Ok(number)) => {
                                out.push_str(&format!("{number:.digits$}"));
                            }
                            _ => out.push_str(&shown),
                        }
                    }
                    None => out.push_str(&rest[at..at + 1 + length]),
                }
                rest = &tail[length..];
            }
            None => {
                out.push('%');
                rest = tail;
            }
        }
    }
    out.push_str(rest);
    out
}

/// A placeholder at the start of `tail` (after its `%`): how long it is,
/// the position it names, and the precision it asks for.
fn placeholder(tail: &str) -> Option<(usize, Option<usize>, Option<usize>)> {
    let bytes = tail.as_bytes();
    let digits = bytes.iter().take_while(|b| b.is_ascii_digit()).count();
    // %1%
    if digits > 0 && bytes.get(digits) == Some(&b'%') {
        return Some((digits + 1, tail[..digits].parse().ok(), None));
    }
    let mut at = 0;
    let mut position = None;
    // %1$d
    if digits > 0 && bytes.get(digits) == Some(&b'$') {
        position = tail[..digits].parse().ok();
        at = digits + 1;
    }
    while at < bytes.len() && b"-+ #0'".contains(&bytes[at]) {
        at += 1;
    }
    while at < bytes.len() && bytes[at].is_ascii_digit() {
        at += 1;
    }
    let mut precision = None;
    if bytes.get(at) == Some(&b'.') {
        let start = at + 1;
        at = start;
        while at < bytes.len() && bytes[at].is_ascii_digit() {
            at += 1;
        }
        precision = tail[start..at].parse().ok();
    }
    while at < bytes.len() && b"hlLqjzt".contains(&bytes[at]) {
        at += 1;
    }
    let conversion = *bytes.get(at)?;
    // A precision is a number's; `%.3s` cuts a text, which no message does.
    let precision = precision.filter(|_| b"eEfFgG".contains(&conversion));
    b"diouxXeEfFgGsc"
        .contains(&conversion)
        .then_some((at + 1, position, precision))
}

#[cfg(test)]
pub(crate) mod tests;
