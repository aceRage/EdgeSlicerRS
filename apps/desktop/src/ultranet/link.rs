//! How the app reaches the bridge: its standard input and output
//! (ULTRANET_BRIDGE.md sections 1 and 2). Where the bridge is looked for,
//! how it is started and stopped; and, for the tests, a pair of pipes in
//! memory that the mock bridge (`mock.rs`) sits at the other end of, so no
//! test starts a process.

use std::{
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
};

/// The bridge's file name.
#[cfg(windows)]
pub const FILE: &str = "ultranet-bridge.exe";
#[cfg(not(windows))]
pub const FILE: &str = "ultranet-bridge";

/// One running bridge, as the client sees it: what it reads, what it
/// writes, and how to stop it.
pub struct Link {
    pub reader: Box<dyn BufRead + Send>,
    pub writer: Box<dyn Write + Send>,
    /// Ends the bridge: closes its input, waits a little, then kills it.
    pub stop: Box<dyn FnMut() + Send>,
}

/// Where the bridge is, first found wins (ULTRANET_BRIDGE.md section 2):
/// `EDGE_ULTRANET_BRIDGE`; `<data dir>/plugins/`; beside the app's
/// executable, in `ultranet/` (and a macOS bundle's `Resources/ultranet/`).
/// `data_dir` is this app's own data folder; the C++ EdgeSlicer's is never
/// looked in.
pub fn find(data_dir: Option<&Path>) -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("EDGE_ULTRANET_BRIDGE").filter(|path| !path.is_empty()) {
        let path = PathBuf::from(path);
        return runnable(&path).then_some(path);
    }
    let mut places = Vec::new();
    if let Some(dir) = data_dir {
        places.push(dir.join("plugins").join(FILE));
    }
    if let Some(exe_dir) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
    {
        places.push(exe_dir.join("ultranet").join(FILE));
        places.push(
            exe_dir
                .join("..")
                .join("Resources")
                .join("ultranet")
                .join(FILE),
        );
    }
    places.into_iter().find(|path| runnable(path))
}

/// A regular file, and on Unix one that may be run.
fn runnable(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    true
}

/// Starts the bridge: `<path> --stdio`, its standard error thrown away
/// unread (the plug-in may write anything there), in its own folder.
pub fn spawn(path: &Path) -> Result<Link, String> {
    use std::process::{Command, Stdio};
    let mut command = Command::new(path);
    command
        .arg("--stdio")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if let Some(dir) = path.parent().filter(|dir| !dir.as_os_str().is_empty()) {
        command.current_dir(dir);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // No console window of its own (CREATE_NO_WINDOW).
        command.creation_flags(0x0800_0000);
    }
    let mut child = command.spawn().map_err(|error| {
        format!(
            "The UltraNet bridge could not be started ({}).",
            error.kind()
        )
    })?;
    let stdin = child
        .stdin
        .take()
        .ok_or("The UltraNet bridge has no input.")?;
    let stdout = child
        .stdout
        .take()
        .ok_or("The UltraNet bridge has no output.")?;
    let input = std::sync::Arc::new(std::sync::Mutex::new(Some(stdin)));
    let writer = ChildInput(input.clone());
    let stop = move || {
        // Closing its input is "bye" (section 2); then 5 seconds, then
        // killed.
        input
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .take();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            if let Ok(Some(_)) = child.try_wait() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let _ = child.kill();
        let _ = child.wait();
    };
    Ok(Link {
        reader: Box::new(BufReader::new(stdout)),
        writer: Box::new(writer),
        stop: Box::new(stop),
    })
}

/// The bridge's input, which `stop` can close while a writer holds it.
struct ChildInput(std::sync::Arc<std::sync::Mutex<Option<std::process::ChildStdin>>>);

impl Write for ChildInput {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        match self
            .0
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .as_mut()
        {
            Some(input) => input.write(bytes),
            None => Err(std::io::ErrorKind::BrokenPipe.into()),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self
            .0
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .as_mut()
        {
            Some(input) => input.flush(),
            None => Err(std::io::ErrorKind::BrokenPipe.into()),
        }
    }
}

#[cfg(test)]
/// One end of a pipe in memory: what is written to the `Pipe` is read
/// from the `PipeReader`. Closing (dropping) the writer is end of input.
pub struct Pipe(std::sync::mpsc::Sender<Vec<u8>>);

#[cfg(test)]
pub struct PipeReader {
    from: std::sync::mpsc::Receiver<Vec<u8>>,
    buffer: Vec<u8>,
    at: usize,
}

/// A pipe in memory, for the tests' bridge.
#[cfg(test)]
pub fn pipe() -> (Pipe, PipeReader) {
    let (to, from) = std::sync::mpsc::channel();
    (
        Pipe(to),
        PipeReader {
            from,
            buffer: Vec::new(),
            at: 0,
        },
    )
}

#[cfg(test)]
impl Write for Pipe {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0
            .send(bytes.to_vec())
            .map_err(|_| std::io::Error::from(std::io::ErrorKind::BrokenPipe))?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
impl std::io::Read for PipeReader {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        if self.at >= self.buffer.len() {
            match self.from.recv() {
                Ok(bytes) => {
                    self.buffer = bytes;
                    self.at = 0;
                }
                Err(_) => return Ok(0),
            }
        }
        let n = out.len().min(self.buffer.len() - self.at);
        out[..n].copy_from_slice(&self.buffer[self.at..self.at + n]);
        self.at += n;
        Ok(n)
    }
}
