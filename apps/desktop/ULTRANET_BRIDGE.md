# The UltraNet bridge: the contract between EdgeSlicerRS and UltraNet

EdgeSlicerRS prints to Bambu Lab printers on the local network through
UltraNet, the owner's own drop-in replacement for Bambu's `bambu_networking`
plug-in. UltraNet's interface is C++ (`std::string` by value,
`std::function` callbacks, `BBL::PrintParams`), which this Rust app cannot
call in its own process. The owner's decision (2026-11-13): UltraNet runs in
a **separate process**, a small executable (the *bridge*) that loads the
plug-in, built and kept in UltraNet's own repository; this app talks to it
over a local connection. This document is that connection. Someone who has
UltraNet's source and this file should be able to write the bridge without
reading this app.

Everything below that names the reference means the C++ EdgeSlicer at the
pin in `reference/EdgeSlicer` (156e9591fe): `src/slic3r/Utils/NetworkAgent.hpp`
(the entry points), `src/slic3r/Utils/bambu_networking.hpp` (`PrintParams`,
the callback types and the error codes), `src/slic3r/GUI/GUI_App.cpp`
l.2630-2815 (the LAN callbacks), `src/slic3r/GUI/Jobs/PrintJob.cpp` (the
send), `src/slic3r/GUI/DeviceManager.cpp` (the printer's state).

Contents: 1. Choices and why. 2. Starting, finding, stopping. 3. Framing.
4. Hello and versions. 5. Requests. 6. Events. 7. Long-running jobs,
progress, cancel. 8. `PrintParams`. 9. Errors. 10. The access code.
11. Security model. 12. C++ mapping, message by message. 13. What the
bridge implementer decides. 14. What this app does with it. 15. Proposed
for a later protocol minor. 16. Protocol 1.1: the Bambu Lab account.

## 1. Choices and why

| Choice | Taken | Why |
| --- | --- | --- |
| Process | One bridge process per running app, started by the app as its child | One agent per app is what the reference has (`GUI_App::m_agent`); a child dies with its parent and needs no discovery. |
| Transport | The child's **standard input and output** (anonymous pipes) | Nothing listens anywhere: no port (not even on 127.0.0.1, which every local process and user can reach), no socket file (which needs a rendezvous path, stale-file cleanup, and on Windows a named pipe with its own access list, which Rust's standard library does not offer and C++ needs Win32 calls for). Only the parent holds the pipe ends, so no other process can connect and no per-launch secret is needed. Both sides need only their standard library. Weighed against the two options the owner named: a Unix socket / named pipe owned by the user is reachable by every process of that user; 127.0.0.1 with a secret is reachable by every local process and makes some systems ask about the firewall. Pipes are reachable by neither. |
| Framing | Newline-delimited JSON, UTF-8, one object per line | Trivial to write and read in both languages (`nlohmann::json` in UltraNet, `serde_json` here), readable in a log, no binary framing to get wrong. |
| Shape | Requests with an `id` and one response each; events without; the plug-in's callbacks are events | The plug-in's entry points map one for one to requests; its callbacks (`OnLocalConnectedFn`, `OnMessageFn`, `OnUpdateStatusFn`, `OnWaitFn`) to events. |
| The file to print | By **path** (`PrintParams.filename`), never bytes over the pipe | The plug-in takes a path and reads the file itself; both processes are the same user on the same machine. |
| The access code | In the request that needs it (`connect_printer`, `PrintParams.password`), in memory only on both sides, never in argv, the environment, a file of the bridge's, a log or an error | Command lines and environments are visible to other processes (`ps`); logs are sent to support. |
| Versions | `hello` first; a protocol number (major) both sides check, a minor for additions; the plug-in's own version and whether the bridge can use it | The app can say "too old" before anything is sent to a printer. |

## 2. Starting, finding, stopping

**Who starts it.** This app, when the person first does something that needs
it (opens Device for a Bambu Lab printer, opens "Send print job to", adds a
printer and connects it). Never at the app's start, never in the
background. One bridge per app process; the app keeps it until it quits.

**How the app finds it**, first found wins:

1. the path in the environment variable `EDGE_ULTRANET_BRIDGE` (development
   and tests);
2. `<data dir>/plugins/ultranet-bridge` (`ultranet-bridge.exe` on
   Windows), where `<data dir>` is **this app's** data folder
   (`EDGE_DESKTOP_DATA`, else `~/Library/Application Support/EdgeSlicerRS`,
   `%APPDATA%\EdgeSlicerRS`, `$XDG_CONFIG_HOME/EdgeSlicerRS`); this app never
   looks in the C++ EdgeSlicer's data folder;
3. beside the app: `<folder of the app's executable>/ultranet/ultranet-bridge[.exe]`,
   and in a macOS bundle `Contents/Resources/ultranet/ultranet-bridge`
   (where the reference keeps its UltraNet sidecar, `GUI_App.cpp`
   l.3811-3890).

The file must be a regular file (on Unix: executable). No marker file is
needed: the bridge says what it is in `hello` (the reference needs
`ultranet.txt` to tell UltraNet from Bambu's library of the same name; a
bridge has a name of its own).

**How it is started:** `ultranet-bridge --stdio`, nothing else on the command
line, the app's environment, working directory the bridge's own folder.
Standard input and output are the protocol; standard error is the bridge's
to write to and **the app throws it away unread** (UltraNet and the plug-ins
it loads may print anything there). The bridge refuses to run without
`--stdio` (so a double-click does not leave a process waiting on a console).

**Standard output belongs to the protocol.** The plug-in or the code it
links may write to standard output (`NetworkAgent.cpp` itself does a
`printf("loading network module at %s\n")`). Before it loads anything the
bridge must move the protocol off file descriptor 1: duplicate fd 1 to a new
descriptor kept for the protocol and point fd 1 at fd 2 (`dup` and `dup2`;
on Windows `_dup`/`_dup2` and `_setmode(fd, _O_BINARY)` on both protocol
streams so no `\r` is added).

**Stopping.** The app sends `bye` and closes the bridge's standard input.
On `bye` or end of input the bridge disconnects the printer
(`disconnect_printer`), cancels running jobs (their `WasCancelledFn` answers
true), destroys the agent (`destroy_agent`) and exits, within 3 seconds; the
app kills it after 5.

**When it dies.** End of the bridge's output, or the child exiting, is a dead
bridge. Every request waiting fails ("The UltraNet bridge stopped."), every
job running ends as failed with that sentence, the printer is shown
disconnected. The next thing the person does that needs it starts it again,
at most once every 5 seconds; after 3 deaths within a minute the app stops
starting it and says so until the person presses "Retry".

**When it is absent.** No file found: the app says "Network plug-in missing"
and why, offers nothing that would need it, and starts nothing.

**When it is too old or too new** (section 4): the app sends `bye`, stops it
and says which.

## 3. Framing

- One JSON object per line, UTF-8, ended by `\n` (a lone `\n`; a `\r` before
  it is ignored on reading). A JSON encoder escapes the new lines inside
  strings, so a line is a message.
- A line is at most 4 MiB. A longer one is a protocol error: the reader drops
  the connection (the app treats the bridge as dead).
- A line that is not a JSON object is answered (if it had a readable `id`)
  with error -32700 and otherwise ignored.
- Unknown fields are ignored by both sides (the minor version adds fields).
- Both directions are written by one writer at a time (a mutex around the
  write of a whole line, which is flushed at once).

Three kinds of message, told by `type`:

```json
{"type":"request","id":7,"method":"connect_printer","params":{...}}
{"type":"response","id":7,"result":{...}}
{"type":"response","id":7,"error":{"code":-32602,"message":"dev_ip is missing"}}
{"type":"event","event":"local_message","params":{...}}
```

Requests go from the app to the bridge only. `id` is an unsigned integer,
new for each request; every request gets exactly one response, in any order.
Events go from the bridge to the app only. The bridge must handle requests
while jobs run (a job runs on a thread of its own) and must answer each
short request within 10 seconds (the app gives up on it after 15 and treats
a bridge that does that twice in a row as dead).

## 4. Hello and versions

The first request is always `hello`; anything else first is answered with
error -100.

Request:

```json
{"type":"request","id":1,"method":"hello","params":{
  "protocol": 1,
  "client": "EdgeSlicerRS",
  "client_version": "0.0.0",
  "log_dir": "/Users/me/Library/Application Support/EdgeSlicerRS/ultranet/log",
  "config_dir": "/Users/me/Library/Application Support/EdgeSlicerRS/ultranet/config"
}}
```

Response:

```json
{"type":"response","id":1,"result":{
  "protocol": 1,
  "minor": 0,
  "bridge": "ultranet-bridge",
  "bridge_version": "1.0.0",
  "plugin_loaded": true,
  "plugin_version": "02.01.01.52",
  "plugin_expected": "02.01.01.52",
  "plugin_compatible": true,
  "plugin_error": "",
  "methods": ["hello","bye","connect_printer","disconnect_printer","send_message_to_printer",
              "start_local_print","start_local_print_with_record","start_send_gcode_to_sdcard",
              "start_sdcard_print","cancel_job","callback_return"],
  "events": ["local_connect","local_message","job_status","job_wait","job_done"]
}}
```

- `protocol` is the major version. This document is protocol **1**. The app
  refuses a bridge whose `protocol` is not 1: lower is "too old", higher is
  "too new". A bridge answers a `hello` whose `protocol` it cannot speak with
  error -101 and its own `protocol` in `error.data.protocol`.
- `minor` counts additions that an older reader can ignore (a new optional
  field, a new optional method or event). The app needs minor 0.
- `methods` lists every method the bridge answers. The app needs
  `connect_printer`, `disconnect_printer`, `send_message_to_printer`,
  `start_local_print`, `start_send_gcode_to_sdcard`, `cancel_job`; a bridge
  without one of them is "too old".
- `plugin_loaded` false (the library is missing, or will not load), with
  `plugin_error` a sentence saying why (the reference's
  `PluginLoadFailure`, without paths outside the bridge's folder): the
  app says "Network plug-in missing" and that sentence, and sends nothing
  else but `bye`.
- `plugin_compatible` is the reference's rule
  (`NetworkAgent::is_compatible_version`): the first 8 characters of the
  plug-in's `bambu_network_get_version()` equal those of the version the
  bridge was built for (`BAMBU_NETWORK_AGENT_VERSION`, "02.01.01.52" at the
  pin). The bridge decides, because it is the bridge that calls the
  plug-in's C++ interface; false makes the app refuse it.

## 5. Requests

Every request's result holds the plug-in's return value as `ret` (an
`int`, 0 success, the `BAMBU_NETWORK_ERR_*` values of
`bambu_networking.hpp` otherwise) when the entry point returns one. A plug-in
error is a result with a non-zero `ret`, **not** an `error`; `error` is for
the protocol itself (section 9).

| Method | Params | Result | Plug-in entry point | Used by this app |
| --- | --- | --- | --- | --- |
| `hello` | section 4 | section 4 | the start-up below | yes |
| `bye` | `{}` | `{}`, then the bridge exits | `disconnect_printer`, `destroy_agent` | yes |
| `connect_printer` | `dev_id` (serial, upper case), `dev_ip`, `username` ("bblp"), `password` (the access code), `use_ssl` (bool) | `{ret}`; the outcome comes as `local_connect` | `connect_printer` | yes |
| `disconnect_printer` | `{}` | `{ret}` | `disconnect_printer` | yes |
| `send_message_to_printer` | `dev_id`, `json` (a **string**: the command's JSON text, exactly as published), `qos` (0 or 1), `flag` (`MessageFlag`, 0) | `{ret}` | `send_message_to_printer` | yes (`pushall`, `get_version`, pause, resume, stop) |
| `start_local_print` | `params` (section 8) | `{job}` at once; `job_status` events; `job_done` | `start_local_print(params, update, cancel)` | yes |
| `start_local_print_with_record` | `params` | `{job}` | `start_local_print_with_record(params, update, cancel, wait)` | no (the reference uses it for cloud-bound printers only) |
| `start_send_gcode_to_sdcard` | `params`, `wait` (bool: pass an `OnWaitFn` or `nullptr`) | `{job}` | `start_send_gcode_to_sdcard(params, update, cancel, wait)` | yes (the access-code check before a print, `PrintJob.cpp` l.215-232) |
| `start_sdcard_print` | `params` | `{job}` | `start_sdcard_print(params, update, cancel)` | not yet (printing a file already on the printer) |
| `cancel_job` | `job` | `{}` | sets the flag the job's `WasCancelledFn` returns | yes |
| `callback_return` | `cb`, `value` (bool) | `{}` | the return value of the `OnWaitFn` call `cb` names | yes |
| `watch_printers` (optional) | `targets`: `[{dev_id, dev_ip, username, password}]` | `{ret}` | `bambu_network_ultranet_watch_printers` (UltraNet only) | no |
| `bind_detect` (optional) | `dev_ip`, `sec_link` ("secure") | `{ret, detect: {result_msg, command, dev_id, model_id, dev_name, version, bind_state, connect_type}}` | `bind_detect` | no: printers are added by hand (IP, access code, serial, model), as the reference does on macOS |

What is **not** in the protocol, on purpose: `start_discovery` and
`set_on_ssdp_msg_fn` (this app looks for no printer on the network; the
bridge must not start discovery on its own either), Bambu's cloud beyond
the account of section 16 (`connect_server`, the cloud MQTT session,
`start_print` through the cloud, `bind`, presets, model mall, tracking),
the camera. A bridge may answer these with -32601. The plug-in's own
sign-in entry points (`change_user`, `user_logout`, ...) are not methods
either: section 16's `account_*` methods stand for them.

The plug-in holds **one** LAN session at a time (`connect_printer` takes no
handle back; `disconnect_printer` takes no `dev_id`). So does the protocol:
connecting another printer ends the session to the first, as in the
reference.

## 6. Events

| Event | Params | Plug-in callback |
| --- | --- | --- |
| `local_connect` | `status` (0 ok, 1 failed, 2 lost: `ConnectStatus`), `dev_id`, `msg` (the plug-in's reason: "5" or "rc=5" for a refused access code) | `OnLocalConnectedFn(int status, std::string dev_id, std::string msg)` set by `set_on_local_connect_fn` |
| `local_message` | `dev_id`, `msg` (a **string**: the printer's MQTT report as received, unparsed) | `OnMessageFn(std::string dev_id, std::string msg)` set by `set_on_local_message_fn` |
| `job_status` | `job`, `stage` (`SendingPrintJobStage`), `code` (0..100 progress within the stage, else an error code), `info` (the plug-in's text) | the job's `OnUpdateStatusFn(int status, int code, std::string msg)` |
| `job_wait` | `job`, `cb` (unsigned, new for each call), `status`, `job_info` (string) | the job's `OnWaitFn(int status, std::string job_info)`: the app answers with `callback_return` |
| `job_done` | `job`, `ret` (the entry point's return value) | the entry point returned |

`msg` of `local_message` is passed as the plug-in gives it, so the app parses
the printer's JSON with the reference's rules (`MachineObject::parse_json`)
and the bridge needs no knowledge of the printer's schema.

The bridge must not send `local_message` for a printer other than the one
connected (or watched with `watch_printers`). Messages may arrive at any
rate; the app reads them as they come.

## 7. Long-running jobs, progress, cancel

The four `start_*` methods answer at once with a `job` number (unsigned, new
for each job, never reused while the bridge runs), run the plug-in's call on
a thread of their own, and report:

- every `OnUpdateStatusFn` call as `job_status`, in order;
- every `OnWaitFn` call as `job_wait`; the plug-in's thread then waits for
  the app's `callback_return` with that `cb`. No answer in 60 seconds:
  `true` (the reference's `wait_fn` answers `true` on its own time-out,
  `PrintJob.cpp` l.501-509). End of input: `true`.
- the return value as `job_done` (always the last message about the job).

`cancel_job` sets a flag; the job's `WasCancelledFn` returns it. The plug-in
looks at it between pieces of its work and returns
`BAMBU_NETWORK_ERR_CANCELED` (-18), which comes as `job_done` like any
other result. A cancel for a job that has ended is answered `{}` and
ignored.

The reference's progress (`PrintJob.cpp` l.360-440): stage
`PrintingStageCreate` 20 %, `Upload` 30 % (plus `code` % of the 40 % up to
70 % while `code` is 0..100), `Waiting` 70 %, `Record` 75 %, `Sending` 97 %,
`Finished` 100 %. The app shows the same.

Several jobs may run at once in the protocol; the app starts one at a time.

## 8. `PrintParams`

`params` is `BBL::PrintParams` (`bambu_networking.hpp` l.235) field for
field, by the same names, with JSON types: `std::string` is a string, `int`
an integer, `bool` a boolean. A field left out is the struct's default (an
empty string, 0, false; `extruder_cali_manual_mode` 0).

| Field | Type | What this app sends for a LAN print (`PrintJob.cpp` l.205-260) |
| --- | --- | --- |
| `dev_id` | string | the printer's serial |
| `task_name` | string | "" |
| `project_name` | string | the project's name, at most 100 characters |
| `preset_name` | string | `<project>_plate_<n>` |
| `filename` | string | **absolute path** of the plate's sliced file (`.gcode.3mf`), written by this app for this job, readable by the user |
| `config_filename` | string | "" (only the cloud uses it) |
| `plate_index` | int | the plate, from 1 |
| `ftp_folder` | string | the model's `ftp_folder` (`resources/printers/<model>.json`: "sdcard/" for the P1, A1 series; else "") |
| `ftp_file`, `ftp_file_md5` | string | "" |
| `ams_mapping` | string | the v0 tray list, a JSON array as text (`"[0,-1,2]"`) |
| `ams_mapping2` | string | the v1 list, JSON text (`[{"ams_id":0,"slot_id":0},...]`) |
| `ams_mapping_info` | string | per filament: `[{"ams":0,"targetColor":"FF0000FF","filamentId":"GFA00","filamentType":"PLA","sourceColor":"#FF0000"},...]` |
| `nozzles_info` | string | "[]" (a printer with one nozzle) |
| `connection_type` | string | "lan" |
| `comments` | string | "" |
| `origin_profile_id`, `stl_design_id` | int | 0 |
| `origin_model_id` | string | "" |
| `print_type` | string | "from_normal" |
| `dst_file` | string | "" (a file already on the printer: its path there) |
| `dev_name` | string | the printer's name |
| `dev_ip` | string | the printer's address |
| `use_ssl_for_ftp`, `use_ssl_for_mqtt` | bool | true |
| `username` | string | "bblp" |
| `password` | string | **the access code** (section 10) |
| `task_bed_leveling` | bool | "Bed Leveling" |
| `task_flow_cali` | bool | "Flow Dynamics Calibration" |
| `task_vibration_cali` | bool | false |
| `task_layer_inspect` | bool | true |
| `task_record_timelapse` | bool | "Timelapse" (true when the option is not offered) |
| `task_use_ams` | bool | "Enable AMS" on a printer with an AMS, else false |
| `task_bed_type` | string | the plate's bed as the G-code names it (`cool_plate`, `eng_plate`, `hot_plate`, `textured_plate`, `textured_cool_plate`, `supertack_plate`) |
| `extra_options` | string | "" |
| `auto_bed_leveling`, `auto_flow_cali`, `auto_offset_cali` | int | 0 |
| `nozzle_mapping_request` | string | "" |
| `extruder_cali_manual_mode` | int | 0 |

The access-code check (`start_send_gcode_to_sdcard`) sends the same fields
but `project_name` "verify_job" and `filename` a small text file the app
writes ("just a test file", as the reference's
`resources/check_access_code.txt`).

## 9. Errors

`error` is `{code, message, data?}`; `message` is one sentence a person can
read, in English, **never** holding an access code.

| Code | Meaning |
| --- | --- |
| -32700 | the line is not JSON |
| -32600 | not a request (`type`, `id`, `method` wrong or missing) |
| -32601 | no such method |
| -32602 | a parameter is missing or of the wrong type (`message` names it, never its value) |
| -100 | `hello` has not been answered yet |
| -101 | `hello` asked for a protocol the bridge cannot speak (`data.protocol` is the bridge's) |
| -102 | the plug-in is not loaded |
| -103 | the bridge is stopping (after `bye`) |

## 10. The access code

- It travels in `connect_printer.password` and `PrintParams.password` only,
  and only on the pipe.
- The bridge keeps it in memory for the call, and for as long as the
  plug-in's session needs it (re-dialling); it writes it nowhere: not to a
  log, not to standard error, not to a file, not in an error message, not in
  an event. Where the plug-in writes its own log, the bridge must make sure
  it is not there either (the reference's `NetworkAgent::connect_printer`
  logs it on failure, `NetworkAgent.cpp` l.1071; the bridge must not copy
  that).
- This app keeps it in the operating system's store (`src/secrets.rs`): the
  macOS login keychain, Windows Credential Manager, the Secret Service on
  Linux, through the `keyring-core` crate. Every printer's code is in ONE
  item (service `EdgeSlicerRS`, account `Bambu Lab access codes`, JSON
  `{ "version": 1, "codes": { "<serial>": "<code>" } }`), so one answer to
  the system covers every printer. The list of printers
  (`bambu_printers.json`) says whether each has a code
  (`access_code_kept`), never the code.
- The store is read only when a code is needed: "Connect", a send (the
  "Send print job to" dialog connects), a code added, changed or removed,
  or a printer with a code removed; never when the app starts or the
  Device tab opens (the page dials a printer by itself only when the code
  is already at hand, `bambu_printers` `secrets.at_hand`). What it gave is
  kept for the run: at most one read of the store, and so at most one
  macOS prompt to read, a run. It is written only when a code is added,
  changed or removed.
- Where the system's store is not there or refuses (no Secret Service on
  Linux, a locked keychain, access denied at the prompt, `--serve` with no
  session to ask in), the code is kept for the rest of the run in a file of
  the data folder readable by this user only (mode 0600;
  `bambu_printers_secrets.json`), and the Device panel says so in plain
  words (`bambu_printers` `secrets.notice`). There is no setting for it
  (the reference has none: it keeps the code in `app_config`, in plain
  text).
- That file is also where versions before the system's store kept the
  codes. The first time the store is used, the file's codes are written to
  it, read back, and the file is removed only when the store gave back the
  same codes; otherwise the file is left as it is, used for the run, and
  that is said. Removing a printer removes its code from the store.
- `EDGE_SECRETS` chooses another store: `file`, `memory` (for the run) or
  `unavailable` (a system store that refuses all, to see the fallback).
  The tests use the file, memory, or the system store's code on
  `keyring-core`'s mock store; a test build cannot reach the real one.
- The code is never in a project, never in an answer to the page, never in
  a log. The app removes it from anything the bridge says before showing
  it (in case a bridge or a plug-in echoes it).
- On macOS the keychain asks the person ("EdgeSlicerRS wants to use your
  confidential information stored in "EdgeSlicerRS" in your keychain")
  when a program not on the item's access list reads or changes it; the
  program that made the item is on the list. A development build is
  signed ad hoc and so is a new program to the keychain after every
  rebuild: it is asked again even after "Always Allow". A release build
  keeps "Always Allow" across updates when every version is signed with
  the same Developer ID (the item's access list names the program by its
  designated requirement: the bundle identifier and the team), and the
  bundle identifier stays the same.

## 11. Security model

- Nothing listens: the pipes are the child's, and only the app holds them.
  No port, no socket file, no shared memory.
- The bridge runs as the same user as the app, with the app's environment;
  it is trusted as much as the app is (it is code the person installed in
  their own data folder or beside the app). The app does not verify a
  signature; a later version may (13).
- The bridge talks to the printer the app names in `connect_printer` and
  `PrintParams.dev_ip`, and to nothing else: no discovery, no cloud but
  the account's REST calls of section 16 (made only for an `account_*`
  request: no cloud MQTT, no printing through the cloud).
- Everything that makes a printer act (`start_*`, the `print` commands of
  `send_message_to_printer`) is sent only after the person confirmed it in
  the app, with the reference's own confirmations.
- The file to print is a path the app wrote under the system's temporary
  folder (`EdgeSlicerRS/ultranet/`), removed when the job ends.
- A printer in its default (secured) mode refuses every unsigned `print`
  command ("mqtt message verify failed"); UltraNet does not sign, the app
  does not ask it to. The app says "This printer is not in Developer Mode, so
  it rejects commands from EdgeSlicer. Turn on LAN Only + Developer Mode on
  the printer." (`BambuSendDiagnosis.cpp` l.131) before sending.

## 12. C++ mapping, message by message

Pseudo-code for the bridge, using the reference's own `NetworkAgent`
loader (the bridge may link UltraNet directly instead; the calls are the
same). `out(...)` writes one line under the writer mutex.

```cpp
// main.cpp of ultranet-bridge
int main(int argc, char** argv) {
    if (argc != 2 || std::string(argv[1]) != "--stdio") { fprintf(stderr, "run by EdgeSlicerRS only\n"); return 2; }
    int proto_out = dup(1); dup2(2, 1);                    // fd 1 now goes to stderr
#ifdef _WIN32
    _setmode(_fileno(stdin), _O_BINARY); _setmode(proto_out, _O_BINARY);
#endif
    Writer w(proto_out);                                   // mutex + write(line + "\n") + flush
    for (std::string line; std::getline(std::cin, line); ) {
        if (line.size() > 4 << 20) break;
        json m = json::parse(line, nullptr, false);
        if (m.is_discarded() || !m.is_object()) { w.error(m, -32700, "not JSON"); continue; }
        dispatch(w, m);                                    // below; start_* hand off to threads
    }
    shutdown();                                            // end of input == bye
}

// hello
void hello(Writer& w, uint64_t id, const json& p) {
    if (p.value("protocol", 0) != 1) return w.error(id, -101, "protocol 1 only", {{"protocol", 1}});
    int rc = load_plugin();                                // dlopen("libbambu_networking.dylib" beside the bridge), as
                                                           // NetworkAgent::initialize_network_module does
    std::string ver = rc == 0 ? get_version_ptr() : "";    // bambu_network_get_version
    bool ok = rc == 0 && ver.size() >= 8 && ver.substr(0, 8) == std::string(BAMBU_NETWORK_AGENT_VERSION).substr(0, 8);
    if (ok) {
        agent = create_agent_ptr(p["log_dir"]);            // bambu_network_create_agent(log_dir)
        set_config_dir_ptr(agent, p["config_dir"]);
        init_log_ptr(agent);
        set_cert_file_ptr(agent, bridge_dir() + "/cert", "slicer_base64.cer");   // if UltraNet needs it
        set_on_local_connect_fn_ptr(agent, [&](int status, std::string dev_id, std::string msg) {
            w.event("local_connect", {{"status", status}, {"dev_id", dev_id}, {"msg", msg}}); });
        set_on_local_message_fn_ptr(agent, [&](std::string dev_id, std::string msg) {
            w.event("local_message", {{"dev_id", dev_id}, {"msg", msg}}); });
        set_queue_on_main_fn_ptr(agent, [](std::function<void()> f) { main_queue.post(std::move(f)); });
        start_ptr(agent);                                  // no country code, no sign-in: LAN only
    }
    w.result(id, {{"protocol", 1}, {"minor", 0}, {"bridge", "ultranet-bridge"}, {"bridge_version", BRIDGE_VERSION},
                  {"plugin_loaded", rc == 0}, {"plugin_version", ver}, {"plugin_expected", BAMBU_NETWORK_AGENT_VERSION},
                  {"plugin_compatible", ok}, {"plugin_error", rc == 0 ? "" : load_failure_sentence()},
                  {"methods", METHODS}, {"events", EVENTS}});
}

// connect_printer -> int bambu_network_connect_printer(void*, std::string dev_id, std::string dev_ip,
//                                                      std::string username, std::string password, bool use_ssl)
w.result(id, {{"ret", connect_printer_ptr(agent, p["dev_id"], p["dev_ip"], p["username"], p["password"], p["use_ssl"])}});

// disconnect_printer -> int bambu_network_disconnect_printer(void*)
w.result(id, {{"ret", disconnect_printer_ptr(agent)}});

// send_message_to_printer -> int bambu_network_send_message_to_printer(void*, std::string dev_id, std::string json_str, int qos, int flag)
w.result(id, {{"ret", send_message_to_printer_ptr(agent, p["dev_id"], p["json"], p["qos"], p.value("flag", 0))}});

// start_local_print / start_local_print_with_record / start_send_gcode_to_sdcard / start_sdcard_print
void start(Writer& w, uint64_t id, const json& p, Which which) {
    BBL::PrintParams pp = from_json(p["params"]);          // section 8, field by field; missing = default
    uint64_t job = ++last_job;
    auto cancelled = std::make_shared<std::atomic<bool>>(false);
    jobs[job] = cancelled;
    w.result(id, {{"job", job}});
    std::thread([=, &w] {
        OnUpdateStatusFn update = [&w, job](int stage, int code, std::string info) {
            w.event("job_status", {{"job", job}, {"stage", stage}, {"code", code}, {"info", scrub(info, pp.password)}}); };
        WasCancelledFn cancel = [cancelled] { return cancelled->load(); };
        OnWaitFn wait = [&w, job](int status, std::string job_info) -> bool {
            uint64_t cb = ++last_cb;
            auto answer = callbacks.expect(cb);            // a promise<bool>
            w.event("job_wait", {{"job", job}, {"cb", cb}, {"status", status}, {"job_info", job_info}});
            return answer.wait_for(60s) == ready ? answer.get() : true; };
        int ret;
        switch (which) {
        case LocalPrint:  ret = start_local_print_ptr(agent, pp, update, cancel); break;
        case WithRecord:  ret = start_local_print_with_record_ptr(agent, pp, update, cancel, wait); break;
        case ToSdcard:    ret = start_send_gcode_to_sdcard_ptr(agent, pp, update, cancel, p.value("wait", false) ? wait : nullptr); break;
        case SdcardPrint: ret = start_sdcard_print_ptr(agent, pp, update, cancel); break;
        }
        w.event("job_done", {{"job", job}, {"ret", ret}});
        jobs.erase(job);                                   // pp (and its password) goes out of scope here
    }).detach();
}

// cancel_job
if (auto it = jobs.find(p["job"]); it != jobs.end()) it->second->store(true);
w.result(id, json::object());

// callback_return
callbacks.fulfil(p["cb"], p["value"].get<bool>());
w.result(id, json::object());

// bye: disconnect, cancel every job, destroy, exit
disconnect_printer_ptr(agent); for (auto& [_, c] : jobs) c->store(true); callbacks.fulfil_all(true);
w.result(id, json::object()); destroy_agent_ptr(agent); _exit(0);
```

`scrub(text, code)` removes every occurrence of the access code from a text
before it leaves the bridge; the app does the same again.

## 13. What the bridge implementer decides

- Whether the bridge links UltraNet or loads it with `dlopen` /
  `LoadLibrary`, and where the library is (beside the bridge is the
  natural place; never `<C++ EdgeSlicer data dir>/plugins`, which is the C++
  app's).
- Its own version scheme (`bridge_version`), and whether to ship it signed.
- Whether `set_cert_file` is needed for LAN TLS, and which file.
- How the plug-in's own log is kept free of the access code.
- Whether to offer the optional `watch_printers` and `bind_detect` (the app
  works without them).
- On Windows: the bridge has no console window of its own (it is a console
  program, started by the app with `CREATE_NO_WINDOW`; the app does that).
- How the bridge reports an exception thrown by the plug-in inside a job:
  it must still send `job_done` (with `ret` -1, as `BAMBU_NETWORK_ERR_INVALID_HANDLE`)
  and keep running.

## 14. What this app does with it

The print cycle for a printer in LAN mode, as `SelectMachine.cpp`,
`PrintJob.cpp` and `DeviceManager.cpp` drive it (UI_PARITY.md section 4, "A
Bambu Lab printer", lists each step):

1. `hello`; refuse what section 4 says to refuse.
2. `connect_printer(serial, ip, "bblp", code, true)`; wait for
   `local_connect` (20 s). Status 1 with `msg` "5" (or "rc=5", "not
   authorized"): "Incorrect password", the printer marked as refusing its
   code ("The printer refused the access code. Re-enter it").
3. On status 0: `send_message_to_printer` with
   `{"pushing":{"sequence_id":"<n>","command":"pushall","version":1,"push_target":1}}`
   (qos 0) and `{"info":{"sequence_id":"<n>","command":"get_version"}}`
   (qos 1), as `GUI_App.cpp` l.2681 does; then every `local_message` is the
   printer's state.
4. "Send print job to": the checks of `SelectMachineDialog::update_show_status`
   and `on_ok_btn` on that state; then the sliced file written,
   `start_send_gcode_to_sdcard` with the "verify_job" file (unless the
   printer has no card and prints from internal storage), then
   `start_local_print`; after `job_done` 0, the printer's reports watched
   for 25 s for a new `job_id` or a printing state (`PrintJob.cpp`
   `lan_started_fn`), else "The file was uploaded but the printer did not
   start the job. ...".
5. Pause, resume, stop: `send_message_to_printer` with
   `{"print":{"command":"pause"|"resume"|"stop","param":"","sequence_id":"<n>"}}`,
   qos 1 (`MachineObject::command_task_*`); stop only after "Are you sure you
   want to cancel this print?".
6. The Device page (device-pages pass), all through the
   `send_message_to_printer` this protocol already has, each only after the
   person confirmed it: `gcode_line` for `M104 S` / `M140 S` (targets),
   `M106 P<1|2|3> S<0..255>` (fans), the jog block of
   `command_axis_control`, `G28` (home), `M83` / `G0 E` (extruder);
   `print_speed` (levels 1-4); `set_ctt` (chamber, only when the printer
   reports `support_chamber_temp_edit`); `system.ledctrl` for
   `chamber_light`. Read from the reports already forwarded: the fans
   (`cooling_fan_speed`, `big_fan1_speed`, `big_fan2_speed`, `fan_gear`),
   `spd_lvl`, `lights_report`, `home_flag`, `stg_cur`, `hms`, and the
   `get_version` answer's `module` list ("Update"'s versions). No new
   message was needed for these.

## 15. Proposed for a later protocol minor (not part of protocol 1.0)

**Proposed, not in force.** A bridge of protocol 1.0 need not answer any of
this; each is an addition an older reader can ignore (section 4: `minor`),
offered in `methods` / `events` when a bridge has it. Minor 1 went to the
account (section 16), so a bridge that offers these answers `hello` with
the minor after it, 2 (this section first named minor 1 for them; the app
decides by `methods`, never by the minor's number, so nothing it does
changed). Written down so the Bambu Device page's
remaining parts (UI_PARITY.md section 4, "One Device page for every printer
family") have a contract to be built against, and **the app side is built
against it**: each part of the page is drawn only when the bridge lists its
methods, and otherwise says that it needs a bridge of this later minor and
which method the running one lacks. Each keeps section 11's model: nothing
listens, the access code stays on the pipe, no cloud, no discovery.

| Proposed | Params / result | Plug-in / reference | For |
| --- | --- | --- | --- |
| `camera_start` (method) | `dev_id`; result `{ret, kind}` with `kind` "jpeg" (P1, A1: the BambuSource tunnel on port 6000) or "rtsps" (X1, H2, P2S: `rtsps://bblp:<code>@<ip>:322/streaming/live/1`), the stream opened by the bridge with the code it already holds | `MediaPlayCtrl.cpp` l.340-354; the `Bambu_*` tunnel functions of BambuSource (not in the plug-in's `bambu_network_*` entry points: the bridge would load BambuSource too, or UltraNet would offer its own) | "Camera" |
| `camera_frame` (event) | `dev_id`, `seq` (increasing), `jpeg` (base64 of one JPEG frame, at most 4 MiB before encoding; for "rtsps" the bridge decodes and re-encodes, at most 2 frames a second); only for a printer whose camera the app started | | the page shows the latest frame in an `<img>` of the app's own origin (a `data:` URL), never the printer's address. The app keeps only the latest frame of each printer, and only one that is base64 of a JPEG (`/9j/...`) |
| `camera_stop` (method) | `dev_id`; result `{ret}` | | Status not looked at any more |
| `files_list` (method) | `dev_id`, `type` ("timelapse", "video", "model"), `storage` ("external", "internal"); result `{ret, files: [{name, path, time, size}]}` (`path` as the printer names it, `time` seconds since 1970) | `PrinterFileSystem` `LIST_INFO` (0x0001) over the same tunnel (`MediaFilePanel.cpp`) | "Storage" |
| `files_delete` (method) | `dev_id`, `paths`; result `{ret, refused?}` | `FILE_DEL` (0x0003) | "Storage" "Delete", after the reference's "Do you want to delete the file '%s' from printer?"; the app also refuses the file a print is on |
| `files_download` (method, a job) | `dev_id`, `path`, `to` (a path under the system's temporary folder the app names); `job_status` with the percent, `job_done` | `FILE_DOWNLOAD` (0x0004) | "Storage" "Download"; the model's "Print" (`start_sdcard_print` is in protocol 1 already). **Not used by the app yet** |
| `firmware_upgrade` (method) | `dev_id`, `module` ("ota"), `version` (the new version the person was shown and confirmed); result `{ret, refused?}` (`refused` a sentence when the bridge's own check fails, `ret` non-zero) | `MachineObject::command_upgrade_confirm` (DeviceManager.cpp l.2095): `{"upgrade":{"command":"upgrade_confirm","sequence_id":"<n>","src_id":1}}`, qos 1, the push update of `UpgradePanel.cpp` (`confirm_upgrade`) | "Update" "Update firmware" |

`ret` is 0 for success, as in section 5; `refused`, when present, is one
sentence a person can read, never holding the access code.

**The firmware update, in detail.** It is the riskiest command of the
page, so it has a method of its own instead of a `send_message_to_printer`
the bridge would pass on blindly: the app never sends an `upgrade` command
through `send_message_to_printer` (a 1.0 bridge would carry it), and a
bridge that does not list `firmware_upgrade` gets none. No cloud is
involved: the printer itself reports whether it found a newer firmware, in
its `push_status` report's `upgrade_state` (`dis_state` 1 "available",
`new_version_state` 1, `new_ver_list` `[{name, cur_ver, new_ver}]`,
`ota_new_version_number`; DeviceManager.cpp l.4580-4670), which the bridge
already forwards as `local_message`; the printer fetches the firmware
itself. Before it publishes `upgrade_confirm`, the bridge checks the
printer's last report again: `dis_state` 1, the offered `ota` version equal
to `version`, and `gcode_state` not `RUNNING`, `PAUSE`, `PREPARE` or
`SLICING`; otherwise `ret` -1 with `refused`. The app checks the same
before it asks the person ("Are you sure you want to update? This will take
about 10 minutes. Do not turn off the power while the printer is
updating.", with each module's version and the new one), refuses while a
job is on the printer, and never sends it on its own. Progress comes back in
the reports (`dis_state` 2, `status`, `progress`; 3 when finished).

Still not proposed, on purpose: the cloud's firmware list and its download
(`get_printer_firmware`, `upgrade` `start` with a URL): Bambu's cloud, which
this protocol leaves out (section 5). The HMS texts come from the
reference's own `resources/hms` files; their daily refresh from Bambu's
cloud is not proposed either.

**Changes to this section since it was first proposed** (the device-pages
pass): `firmware_upgrade` added (the update was "not proposed" before:
the printer's own `upgrade_state` needs no cloud); `minor` 1 named as the
minor that offers all of it; `camera_frame`'s `seq`, size limit and
"only for a printer whose camera was started" stated; `camera_stop`,
`files_delete` and `firmware_upgrade` answer `{ret}` (and `refused`);
`files_list`'s `path` and `time` defined. Protocol 1.0 is unchanged.

## 16. Protocol 1.1: the Bambu Lab account

**In force.** The owner's decision (2026-11-26): the app signs in to a Bambu
Lab account, shows who is signed in and the account's printers, and prints
to those printers **over the LAN** with the access code the account gives.
Printing through Bambu's cloud stays out (section 5). Everything here is an
addition an older reader ignores: a bridge that offers it answers `hello`
with `minor` 1 and lists the methods and the event below. The app needs
minor 0 still; it offers "Log in to Bambu Account..." only when the running
bridge lists `account_sign_in`, `account_restore`, `account_sign_out` and
`account_printers`, and says otherwise that the bridge is too old for it.

The reference this follows (at the pin, c9aec301bd):
`src/slic3r/GUI/WebUserLoginDialog.cpp` (the sign-in page and what it hands
back), `src/slic3r/Utils/LoginUserAgent.cpp` (its User-Agent),
`src/slic3r/GUI/GUI_App.cpp` (sign-out, an expired session),
`src/slic3r/GUI/MainFrame.cpp` l.3297-3307 (the Account menu),
`src/slic3r/GUI/DeviceManager.cpp` (the account's printers); and UltraNet's
`UltraNetAgent.cpp` (`changeUser`, `restoreSession`, `cloudRefresh`,
`userLogout`) and `BambuCloud.hpp` (the REST calls).

### 16.1 The sign-in page, and what the bridge is given

The page is Bambu's own, `<bambulab host>/sign-in` (the bare path, not
localed: WebUserLoginDialog.cpp l.84-90; the host is
`bambu_network_get_bambulab_host`, `https://bambulab.com` in UltraNet,
UltraNetAgent.cpp `bambulabHost`). The **app** shows it, in a window of its
own that loads nothing else (section 16.6); the bridge never sees the page.
The page runs its slicer sign-in only for a User-Agent that ends in
`BBL-Slicer/v02.08.02.61 (<dark|light>) BBL-Language/<en>`, after a browser
prefix (LoginUserAgent.cpp `bbl_login_user_agent`; the version is the one
bambulab.com gates on, `BBL_LOGIN_UA_VERSION`, not the slicer's own). It
hands its result to the slicer as a script message, `window.wx.postMessage`
of a JSON text (WebUserLoginDialog.cpp l.273-424):

| `command` | `data` | The reference | Here |
| --- | --- | --- | --- |
| `user_ticket_login` | `ticket`: a short-lived ticket | l.303-370: `get_my_token(ticket)`, then `get_my_profile(accessToken)`, then `change_user` with the tokens and the profile | `account_sign_in {ticket}` |
| `user_login` | `token`, `refresh_token`, `expires_in`, `refresh_expires_in`, `user {uid, name, account, avatar}` | l.285-302: `change_user(<the message>)` | `account_restore` with those tokens |
| `get_localhost_url`, `thirdparty_login`, `new_webpage` | | l.371-419: Google / Apple sign-in through the system's browser and a listener on `localhost:13650` | **not built**: it needs a listening port (section 11: nothing listens) and another application; the window says the sign-in with a password is the one offered |
| `autotest_token` | | l.281-284 | ignored |

### 16.2 Methods

| Method | Params | Result | UltraNet / reference |
| --- | --- | --- | --- |
| `account_sign_in` | `ticket` | `{ret, http, user, session}` | `bambu_network_get_my_token`, `bambu_network_get_my_profile` (`uidStr`, `name`), `bambu_network_change_user` with the message WebUserLoginDialog.cpp l.344-354 builds |
| `account_restore` | `session`, `user` | `{ret, http, user, session?}` | the rule of `Agent::restoreSession` (UltraNetAgent.cpp l.1400-1431), the session given by the app instead of read from a store; `change_user` |
| `account_refresh` | `{}` | `{ret, http, session?}` | `cloud_refresh_session` (BambuCloud.hpp; `Agent::cloudRefresh`, UltraNetAgent.cpp l.1434-1470), then `change_user` |
| `account_sign_out` | `{}` | `{}` | `bambu_network_user_logout` (GUI_App.cpp `request_user_logout` l.6445) |
| `account_profile` | `{}` | `{signed_in, user}` | `is_user_login`, `get_user_id`, `get_user_name`; asks nothing of the cloud |
| `account_printers` | `{}` | `{ret, http, printers, session?}` | `GET /v1/iot-service/api/user/bind` (`cloud_get_user_printers`, reshaped by `cloud_reshape_printers`): what `DeviceManager::update_user_machine_list_info` and `parse_user_print_info` read (DeviceManager.cpp l.8669-8755) |

| Event | Params | When |
| --- | --- | --- |
| `account_expired` | `{}` | the bridge ended the session because the cloud refused it and its refresh (GUI_App.cpp l.6873-6880: a 401 signs out, "Login information expired. Please login again.") |

The shapes:

- `user`: `{id, name}` (`uidStr` and `name` of the profile; the reference's
  `get_user_id`, `get_user_name`). The account's e-mail (`account`) and the
  avatar are not passed: the app shows the name only, and the page loads
  nothing from the network (an avatar is a URL of Bambu's).
- `session`: `{access_token, refresh_token, expires_in, refresh_expires_in,
  acquired_at}`: the two tokens as strings, the lifetimes in seconds
  (`expiresIn`, `refreshExpiresIn` of the ticket and refresh answers), and
  when they were issued (seconds since 1970, the bridge's clock). In a
  result it is present when the call made new tokens (always for
  `account_sign_in`; for `account_restore`, `account_refresh` and
  `account_printers` when a refresh was made), and the app keeps it in
  place of the one it had.
- `printers`: `[{dev_id, dev_name, dev_online, dev_model_name, task_status,
  dev_access_code}]`, `parse_user_print_info`'s fields as
  `cloud_reshape_printers` gives them. The cloud gives no address (the
  reference takes it from its own `app_config` `ip_address`, written when it
  saw the printer on the LAN, DeviceManager.cpp l.8695-8698): the app asks
  the person for it (section 16.5).
- `ret`: 0 done; -1 the cloud refused or could not be reached (`http` is the
  HTTP status of the call that failed, 0 when no answer came); -2 the
  session is over (the access token expired or was refused, and so was its
  refresh: the bridge is signed out, `account_expired` is sent, the app
  signs out); -3 not signed in (`account_refresh`, `account_printers` before
  a sign-in or restore). A ticket or token of the wrong form (anything but
  letters, digits and `-._~+/=`, or longer than 8 KiB) is error -32602:
  UltraNet puts the ticket into a URL path and the refresh token into a JSON
  text as they are.

**`account_restore`** (the rule of `restoreSession`, its margin of 300 s):
the access token is good while `acquired_at + expires_in - 300` is after
now: `change_user` with the session as it is, no call to the cloud. Else the
refresh token is good while `acquired_at + refresh_expires_in - 300` is
after now: a refresh, then `change_user` with the new tokens, which come back
in `session`. Else, or when the refresh is refused: `ret` -2. A restore while
signed in first signs out (the plug-in keeps one user).

**`account_printers`**: a 401 from the cloud is answered as the reference's
HTTP error handler would (GUI_App.cpp l.6873): the bridge refreshes once
and asks again; a refused refresh is `ret` -2. Any other failure is `ret`
-1 with `http`, and **never** an empty list: the app keeps the printers it
had (UltraNet's own `get_user_print_info` answers 0 with an empty list on a
failure, which is why the bridge calls the REST function itself).

### 16.3 Timing

The `account_*` methods call the cloud (two calls for `account_sign_in`, up
to three for `account_printers`; UltraNet's HTTPS client waits up to 20 s
for each connect, send and receive). They are therefore not under the 10 s
rule of section 3: the bridge runs them one at a time on a thread of their
own (so LAN requests are not held behind them), never answers them with a
time-out of its own, and answers each within 150 s; the app waits 160 s. If
the app gives up on one all the same, it sends `account_sign_out` and
forgets the session it had asked about, so neither side keeps a session the
other does not know of (a late result is dropped, as any late answer is).
`account_profile` and `account_sign_out` touch no network and are short
requests.

### 16.4 The tokens and the account's access codes

They are secrets as the access code is (section 10), with one place each:

- The **ticket** travels in `account_sign_in.ticket` only. The tokens travel
  in `account_restore.session` and in the results' `session`; the access
  codes in `account_printers`' `dev_access_code`. Nowhere else: never in an
  event, an error, a log, standard error, a file of the bridge's, argv or
  the environment.
- The bridge keeps them in memory for the session (UltraNet holds the tokens
  for its calls) and writes them nowhere: its session store is
  `bridge/SessionStoreOff.cpp`, which keeps nothing, so the session lives
  only in the app's store and in the bridge's memory while it runs. The
  bridge removes every ticket, token and account code it has seen from every
  event and error it writes, as it does the access codes, and overwrites its
  copies of the parameters when a request ends. UltraNet's own log
  (`ultranet_print.log`) gets, on a failed call only, the request's shape
  with the credentials taken out (BambuCloud.hpp's `diag`).
- **The app keeps them in the system's store**, in the one item of section
  10, under a versioned field of its own:
  `{ "version": 1, "codes": {...}, "account": { "version": 1, "user": {id,
  name}, "session": {...}, "codes": { "<serial>": "<code>" } } }`. An older
  app reads `codes` and ignores `account` (it would drop it on its next
  write: the person signs in again). The account's codes are kept apart
  from the codes typed for a printer, so signing out removes exactly them.
  In the file fallback (`bambu_printers_secrets.json`) the same object is
  kept under the id `@account`. The store is read when the account is
  needed (sign-in, sign-out, "Refresh", a send to an account printer), never
  when the app starts or a page is looked at: who is signed in and the list
  of the account's printers (no codes) are kept in `bambu_printers.json` so
  Device can show them.
- The page is told who is signed in (the name), the account's printers
  without their codes, and states. Never a token, a ticket or a code. The
  app removes all three from anything the bridge says before it shows it.

### 16.5 What the app does with the account's printers

`parse_user_print_info` keeps the account's printers apart from those
added on the LAN (`userMachineList` and `localMachineList`), and a printer's
code typed by the person (`user_access_code`) wins over the one the cloud
gives (`access_code`; `MachineObject::get_access_code`, DeviceManager.cpp
l.502-507). The app does the same: Device lists the printers added by hand
and those of the account (marked as the account's; one in both is listed
once, its address and typed code its own), the account's codes kept in the
store's `account` field. A printer of the account with no address asks for
one before it can be connected (the reference reads its last seen address
from `app_config`; this app looks for none on the network). **Signing out**
(`request_user_logout`, `clean_user_info` DeviceManager.cpp l.8393-8411)
removes the account's printers and their codes; a printer also added by
hand stays, with its own address and code. The reference keeps a code the
cloud gave in `app_config`'s `access_code` after the sign-out (it clears it
only in memory); here it goes with the account.

### 16.6 The sign-in window (the app's side, for the bridge implementer's information)

A window of the app's own (`bambu-sign-in`), titled "Login", 650 x 840
(WebUserLoginDialog.cpp l.131-134), that shows only the sign-in page: it
navigates only to `https` pages of `bambulab.com` and its subdomains
(anything else is refused and not opened elsewhere), opens no other
window, has no access to the app's calls (Tauri allows none to a remote
page, and each of the app's commands refuses any window but the main one),
and hands the result back without a call: a script the window runs first
defines `window.wx.postMessage`, which turns the message into a navigation
to `https://sign-in-result.edgeslicer.invalid/#<the message>`; the window's
navigation guard stops that navigation (the `.invalid` name never resolves,
RFC 6761), reads the message there and hands it to the Rust side, which
calls the bridge. The ticket never reaches the main page. A test-only
override (`EDGE_BAMBU_SIGN_IN=http://127.0.0.1:<port>/...`, loopback only,
refused otherwise) points the window at a local page; no test opens it.
