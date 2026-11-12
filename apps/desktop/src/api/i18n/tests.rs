use super::*;

#[test]
fn english_shows_the_message_ids_as_the_cpp_shows_them() {
    let english = english();
    // ParamsPanel.cpp l.229: the process tab's switch.
    assert_eq!(english.tr("Advance"), "Advanced");
    assert_eq!(english.tr("Lay on face"), "Lay on Face");
    assert_eq!(english.tr("Top Solid Layers"), "Top solid layers");
    // What the en catalog leaves alone is the message id.
    assert_eq!(english.tr("Layer height"), "Layer height");
    assert_eq!(english.tr("not a message id"), "not a message id");
    let count = |language: &Language| {
        table_json(language)["messages"]
            .as_object()
            .map_or(0, |m| m.len())
    };
    assert!(count(find(ENGLISH).unwrap()) > 500);
}

#[test]
fn every_language_has_its_table_and_reads_it() {
    let mut codes = std::collections::HashSet::new();
    for language in &LANGUAGES {
        assert!(codes.insert(language.code), "{} twice", language.code);
        let catalog = catalog(language);
        assert_eq!(catalog.code, language.code);
        let table = table_json(language);
        let count = table["messages"].as_object().map_or(0, |m| m.len());
        assert!(count > 500, "{}: {count}", language.code);
        assert_eq!(table["language"], language.catalog);
        assert_eq!(table["plural"].as_str().map(str::len), Some(200));
    }
    // A table for every catalog the tool wrote, and the other way round.
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/api/i18n");
    let mut written: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|entry| {
            let name = entry.ok()?.file_name().into_string().ok()?;
            name.strip_suffix(".json").map(str::to_owned)
        })
        .collect();
    written.sort();
    let mut listed: Vec<String> = LANGUAGES.iter().map(|l| l.catalog.to_owned()).collect();
    listed.sort();
    assert_eq!(written, listed);
}

#[test]
fn a_translation_is_found_with_its_context_and_its_plural() {
    let german = catalog(find("de_DE").unwrap());
    assert_eq!(german.tr("Advance"), "Erweitert");
    // "Top" of the layers (msgctxt "Layers").
    assert_ne!(german.trc("Layers", "Top"), "Top");
    assert_eq!(german.trc("No such context", "Top"), "Top");
    let one = german.trn("%1$d error repaired", "%1$d errors repaired", 1);
    let two = german.trn("%1$d error repaired", "%1$d errors repaired", 2);
    assert_eq!(one, "%1$d Fehler repariert");
    assert_eq!(two, "%1$d Fehler repariert");
    // A message id with no plural in the table is gettext's: the
    // singular for one, the plural for any other number.
    assert_eq!(german.trn("one thing", "%d things", 1), "one thing");
    assert_eq!(german.trn("one thing", "%d things", 0), "%d things");
    // Russian has three forms, worked out for any n.
    let russian = catalog(find("ru").unwrap());
    let forms: Vec<String> = [1, 2, 5, 11, 21, 22, 25, 101, 111, 1001, 1_000_002]
        .iter()
        .map(|&n| russian.trn("%1$d error repaired", "%1$d errors repaired", n))
        .collect();
    assert_eq!(forms[0], forms[4]); // 1, 21
    assert_eq!(forms[0], forms[7]); // 101
    assert_eq!(forms[0], forms[9]); // 1001
    assert_eq!(forms[1], forms[5]); // 2, 22
    assert_eq!(forms[1], forms[10]); // 1000002
    assert_eq!(forms[2], forms[3]); // 5, 11
    assert_eq!(forms[2], forms[8]); // 111
    assert_ne!(forms[0], forms[2]);
}

#[test]
fn a_language_is_found_from_the_systems() {
    let code = |tag: &str| of_system(tag).map(|language| language.code);
    assert_eq!(code("de-AT"), Some("de_DE"));
    assert_eq!(code("de_DE.UTF-8"), Some("de_DE"));
    assert_eq!(code("fr"), Some("fr_FR"));
    assert_eq!(code("en-US"), Some("en_GB"));
    assert_eq!(code("zh-Hans-CN"), Some("zh_CN"));
    assert_eq!(code("zh-CN"), Some("zh_CN"));
    assert_eq!(code("zh-Hant"), Some("zh_TW"));
    assert_eq!(code("zh-HK"), Some("zh_TW"));
    assert_eq!(code("pt-BR"), Some("pt_BR"));
    assert_eq!(code("pt-PT"), None);
    assert_eq!(code("fi-FI"), None);
    assert_eq!(code(""), None);
    assert_eq!(find("de_DE").map(|l| l.catalog), Some("de"));
    assert_eq!(find("de").map(|l| l.code), Some("de_DE"));
    assert_eq!(find("en_US").map(|l| l.code), Some("en_GB"));
    assert!(find("zh").is_none());
}

#[test]
fn a_call_composes_in_its_language_and_anything_else_in_english() {
    let german = catalog(find("de_DE").unwrap());
    assert_eq!(tr("Advance"), "Advanced");
    let inside = with(german, || {
        // Nested, as a call that makes another does.
        let nested = with(english(), || tr("Advance"));
        (tr("Advance"), nested, tr("Advance"))
    });
    assert_eq!(
        inside,
        ("Erweitert".into(), "Advanced".into(), "Erweitert".into())
    );
    assert_eq!(tr("Advance"), "Advanced");
    // The context and the plural follow the call's language as well.
    let german = catalog(find("de_DE").unwrap());
    let (top, errors) = with(german.clone(), || {
        (
            trc("Layers", "Top"),
            trn("%1$d error repaired", "%1$d errors repaired", 3),
        )
    });
    assert_eq!(top, german.trc("Layers", "Top"));
    assert_eq!(errors, "%1$d Fehler repariert");
    assert_eq!(trc("Layers", "Top"), english().trc("Layers", "Top"));
    assert_eq!(
        trn("%1$d error repaired", "%1$d errors repaired", 3),
        "%1$d errors repaired"
    );
}

#[test]
fn values_go_into_the_placeholders_as_the_cpp_puts_them() {
    assert_eq!(fill("Plate %d", &[&3]), "Plate 3");
    assert_eq!(fill("%1% of %2%", &[&"a", &"b"]), "a of b");
    assert_eq!(fill("%2% before %1%", &[&"a", &"b"]), "b before a");
    assert_eq!(fill("%1$d error repaired", &[&4]), "4 error repaired");
    assert_eq!(fill("%s: %.2f mm", &[&"Width", &0.4567]), "Width: 0.46 mm");
    assert_eq!(fill("100%% of %s", &[&"it"]), "100% of it");
    // A percent sign that is no placeholder, and one with no value.
    assert_eq!(fill("50 % done %s", &[]), "50 % done %s");
    assert_eq!(fill("{layer_num} %d", &[&1]), "{layer_num} 1");
}

// ---- every word shown: a message id, or listed ----

/// The message ids of the reference's template
/// (`localization/i18n/Snapmaker_Orca.pot`), read as far as this needs:
/// each `msgid`, its lines joined and its escapes read. A string the
/// template never closes (it has one) is taken as it stands.
fn message_ids() -> std::collections::HashSet<String> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../reference/EdgeSlicer/localization/i18n/Snapmaker_Orca.pot");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    let unquote = |line: &str| -> String {
        let inner = line.trim().strip_prefix('"').unwrap_or(line);
        let inner = inner.strip_suffix('"').unwrap_or(inner);
        let mut out = String::new();
        let mut chars = inner.chars();
        while let Some(c) = chars.next() {
            if c != '\\' {
                out.push(c);
                continue;
            }
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some(other) => out.push(other),
                None => {}
            }
        }
        out
    };
    let mut ids = std::collections::HashSet::new();
    let mut current: Option<String> = None;
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("msgid ") {
            current = Some(unquote(rest));
        } else if line.starts_with('"') {
            if let Some(id) = current.as_mut() {
                id.push_str(&unquote(line));
            }
        } else if let Some(id) = current.take()
            && !id.is_empty()
        {
            ids.insert(id);
        }
    }
    ids
}

/// A JavaScript string literal at the start of `text` (after its quote),
/// read: its text, or None for a template with a `${…}` in it.
fn js_literal(text: &str) -> Option<String> {
    let mut chars = text.chars();
    let quote = chars.next()?;
    let mut out = String::new();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next()? {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                'u' => {
                    let code: String = chars.by_ref().take(4).collect();
                    out.push(char::from_u32(u32::from_str_radix(&code, 16).ok()?)?);
                }
                other => out.push(other),
            },
            '$' if quote == '`' && chars.clone().next() == Some('{') => return None,
            c if c == quote => return Some(out),
            c => out.push(c),
        }
    }
    None
}

/// What `call(` is given first wherever it is called as a function in
/// `source` (not as a method, nor as part of a longer name): the literal
/// texts, by line.
fn literals_of(source: &str, call: &str, rust: bool) -> Vec<(usize, String)> {
    let mut found = Vec::new();
    let opening = format!("{call}(");
    let mut from = 0;
    while let Some(at) = source[from..].find(&opening) {
        let start = from + at;
        from = start + opening.len();
        let before = source[..start].chars().next_back();
        if before.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '.' || c == '$') {
            continue;
        }
        let rest = source[from..].trim_start();
        let literal = if rust {
            rest.strip_prefix('"')
                .and_then(|rest| js_literal(&format!("\"{rest}")))
        } else if rest.starts_with(['\'', '"', '`']) {
            js_literal(rest)
        } else {
            None
        };
        if let Some(text) = literal {
            found.push((source[..start].lines().count(), text));
        }
    }
    found
}

/// The words of a static page: the text between its tags (not in a
/// script, a style or a drawing) and its titles, labels and placeholders,
/// spaces made one.
fn html_words(page: &str) -> Vec<(usize, String)> {
    let mut found = Vec::new();
    let tidy = |text: &str| text.split_whitespace().collect::<Vec<_>>().join(" ");
    let line_of = |at: usize| page[..at].lines().count();
    let mut at = 0;
    let mut skipping: Option<&str> = None;
    while let Some(open) = page[at..].find('<') {
        let open = at + open;
        let text = tidy(&decode(&page[at..open]));
        if skipping.is_none() && text.chars().any(char::is_alphabetic) {
            found.push((line_of(at), text));
        }
        let close = open + page[open..].find('>').expect("a tag that ends");
        let tag = &page[open + 1..close];
        let name: String = tag
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '/' || *c == '!')
            .collect();
        match name.as_str() {
            "script" | "style" | "svg" => {
                skipping = Some(if name == "svg" {
                    "/svg"
                } else if name == "script" {
                    "/script"
                } else {
                    "/style"
                })
            }
            closing if Some(closing) == skipping => skipping = None,
            _ => {}
        }
        if skipping.is_none() && !name.starts_with('!') {
            for attribute in ["title", "aria-label", "placeholder"] {
                let key = format!(" {attribute}=\"");
                if let Some(start) = tag.find(&key) {
                    let value = &tag[start + key.len()..];
                    let value = &value[..value.find('"').unwrap_or(value.len())];
                    let value = tidy(&decode(value));
                    if value.chars().any(char::is_alphabetic) {
                        found.push((line_of(open), value));
                    }
                }
            }
        }
        at = close + 1;
    }
    found
}

fn decode(text: &str) -> String {
    text.replace("&quot;", "\"")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

/// The list of what is shown and is not a message id (`wording.txt`):
/// the C++'s words newer than its catalog, and this app's own.
fn listed() -> (Vec<String>, Vec<String>) {
    let text = include_str!("wording.txt");
    let (mut newer, mut own) = (Vec::new(), Vec::new());
    let mut section = "";
    for line in text.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        if line == "[newer]" || line == "[own]" {
            section = if line == "[newer]" { "newer" } else { "own" };
            continue;
        }
        // The words, then a tab and where the reference has them.
        let words = line.split('\t').next().unwrap_or(line).replace("\\n", "\n");
        match section {
            "newer" => newer.push(words),
            "own" => own.push(words),
            _ => panic!("wording.txt: {line} is in no section"),
        }
    }
    (newer, own)
}

/// Every string the page passes through `t` (and `tc`, `tn`), every one
/// the session passes through `tr`, and every word of the static page
/// (`index.html`, which i18n.js puts in the language) is a message id of
/// the reference's catalog, as the C++ source writes it; or it is listed
/// in `wording.txt`, as words of the C++ newer than its catalog or as this
/// app's own. A form that someone typed as the English reads (the en
/// catalog's "Advanced" for "Advance") is neither, and fails here: other
/// languages would not find it.
#[test]
fn every_word_shown_is_a_message_id_or_listed() {
    let ids = message_ids();
    assert!(ids.len() > 4000, "{} message ids", ids.len());
    assert!(ids.contains("Advance") && ids.contains("Lay on face"));
    let (newer, own) = listed();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut sources: Vec<(String, String)> = Vec::new();
    let mut read_dir = |dir: &std::path::Path, extension: &str| {
        let mut stack = vec![dir.to_path_buf()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|e| e == extension) {
                    let name = path
                        .strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/");
                    sources.push((name, std::fs::read_to_string(&path).unwrap()));
                }
            }
        }
    };
    read_dir(&root.join("ui"), "js");
    read_dir(&root.join("ui"), "html");
    read_dir(&root.join("src"), "rs");
    let mut shown: Vec<(String, usize, String)> = Vec::new();
    for (name, source) in &sources {
        if name.ends_with(".js") {
            for call in ["t", "tc", "tn"] {
                for (line, text) in literals_of(source, call, false) {
                    shown.push((name.clone(), line, text));
                }
            }
            // tc's message id is its second: none is used yet.
            assert!(
                literals_of(source, "tc", false).is_empty(),
                "{name}: tc is read as t here"
            );
        } else if name.ends_with(".html") {
            for (line, text) in html_words(source) {
                shown.push((name.clone(), line, text));
            }
        } else if !name.ends_with("tests.rs") && !name.contains("/tests/") {
            for (line, text) in literals_of(source, "tr", true) {
                shown.push((name.clone(), line, text));
            }
        }
    }
    assert!(shown.len() > 400, "{} strings found", shown.len());
    let mut unknown = Vec::new();
    for (name, line, text) in &shown {
        // "Words [Key]": the words are the message id, as a tooltip with
        // its key is built.
        let words = text
            .rsplit_once(" [")
            .filter(|(_, key)| key.ends_with(']'))
            .map_or(text.as_str(), |(words, _)| words);
        if ids.contains(text) || ids.contains(words) || newer.contains(text) || own.contains(text) {
            continue;
        }
        unknown.push(format!("{name}:{line}: {text:?}"));
    }
    unknown.sort();
    unknown.dedup();
    assert!(
        unknown.is_empty(),
        "not a message id of Snapmaker_Orca.pot, nor in wording.txt:\n{}",
        unknown.join("\n")
    );
    // The lists hold only what is still so.
    let everything: String = sources.iter().map(|(_, source)| source.as_str()).collect();
    for words in newer.iter().chain(&own) {
        assert!(
            !ids.contains(words),
            "wording.txt: {words:?} is a message id now; pass it through t or tr"
        );
        // As a script or the session may write it: a new line as `\n`, an
        // apostrophe as `\'`.
        let written = words.replace('\n', "\\n");
        let quoted = written.replace('\'', "\\'");
        assert!(
            [words.as_str(), &written, &quoted]
                .iter()
                .any(|form| everything.contains(form)),
            "wording.txt: {words:?} is shown nowhere any more"
        );
    }
}
