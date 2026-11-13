//! The hand-offs' pure parts, against the reference's own cases
//! (tests/slic3rutils/bambu_connect_tests.cpp,
//! bambu_studio_launcher_tests.cpp). Nothing here starts a program.

use super::*;

#[test]
fn the_url_is_encoded_as_encode_uri_component_does() {
    assert_eq!(
        encode_uri_component("/tmp/cube.gcode.3mf"),
        "%2Ftmp%2Fcube.gcode.3mf"
    );
    assert_eq!(encode_uri_component("Cube"), "Cube");
    assert_eq!(encode_uri_component("A-Z_a.z!~*'()09"), "A-Z_a.z!~*'()09");
    assert_eq!(encode_uri_component("a b"), "a%20b");
    assert_eq!(
        encode_uri_component("&=?#+%:;,@$"),
        "%26%3D%3F%23%2B%25%3A%3B%2C%40%24"
    );
    assert_eq!(encode_uri_component("C:\\Users\\x"), "C%3A%5CUsers%5Cx");
    assert_eq!(encode_uri_component(""), "");
    assert_eq!(encode_uri_component("café"), "caf%C3%A9");
    assert_eq!(encode_uri_component("漢字"), "%E6%BC%A2%E5%AD%97");
    assert_eq!(encode_uri_component("🙂"), "%F0%9F%99%82");
}

#[test]
fn the_url_is_the_one_bambu_documents_byte_for_byte() {
    assert_eq!(
        connect_url(
            "/Users/me/tmp/EdgeSlicerRS/bambu-connect/My Cube (v2) 漢.gcode.3mf",
            "My Cube (v2) 漢"
        ),
        "bambu-connect://import-file?path=%2FUsers%2Fme%2Ftmp%2FEdgeSlicerRS%2Fbambu-connect%2FMy%20Cube%20(v2)%20%E6%BC%A2.gcode.3mf&name=My%20Cube%20(v2)%20%E6%BC%A2&version=1.0.0"
    );
    assert_eq!(
        connect_url(r"C:\Temp\a&b.gcode.3mf", "a_b"),
        "bambu-connect://import-file?path=C%3A%5CTemp%5Ca%26b.gcode.3mf&name=a_b&version=1.0.0"
    );
    assert!(is_connect_url(&connect_url("/x y", "x")));
    assert!(!is_connect_url("https://example.com"));
    assert!(!is_connect_url("bambu-connect://import-file?path=a b"));
}

#[test]
fn the_jobs_name_is_the_file_name_made_safe() {
    for (file, name) in [
        ("Cube_PLA_1h2m.gcode.3mf", "Cube_PLA_1h2m"),
        ("Cube.GCODE.3MF", "Cube"),
        ("Cube.3mf", "Cube"),
        ("Cube.gcode", "Cube"),
        ("C:\\out\\Cube.gcode.3mf", "Cube"),
        ("/tmp/out/Cube.gcode.3mf", "Cube"),
        ("my.model.v2.gcode.3mf", "my.model.v2"),
        ("A&B=C?D#E%F+G.gcode.3mf", "A_B_C_D_E_F_G"),
        ("  tab\tbed \x01\x7f.gcode.3mf", "tabbed"),
        (".gcode.3mf", "plate"),
        ("", "plate"),
        ("Café 漢.gcode.3mf", "Café 漢"),
    ] {
        assert_eq!(connect_name(file), name, "{file:?}");
    }
}

#[test]
fn the_files_name_is_one_every_system_takes() {
    for (name, file) in [
        ("Cube", "Cube.gcode.3mf"),
        ("a<b>c:d\"e/f\\g|h?i*j", "a_b_c_d_e_f_g_h_i_j.gcode.3mf"),
        ("a&b=c#d%e+f", "a_b_c_d_e_f.gcode.3mf"),
        ("trailing. . ", "trailing.gcode.3mf"),
        ("", "plate.gcode.3mf"),
        ("...", "plate.gcode.3mf"),
        ("CON", "CON_.gcode.3mf"),
        ("com1", "com1_.gcode.3mf"),
        ("console", "console.gcode.3mf"),
        ("Café", "Café.gcode.3mf"),
    ] {
        assert_eq!(connect_file_name(name), file, "{name:?}");
    }
    let long = "x".repeat(150);
    assert_eq!(
        connect_file_name(&long),
        format!("{}.gcode.3mf", "x".repeat(100))
    );
    // A two-byte character across the cut is left out whole.
    let mixed = format!("{}é", "y".repeat(99));
    assert_eq!(
        connect_file_name(&mixed),
        format!("{}.gcode.3mf", "y".repeat(99))
    );
    let fits = format!("{}é", "z".repeat(98));
    assert_eq!(connect_file_name(&fits), format!("{fits}.gcode.3mf"));
}

#[test]
fn only_old_sliced_files_are_cleared_from_the_folder() {
    let day = 24 * 60 * 60;
    let now = 10 * day;
    let entries = vec![
        ("old.gcode.3mf".to_owned(), now - 4 * day),
        ("new.gcode.3mf".to_owned(), now - day),
        ("old.txt".to_owned(), now - 4 * day),
        ("OLD.GCODE.3MF".to_owned(), now - 5 * day),
        ("unknown.gcode.3mf".to_owned(), 0),
    ];
    assert_eq!(
        stale_connect_files(&entries, now, CONNECT_KEPT_SECONDS),
        ["old.gcode.3mf", "OLD.GCODE.3MF"]
    );
}

#[test]
fn bambu_connect_is_found_as_the_reference_finds_it() {
    let full = WindowsRegistration {
        key_exists: true,
        has_url_protocol: true,
        open_command: r#""C:\Program Files\Bambu Connect\Bambu Connect.exe" "%1""#.into(),
    };
    let there = |path: &str| path == r"C:\Program Files\Bambu Connect\Bambu Connect.exe";
    assert_eq!(connect_on_windows(&full, &there), Installed::Yes);
    assert_eq!(connect_on_windows(&full, &|_| false), Installed::No);
    let no_protocol = WindowsRegistration {
        has_url_protocol: false,
        ..full.clone()
    };
    assert_eq!(connect_on_windows(&no_protocol, &there), Installed::No);
    let variable = WindowsRegistration {
        open_command: r"%LOCALAPPDATA%\Bambu Connect\x.exe %1".into(),
        ..full.clone()
    };
    assert_eq!(connect_on_windows(&variable, &|_| false), Installed::Yes);
    assert_eq!(
        executable_from_command(r"C:\a b\x.EXE --open %1"),
        r"C:\a b\x.EXE"
    );
    assert_eq!(executable_from_command("handler %1"), "handler");
    assert_eq!(connect_on_linux(None), Installed::Unknown);
    assert_eq!(connect_on_linux(Some(" \n")), Installed::No);
    assert_eq!(
        connect_on_linux(Some("bambu-connect.desktop\n")),
        Installed::Yes
    );
    let plist = b"<key>CFBundleURLSchemes</key><array><string>bambu-connect</string></array>";
    assert_eq!(
        connect_on_macos(&[None, Some(plist.to_vec())]),
        Installed::Yes
    );
    assert_eq!(
        connect_on_macos(&[Some(b"<plist/>".to_vec()), None]),
        Installed::No
    );
    assert_eq!(
        reg_value(
            "\nHKEY_CLASSES_ROOT\\bambu-connect\\shell\\open\\command\n    (Default)    REG_SZ    \"C:\\x.exe\" \"%1\"\n"
        ),
        Some("\"C:\\x.exe\" \"%1\"".into())
    );
}

#[test]
fn bambu_studio_is_started_with_the_references_arguments() {
    let file = "/Users/me/My Cube.3mf";
    assert_eq!(
        macos_by_name(file),
        ["open", "-a", "BambuStudio", "/Users/me/My Cube.3mf"]
    );
    assert_eq!(
        macos_by_bundle(file),
        [
            "open",
            "-b",
            "com.bambulab.bambu-studio",
            "/Users/me/My Cube.3mf"
        ]
    );
    assert_eq!(
        linux_on_path(file),
        ["bambu-studio", "/Users/me/My Cube.3mf"]
    );
    assert_eq!(
        linux_flatpak(file),
        [
            "flatpak",
            "run",
            "com.bambulab.BambuStudio",
            "/Users/me/My Cube.3mf"
        ]
    );
    let tries = bambu_studio_tries(file);
    assert!(!tries.is_empty() && tries.iter().all(|argv| argv.last().unwrap() == file));
}

#[test]
fn windows_arguments_are_quoted_as_command_line_to_argv_reads_them() {
    assert_eq!(win_quote("plain"), "plain");
    assert_eq!(win_quote(""), "\"\"");
    assert_eq!(win_quote("a b"), "\"a b\"");
    assert_eq!(win_quote("say \"hi\""), "\"say \\\"hi\\\"\"");
    assert_eq!(win_quote(r"C:\dir with space\"), r#""C:\dir with space\\""#);
    assert_eq!(win_quote(r#"a\"b c"#), r#""a\\\"b c""#);
    assert_eq!(
        win_command_line(
            r"C:\Program Files\Bambu Studio\bambu-studio.exe",
            r"C:\Users\me\My Cube.3mf"
        ),
        r#""C:\Program Files\Bambu Studio\bambu-studio.exe" "C:\Users\me\My Cube.3mf""#
    );
}

#[test]
fn windows_finds_bambu_studio_in_the_references_order() {
    let at = |paths: &'static [&'static str]| move |path: &str| paths.contains(&path);
    let app = vec![
        String::new(),
        r"D:\BS\bambu-studio.exe".to_owned(),
        String::new(),
        String::new(),
    ];
    let entries = vec![
        UninstallEntry {
            display_name: "Bambu Connect".into(),
            install_location: r"E:\Connect".into(),
            ..Default::default()
        },
        UninstallEntry {
            display_name: "Bambu Studio 2.8".into(),
            install_location: r"E:\Studio\".into(),
            display_icon: r"F:\Icon\bambu-studio.exe,0".into(),
        },
    ];
    // App Paths first.
    assert_eq!(
        discover_windows(
            &app,
            &entries,
            "",
            &at(&[r"D:\BS\bambu-studio.exe", WIN_DEFAULT])
        ),
        Some(r"D:\BS\bambu-studio.exe".into())
    );
    // Then the install folder (never Bambu Connect's), then the icon.
    assert_eq!(
        discover_windows(
            &[],
            &entries,
            "",
            &at(&[r"E:\Studio\bambu-studio.exe", WIN_DEFAULT])
        ),
        Some(r"E:\Studio\bambu-studio.exe".into())
    );
    assert_eq!(
        discover_windows(&[], &entries, "", &at(&[r"F:\Icon\bambu-studio.exe"])),
        Some(r"F:\Icon\bambu-studio.exe".into())
    );
    assert_eq!(
        discover_windows(&[], &[], "", &at(&[r"E:\Connect\bambu-studio.exe"])),
        None
    );
    // The program `.3mf` opens with, only when it is Bambu Studio.
    assert_eq!(
        discover_windows(
            &[],
            &[],
            r"G:\Other\slicer.exe",
            &at(&[r"G:\Other\slicer.exe"])
        ),
        None
    );
    assert_eq!(
        discover_windows(
            &[],
            &[],
            r"G:\BambuStudio.exe",
            &at(&[r"G:\BambuStudio.exe"])
        ),
        Some(r"G:\BambuStudio.exe".into())
    );
    assert_eq!(
        discover_windows(&[], &[], "", &at(&[WIN_DEFAULT])),
        Some(WIN_DEFAULT.into())
    );
    assert!(is_bambu_studio_name("BambuStudio"));
    assert!(!is_bambu_studio_name("Bambu Handy"));
}

#[test]
fn a_program_is_found_on_the_path() {
    let path = std::env::join_paths([Path::new("/a"), Path::new("/b")]).unwrap();
    let path = path.to_string_lossy();
    let there = |p: &Path| p == Path::new("/b").join("bambu-studio");
    assert!(on_path(&path, "bambu-studio", &there));
    assert!(!on_path(&path, "flatpak", &there));
}
