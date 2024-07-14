//! Miscellaneous helpers for running commands

use std::{
    borrow::Cow,
    fmt,
    fmt::Display,
    io::{self, Read, Write},
    path::Path,
    process::{Child, ChildStderr, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

/// Represents the types of errors that may occur while using cc-rs.
#[derive(Clone, Debug)]
enum ErrorKind {
    /// Error occurred while performing I/O.
    IOError,
    /// Error occurred while using external tools (ie: invocation of compiler).
    ToolExecError,
    /// Error occurred due to missing external tools.
    ToolNotFound,
}

/// Represents an internal error that occurred, with an explanation.
#[derive(Clone, Debug)]
pub struct Error {
    /// Describes the kind of error that occurred.
    kind: ErrorKind,
    /// More explanation of error that occurred.
    message: Cow<'static, str>,
}

impl Error {
    fn new(kind: ErrorKind, message: impl Into<Cow<'static, str>>) -> Error {
        Error {
            kind,
            message: message.into(),
        }
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Error {
        Error::new(ErrorKind::IOError, format!("{}", e))
    }
}

impl Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.kind, self.message)
    }
}

impl std::error::Error for Error {}

#[derive(Clone, Debug)]
pub(crate) struct CargoOutput {
    pub(crate) metadata: bool,
    pub(crate) warnings: bool,
    pub(crate) debug: bool,
    pub(crate) output: OutputKind,
    checked_dbg_var: Arc<AtomicBool>,
}

/// Different strategies for handling compiler output (to stdout)
#[derive(Clone, Debug)]
pub(crate) enum OutputKind {
    /// Forward the output to this process' stdout (Stdio::inherit)
    Forward,
}

impl CargoOutput {
    pub(crate) fn new() -> Self {
        #[allow(clippy::disallowed_methods)]
        Self {
            metadata: true,
            warnings: true,
            output: OutputKind::Forward,
            debug: std::env::var_os("CC_ENABLE_DEBUG_OUTPUT").is_some(),
            checked_dbg_var: Arc::new(AtomicBool::new(false)),
        }
    }

    pub(crate) fn print_debug(&self, arg: &dyn Display) {
        if self.metadata && !self.checked_dbg_var.load(Ordering::Relaxed) {
            self.checked_dbg_var.store(true, Ordering::Relaxed);
            println!("cargo:rerun-if-env-changed=CC_ENABLE_DEBUG_OUTPUT");
        }
        if self.debug {
            println!("{}", arg);
        }
    }

    fn stdio_for_warnings(&self) -> Stdio {
        if self.warnings {
            Stdio::piped()
        } else {
            Stdio::null()
        }
    }

    fn stdio_for_output(&self) -> Stdio {
        match self.output {
            OutputKind::Forward => Stdio::inherit(),
        }
    }
}

pub(crate) struct StderrForwarder {
    inner: Option<(ChildStderr, Vec<u8>)>,
}

const MIN_BUFFER_CAPACITY: usize = 100;

impl StderrForwarder {
    pub(crate) fn new(child: &mut Child) -> Self {
        Self {
            inner: child
                .stderr
                .take()
                .map(|stderr| (stderr, Vec::with_capacity(MIN_BUFFER_CAPACITY))),
        }
    }

    fn forward_available(&mut self) -> bool {
        if let Some((stderr, buffer)) = self.inner.as_mut() {
            loop {
                let old_data_end = buffer.len();

                // For non-blocking we check to see if there is data available, so we should try to
                // read at least that much. For blocking, always read at least the minimum amount.
                let to_reserve = MIN_BUFFER_CAPACITY;
                buffer.reserve(to_reserve);

                // Safety: stderr.read only writes to the spare part of the buffer, it never reads from it
                match stderr
                    .read(unsafe { &mut *(buffer.spare_capacity_mut() as *mut _ as *mut [u8]) })
                {
                    Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                        // No data currently, yield back.
                        break false;
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::Interrupted => {
                        // Interrupted, try again.
                        continue;
                    }
                    Ok(bytes_read) if bytes_read != 0 => {
                        // Safety: bytes_read bytes is written to spare part of the buffer
                        unsafe { buffer.set_len(old_data_end + bytes_read) };
                        let mut consumed = 0;
                        for line in buffer.split_inclusive(|&b| b == b'\n') {
                            // Only forward complete lines, leave the rest in the buffer.
                            if let Some((b'\n', line)) = line.split_last() {
                                consumed += line.len() + 1;
                                write_warning(line);
                            }
                        }
                        buffer.drain(..consumed);
                    }
                    res => {
                        // End of stream: flush remaining data and bail.
                        if old_data_end > 0 {
                            write_warning(&buffer[..old_data_end]);
                        }
                        if let Err(err) = res {
                            write_warning(
                                format!("Failed to read from child stderr: {err}").as_bytes(),
                            );
                        }
                        self.inner.take();
                        break true;
                    }
                }
            }
        } else {
            true
        }
    }

    fn forward_all(&mut self) {
        let forward_result = self.forward_available();
        assert!(forward_result, "Should have consumed all data");
    }
}

fn write_warning(line: &[u8]) {
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    stdout.write_all(b"cargo:warning=").unwrap();
    stdout.write_all(line).unwrap();
    stdout.write_all(b"\n").unwrap();
}

fn wait_on_child(
    cmd: &Command,
    program: &Path,
    child: &mut Child,
    cargo_output: &CargoOutput,
) -> Result<(), Error> {
    StderrForwarder::new(child).forward_all();

    let status = match child.wait() {
        Ok(s) => s,
        Err(e) => {
            return Err(Error::new(
                ErrorKind::ToolExecError,
                format!(
                    "Failed to wait on spawned child process, command {:?} with args {}: {}.",
                    cmd,
                    program.display(),
                    e
                ),
            ));
        }
    };

    cargo_output.print_debug(&status);

    if status.success() {
        Ok(())
    } else {
        Err(Error::new(
            ErrorKind::ToolExecError,
            format!(
                "Command {:?} with args {} did not execute successfully (status code {}).",
                cmd,
                program.display(),
                status
            ),
        ))
    }
}

pub(crate) fn run(
    cmd: &mut Command,
    program: impl AsRef<Path>,
    cargo_output: &CargoOutput,
) -> Result<(), Error> {
    let program = program.as_ref();

    let mut child = spawn(cmd, program, cargo_output)?;
    wait_on_child(cmd, program, &mut child, cargo_output)
}

pub(crate) fn spawn(
    cmd: &mut Command,
    program: &Path,
    cargo_output: &CargoOutput,
) -> Result<Child, Error> {
    struct ResetStderr<'cmd>(&'cmd mut Command);

    impl Drop for ResetStderr<'_> {
        fn drop(&mut self) {
            // Reset stderr to default to release pipe_writer so that print thread will
            // not block forever.
            self.0.stderr(Stdio::inherit());
        }
    }

    cargo_output.print_debug(&format_args!("running: {:?}", cmd));

    let cmd = ResetStderr(cmd);
    let child = cmd
        .0
        .stderr(cargo_output.stdio_for_warnings())
        .stdout(cargo_output.stdio_for_output())
        .spawn();
    match child {
        Ok(child) => Ok(child),
        Err(ref e) if e.kind() == io::ErrorKind::NotFound => {
            let extra = if cfg!(windows) {
                " (see https://docs.rs/cc/latest/cc/#compile-time-requirements \
for help)"
            } else {
                ""
            };
            Err(Error::new(
                ErrorKind::ToolNotFound,
                format!(
                    "Failed to find tool. Is `{}` installed?{}",
                    program.display(),
                    extra
                ),
            ))
        }
        Err(e) => Err(Error::new(
            ErrorKind::ToolExecError,
            format!(
                "Command {:?} with args {} failed to start: {:?}",
                cmd.0,
                program.display(),
                e
            ),
        )),
    }
}

