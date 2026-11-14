//! Handing a file to another application: Bambu Connect (its
//! `bambu-connect://` URL, `Utils/BambuConnect.cpp`) and Bambu Studio
//! (`Utils/BambuStudioLauncher.cpp`).
//!
//! Everything here but [`perform`] is pure: the URL, the names, the
//! argument lists and the decisions, tested without any other
//! application. [`perform`] is the one place the app starts another
//! program, and only the window does it, on the person's click (main.rs
//! `hand_off`): in a browser (`--serve`) and in every test the session
//! answers with what would be opened instead. It is not compiled into
//! the tests at all.

use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------
// Bambu Connect (BambuConnect.cpp)
// ---------------------------------------------------------------------

/// `URL_SCHEME`, `PROTOCOL_VERSION`.
pub const CONNECT_SCHEME: &str = "bambu-connect";
pub const CONNECT_VERSION: &str = "1.0.0";
/// Bambu's own page on Bambu Connect, shown as a link (`INFO_URL`).
pub const CONNECT_INFO_URL: &str = "https://wiki.bambulab.com/en/software/bambu-connect";
/// How long a file handed to Bambu Connect is kept: it is read when
/// Print is pressed there (`TEMP_MAX_AGE_S`, three days).
pub const CONNECT_KEPT_SECONDS: u64 = 3 * 24 * 60 * 60;

/// JavaScript's `encodeURIComponent` over the UTF-8 bytes: `A-Z a-z 0-9
/// - _ . ! ~ * ' ( )` stay, every other byte is `%XX` in upper-case hex
/// (`encode_uri_component`).
pub fn encode_uri_component(text: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(text.len() * 3);
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&byte) {
            out.push(char::from(byte));
        } else {
            out.push('%');
            out.push(char::from(HEX[usize::from(byte >> 4)]));
            out.push(char::from(HEX[usize::from(byte & 15)]));
        }
    }
    out
}

/// The import URL (`build_import_url`): the file's absolute path as the
/// system writes it, and the name Bambu Connect shows for the job.
pub fn connect_url(path: &str, name: &str) -> String {
    format!(
        "{CONNECT_SCHEME}://import-file?path={}&name={}&version={CONNECT_VERSION}",
        encode_uri_component(path),
        encode_uri_component(name)
    )
}

/// Whether a URL is one this app hands to Bambu Connect: the window
/// opens nothing else.
pub fn is_connect_url(url: &str) -> bool {
    url.starts_with("bambu-connect://import-file?path=")
        && !url.chars().any(|c| c.is_whitespace() || c.is_control())
}

fn ends_with_ignoring_case(text: &str, suffix: &str) -> bool {
    text.len() >= suffix.len()
        && text.as_bytes()[text.len() - suffix.len()..].eq_ignore_ascii_case(suffix.as_bytes())
}

fn trim_spaces(text: &str) -> &str {
    text.trim_matches(|c| c == ' ' || c == '\t')
}

/// The job's name from the export file's name (`display_name_for`): the
/// leaf, without `.gcode.3mf`, `.3mf` or `.gcode`, `& = ? # % +` made `_`
/// so no query parser can split it, control characters left out, spaces
/// trimmed; "plate" when nothing is left.
pub fn connect_name(file_name: &str) -> String {
    let leaf = file_name.rsplit(['/', '\\']).next().unwrap_or(file_name);
    let mut stem = leaf;
    for extension in [".gcode.3mf", ".3mf", ".gcode"] {
        if ends_with_ignoring_case(stem, extension) {
            stem = &stem[..stem.len() - extension.len()];
            break;
        }
    }
    let cleaned: String = stem
        .chars()
        .filter(|c| !(u32::from(*c) < 0x20 || *c == '\x7f'))
        .map(|c| if "&=?#%+".contains(c) { '_' } else { c })
        .collect();
    let out = trim_spaces(&cleaned);
    if out.is_empty() {
        "plate".into()
    } else {
        out.to_owned()
    }
}

/// At most `max` bytes of `text`, never cut inside a character.
fn cut_to(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

fn is_windows_device(stem: &str) -> bool {
    let lower = stem.to_ascii_lowercase();
    if ["con", "prn", "aux", "nul"].contains(&lower.as_str()) {
        return true;
    }
    let bytes = lower.as_bytes();
    bytes.len() == 4
        && (lower.starts_with("com") || lower.starts_with("lpt"))
        && (b'1'..=b'9').contains(&bytes[3])
}

/// The file's name in the folder handed to Bambu Connect
/// (`temp_file_name_for`): what no system takes in a name made `_`, at
/// most 100 bytes, trailing dots and spaces and leading spaces trimmed,
/// a Windows device name given a `_`, with `.gcode.3mf`.
pub fn connect_file_name(name: &str) -> String {
    let stem: String = name
        .chars()
        .filter(|c| !(u32::from(*c) < 0x20 || *c == '\x7f'))
        .map(|c| {
            if "<>:\"/\\|?*&=#%+".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    let stem = cut_to(&stem, 100)
        .trim_end_matches(['.', ' '])
        .trim_start_matches(' ');
    let mut stem = if stem.is_empty() {
        "plate".to_owned()
    } else {
        stem.to_owned()
    };
    if is_windows_device(&stem) {
        stem.push('_');
    }
    format!("{stem}.gcode.3mf")
}

/// The files of the folder handed to Bambu Connect that go
/// (`stale_temp_files`): only `.gcode.3mf` files, only those written more
/// than `kept` seconds before `now`. `(name, seconds since the epoch)`.
pub fn stale_connect_files(entries: &[(String, u64)], now: u64, kept: u64) -> Vec<String> {
    entries
        .iter()
        .filter(|(name, written)| {
            ends_with_ignoring_case(name, ".gcode.3mf")
                && *written > 0
                && now.saturating_sub(*written) > kept
        })
        .map(|(name, _)| name.clone())
        .collect()
}

/// Whether Bambu Connect is there to take the URL (`Installed`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Installed {
    Yes,
    No,
    /// Could not tell: the URL is opened and a failure said.
    Unknown,
}

/// What Windows has under `HKEY_CLASSES_ROOT\bambu-connect`.
#[derive(Clone, Debug, Default)]
pub struct WindowsRegistration {
    pub key_exists: bool,
    pub has_url_protocol: bool,
    pub open_command: String,
}

/// The program of a shell open command (`executable_from_command`).
pub fn executable_from_command(command: &str) -> String {
    let command = trim_spaces(command);
    if command.is_empty() {
        return String::new();
    }
    if let Some(rest) = command.strip_prefix('"') {
        return rest.split('"').next().unwrap_or(rest).to_owned();
    }
    if let Some(at) = command.to_ascii_lowercase().find(".exe") {
        return command[..at + 4].to_owned();
    }
    command.split(' ').next().unwrap_or(command).to_owned()
}

/// `detect_windows`: the key, its "URL Protocol" and an open command; a
/// command whose program is gone (left by an uninstall) is no Bambu
/// Connect, one naming an environment variable is trusted.
pub fn connect_on_windows(reg: &WindowsRegistration, exists: &dyn Fn(&str) -> bool) -> Installed {
    if !reg.key_exists || !reg.has_url_protocol || trim_spaces(&reg.open_command).is_empty() {
        return Installed::No;
    }
    let program = executable_from_command(&reg.open_command);
    if program.is_empty() || program.contains('%') || exists(&program) {
        Installed::Yes
    } else {
        Installed::No
    }
}

/// `detect_linux`: what `xdg-mime query default
/// x-scheme-handler/bambu-connect` said, none when it did not run.
pub fn connect_on_linux(said: Option<&str>) -> Installed {
    match said {
        None => Installed::Unknown,
        Some(text) if text.trim().is_empty() => Installed::No,
        Some(_) => Installed::Yes,
    }
}

/// macOS. **Differs**: the reference asks NSWorkspace which application
/// opens `bambu-connect://` (BambuConnectMac.mm), which needs AppKit;
/// here the places applications are installed are looked through for a
/// "Bambu Connect.app" whose `Info.plist` names the scheme. Given each
/// candidate's `Info.plist` as read (none where there is no bundle).
pub fn connect_on_macos(plists: &[Option<Vec<u8>>]) -> Installed {
    let names = |bytes: &[u8]| {
        bytes
            .windows(CONNECT_SCHEME.len())
            .any(|window| window == CONNECT_SCHEME.as_bytes())
    };
    if plists.iter().flatten().any(|bytes| names(bytes)) {
        Installed::Yes
    } else {
        Installed::No
    }
}

/// Where "Bambu Connect.app" is looked for on macOS.
#[cfg_attr(test, allow(dead_code))]
pub fn connect_bundles_macos(home: Option<&Path>) -> Vec<PathBuf> {
    let mut out = vec![PathBuf::from("/Applications/Bambu Connect.app")];
    if let Some(home) = home {
        out.push(home.join("Applications/Bambu Connect.app"));
    }
    out
}

// ---------------------------------------------------------------------
// Bambu Studio (BambuStudioLauncher.cpp)
// ---------------------------------------------------------------------

/// `macos_open_by_name_args`: `open -a BambuStudio <file>`.
pub fn macos_by_name(file: &str) -> Vec<String> {
    ["open", "-a", "BambuStudio", file]
        .map(str::to_owned)
        .into()
}

/// `macos_open_by_bundle_id_args`: `open -b com.bambulab.bambu-studio <file>`.
pub fn macos_by_bundle(file: &str) -> Vec<String> {
    ["open", "-b", "com.bambulab.bambu-studio", file]
        .map(str::to_owned)
        .into()
}

/// `linux_path_exec_args`: `bambu-studio <file>`.
pub fn linux_on_path(file: &str) -> Vec<String> {
    ["bambu-studio", file].map(str::to_owned).into()
}

/// `linux_flatpak_args`: `flatpak run com.bambulab.BambuStudio <file>`.
pub fn linux_flatpak(file: &str) -> Vec<String> {
    ["flatpak", "run", "com.bambulab.BambuStudio", file]
        .map(str::to_owned)
        .into()
}

/// Whether `program` is in one of the folders of a `PATH`, as
/// `boost::process::search_path` finds it.
pub fn on_path(path: &str, program: &str, exists: &dyn Fn(&Path) -> bool) -> bool {
    std::env::split_paths(path).any(|dir| !dir.as_os_str().is_empty() && exists(&dir.join(program)))
}

/// `win_quote_argument`: CommandLineToArgvW's quoting.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn win_quote(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '"']) {
        return arg.to_owned();
    }
    let mut out = String::from('"');
    let mut slashes = 0;
    for c in arg.chars() {
        match c {
            '\\' => slashes += 1,
            '"' => {
                out.extend(std::iter::repeat_n('\\', slashes * 2 + 1));
                out.push('"');
                slashes = 0;
            }
            _ => {
                out.extend(std::iter::repeat_n('\\', slashes));
                slashes = 0;
                out.push(c);
            }
        }
    }
    out.extend(std::iter::repeat_n('\\', slashes * 2));
    out.push('"');
    out
}

/// `win_build_command_line`: what the reference hands `wxExecute`; here
/// the program is given to the system apart and the file quoted alone
/// ([`win_quote`]), which reads back the same.
#[cfg_attr(not(test), allow(dead_code))]
pub fn win_command_line(exe: &str, file: &str) -> String {
    format!("{} {}", win_quote(exe), win_quote(file))
}

#[cfg_attr(not(windows), allow(dead_code))]
/// `App Paths\bambu-studio.exe` under HKLM and HKCU, each view
/// (`APP_PATHS_LOOKUPS`), as `reg query` names them.
pub const WIN_APP_PATHS: [&str; 4] = [
    r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\bambu-studio.exe /reg:64",
    r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\bambu-studio.exe /reg:32",
    r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\bambu-studio.exe /reg:64",
    r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\bambu-studio.exe /reg:32",
];
/// `DEFAULT_INSTALL_PATH`.
pub const WIN_DEFAULT: &str = r"C:\Program Files\Bambu Studio\bambu-studio.exe";

/// An uninstall entry as read (`UninstallEntry`).
#[derive(Clone, Debug, Default)]
pub struct UninstallEntry {
    pub display_name: String,
    pub install_location: String,
    pub display_icon: String,
}

/// `looks_like_bambu_studio_display_name`: not "Bambu" alone, which
/// Bambu Handy and Bambu Connect share.
pub fn is_bambu_studio_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.contains("bambu studio") || lower.contains("bambustudio")
}

fn strip_icon_index(icon: &str) -> &str {
    match icon.rsplit_once(',') {
        Some((path, index)) if !index.is_empty() && index.bytes().all(|b| b.is_ascii_digit()) => {
            path
        }
        _ => icon,
    }
}

/// `discover_windows`: the first that is there of the App Paths values,
/// the uninstall entries that are Bambu Studio's (the install folder
/// before the icon), the program `.3mf` opens with when that is Bambu
/// Studio's, and the default place.
pub fn discover_windows(
    app_paths: &[String],
    uninstall: &[UninstallEntry],
    associated: &str,
    exists: &dyn Fn(&str) -> bool,
) -> Option<String> {
    let mut candidates: Vec<String> = app_paths.iter().take(4).cloned().collect();
    for entry in uninstall
        .iter()
        .filter(|entry| is_bambu_studio_name(&entry.display_name))
    {
        if !entry.install_location.is_empty() {
            let dir = &entry.install_location;
            let sep = if dir.ends_with(['\\', '/']) { "" } else { "\\" };
            candidates.push(format!("{dir}{sep}bambu-studio.exe"));
        }
        let icon = strip_icon_index(&entry.display_icon);
        if ends_with_ignoring_case(icon, ".exe") {
            candidates.push(icon.to_owned());
        }
    }
    let lower = associated.to_lowercase();
    if lower.contains("bambu-studio.exe") || lower.contains("bambustudio.exe") {
        candidates.push(associated.to_owned());
    }
    candidates.push(WIN_DEFAULT.to_owned());
    candidates
        .into_iter()
        .find(|path| !path.is_empty() && exists(path))
}

/// The ways Bambu Studio is started on this system, in order, for a file
/// (each a program and its arguments). On Windows the program found.
pub fn bambu_studio_tries(file: &str) -> Vec<Vec<String>> {
    if cfg!(target_os = "macos") {
        vec![macos_by_name(file), macos_by_bundle(file)]
    } else if cfg!(windows) {
        vec![vec!["bambu-studio.exe".into(), file.into()]]
    } else {
        vec![linux_on_path(file), linux_flatpak(file)]
    }
}

/// How a hand-off went (`LaunchOutcome`).
#[cfg_attr(test, allow(dead_code))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Launched,
    NotFound,
    /// Found there but it would not start; what was found.
    Failed(String),
}

// ---------------------------------------------------------------------
// The one place another program is started: the window's, never a test's.
// ---------------------------------------------------------------------

/// What the window is asked to hand off.
#[cfg_attr(test, allow(dead_code))]
#[derive(Clone, Debug)]
pub enum HandOff {
    /// A `bambu-connect://` URL ([`is_connect_url`]).
    Connect(String),
    /// Bambu Connect's own page ([`CONNECT_INFO_URL`] alone), in the
    /// system's browser.
    Page(String),
    /// A project file (`.3mf`) Bambu Studio is to open.
    BambuStudio(PathBuf),
}

/// Whether Bambu Connect is installed here (`detect_installed`).
#[cfg(not(test))]
pub fn connect_installed() -> Installed {
    use std::process::Command;
    if cfg!(target_os = "macos") {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let plists: Vec<Option<Vec<u8>>> = connect_bundles_macos(home.as_deref())
            .iter()
            .map(|bundle| std::fs::read(bundle.join("Contents/Info.plist")).ok())
            .collect();
        connect_on_macos(&plists)
    } else if cfg!(windows) {
        let query = |args: &[&str]| {
            Command::new("reg")
                .arg("query")
                .args(args)
                .output()
                .ok()
                .filter(|out| out.status.success())
                .map(|out| String::from_utf8_lossy(&out.stdout).into_owned())
        };
        let key = query(&[r"HKCR\bambu-connect", "/v", "URL Protocol"]);
        let base = query(&[r"HKCR\bambu-connect"]);
        let command = query(&[r"HKCR\bambu-connect\shell\open\command", "/ve"])
            .and_then(|text| reg_value(&text))
            .unwrap_or_default();
        let reg = WindowsRegistration {
            key_exists: base.is_some(),
            has_url_protocol: key.is_some(),
            open_command: command,
        };
        connect_on_windows(&reg, &|path| Path::new(path).exists())
    } else {
        let said = Command::new("xdg-mime")
            .args(["query", "default", "x-scheme-handler/bambu-connect"])
            .output()
            .ok()
            .filter(|out| out.status.success())
            .map(|out| String::from_utf8_lossy(&out.stdout).into_owned());
        connect_on_linux(said.as_deref())
    }
}

#[cfg(test)]
thread_local! {
    /// What a test says Bambu Connect is.
    pub static CONNECT_IN_TEST: std::cell::Cell<Installed> =
        const { std::cell::Cell::new(Installed::Yes) };
}

/// In a test: what the test said, nothing looked at.
#[cfg(test)]
pub fn connect_installed() -> Installed {
    CONNECT_IN_TEST.with(std::cell::Cell::get)
}

/// The value of a `reg query` answer's one line (`REG_SZ` or
/// `REG_EXPAND_SZ`).
pub fn reg_value(text: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let line = line.trim();
        ["REG_EXPAND_SZ", "REG_SZ"].iter().find_map(|kind| {
            let (_, value) = line.split_once(kind)?;
            Some(value.trim().to_owned()).filter(|value| !value.is_empty())
        })
    })
}

/// Hands off what the person asked for. The only function that starts
/// another program; called from the window alone (main.rs), never from
/// a browser's page or a test (it is not built into the tests).
#[cfg(not(test))]
pub fn perform(what: &HandOff) -> Outcome {
    use std::process::{Command, Stdio};
    let quiet = |command: &mut Command| {
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
    };
    // Waits for a short-lived opener (`open`, `xdg-open`): whether it found
    // what it was asked to open.
    let run = |argv: &[String]| -> bool {
        let mut command = Command::new(&argv[0]);
        command.args(&argv[1..]);
        quiet(&mut command);
        command.status().is_ok_and(|status| status.success())
    };
    // Starts a program and leaves it running.
    let start = |argv: &[String]| -> bool {
        let mut command = Command::new(&argv[0]);
        command.args(&argv[1..]);
        quiet(&mut command);
        command.spawn().is_ok()
    };
    match what {
        HandOff::Connect(url) | HandOff::Page(url) => {
            let allowed = match what {
                HandOff::Page(_) => url == CONNECT_INFO_URL,
                _ => is_connect_url(url),
            };
            if !allowed {
                return Outcome::Failed(url.clone());
            }
            let argv: Vec<String> = if cfg!(target_os = "macos") {
                vec!["open".into(), url.clone()]
            } else if cfg!(windows) {
                vec![
                    "rundll32".into(),
                    "url.dll,FileProtocolHandler".into(),
                    url.clone(),
                ]
            } else {
                vec!["xdg-open".into(), url.clone()]
            };
            if run(&argv) {
                Outcome::Launched
            } else {
                Outcome::Failed(CONNECT_SCHEME.into())
            }
        }
        HandOff::BambuStudio(file) => {
            let project = file
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("3mf"));
            if !project || !file.is_file() {
                return Outcome::Failed(file.to_string_lossy().into_owned());
            }
            let file = file.to_string_lossy().into_owned();
            if cfg!(target_os = "macos") {
                // **Differs**: the reference counts `open -a` as started
                // once `open` itself starts, so its bundle-id fallback is
                // never reached when the name is not registered
                // (BambuStudioLauncher.cpp l.375-389); `open` answers at
                // once whether it found the application, so its answer is
                // waited for here and the fallback taken.
                for argv in [macos_by_name(&file), macos_by_bundle(&file)] {
                    if run(&argv) {
                        return Outcome::Launched;
                    }
                }
                Outcome::NotFound
            } else if cfg!(windows) {
                let read = |key: &str| {
                    let mut parts = key.rsplitn(2, ' ');
                    let view = parts.next().unwrap_or_default();
                    let key = parts.next().unwrap_or_default();
                    Command::new("reg")
                        .args(["query", key, "/ve", view])
                        .output()
                        .ok()
                        .filter(|out| out.status.success())
                        .and_then(|out| reg_value(&String::from_utf8_lossy(&out.stdout)))
                        .unwrap_or_default()
                };
                let app_paths: Vec<String> = WIN_APP_PATHS.iter().map(|key| read(key)).collect();
                // Not read here (**not yet**): the Uninstall keys and the
                // program `.3mf` opens with (AssocQueryString); App Paths
                // and the default place are.
                let found = discover_windows(&app_paths, &[], "", &|path| Path::new(path).exists());
                match found {
                    None => Outcome::NotFound,
                    Some(exe) => {
                        let mut command = Command::new(&exe);
                        #[cfg(windows)]
                        {
                            use std::os::windows::process::CommandExt;
                            command.raw_arg(win_quote(&file));
                        }
                        #[cfg(not(windows))]
                        command.arg(&file);
                        quiet(&mut command);
                        if command.spawn().is_ok() {
                            Outcome::Launched
                        } else {
                            Outcome::Failed(exe)
                        }
                    }
                }
            } else {
                let path = std::env::var("PATH").unwrap_or_default();
                let exists = |path: &Path| path.is_file();
                if on_path(&path, "bambu-studio", &exists) {
                    return if start(&linux_on_path(&file)) {
                        Outcome::Launched
                    } else {
                        Outcome::Failed("bambu-studio".into())
                    };
                }
                if on_path(&path, "flatpak", &exists) {
                    // As the reference: a Flatpak not installed shows as a
                    // failure to start, `flatpak run` being the launch.
                    return if start(&linux_flatpak(&file)) {
                        Outcome::Launched
                    } else {
                        Outcome::Failed("flatpak run com.bambulab.BambuStudio".into())
                    };
                }
                Outcome::NotFound
            }
        }
    }
}

/// In a test nothing is ever started.
#[cfg(test)]
pub fn perform(_: &HandOff) -> Outcome {
    Outcome::Failed("nothing is started from a test".into())
}

#[cfg(test)]
mod tests;
