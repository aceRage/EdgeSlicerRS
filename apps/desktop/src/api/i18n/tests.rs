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
