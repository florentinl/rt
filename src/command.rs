use std::{
    ffi::OsStr,
    io::{self, BufReader, Read},
    path::Path,
    process::{Command, ExitStatus, Stdio},
    sync::Arc,
    thread,
};

use crate::progress::{OutputPolicy, ProgressLogger, StepId};

/// A wrapper around `std::process::Command` that captures output and streams it to a
/// progress sink.
pub struct ManagedCommand {
    command: Command,
    step_id: StepId,
    sink: Arc<dyn ProgressLogger>,
}

impl ManagedCommand {
    #[must_use]
    pub(crate) fn new_uv(subcommand: &str, sink: Arc<dyn ProgressLogger>, step_id: StepId) -> Self {
        let mut command = Command::new(
            std::env::var_os("_RT_UV_BIN").unwrap_or_else(|| std::ffi::OsString::from("uv")),
        );
        command
            .arg(subcommand)
            .arg("--no-config")
            .arg("--color=always")
            .env("UV_PYTHON_PREFERENCE", "only-managed")
            .env("FORCE_COLOR", "1");
        command.stdout(Stdio::piped());
        command.stderr(Stdio::piped());

        Self {
            command,
            step_id,
            sink,
        }
    }

    /// Add arguments to the command.
    #[must_use]
    pub(crate) fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.command.args(args);
        self
    }

    /// Add a single argument to the command.
    #[must_use]
    pub(crate) fn arg<S: AsRef<OsStr>>(mut self, arg: S) -> Self {
        self.command.arg(arg);
        self
    }

    /// Set an environment variable for the command.
    #[must_use]
    pub(crate) fn env<K, V>(mut self, key: K, val: V) -> Self
    where
        K: AsRef<OsStr>,
        V: AsRef<OsStr>,
    {
        self.command.env(key, val);
        self
    }

    /// Set multiple environment variables for the command.
    #[must_use]
    pub(crate) fn envs<I, K, V>(mut self, vars: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<OsStr>,
        V: AsRef<OsStr>,
    {
        self.command.envs(vars);
        self
    }

    /// Set the working directory for the command.
    #[must_use]
    pub(crate) fn current_dir<P: AsRef<Path>>(mut self, dir: P) -> Self {
        self.command.current_dir(dir);
        self
    }

    /// Execute the command and wait for it to complete, streaming output to the `DisplayManager`.
    ///
    /// # Errors
    ///
    /// Returns an error if the child process cannot be spawned or waited on.
    pub(crate) fn status(mut self) -> io::Result<ExitStatus> {
        if self.sink.output_policy() == OutputPolicy::Inherit {
            self.command.stdout(Stdio::inherit());
            self.command.stderr(Stdio::inherit());
            return self.command.status();
        }

        // Spawn the child process
        let mut child = self.command.spawn()?;

        // Capture stdout and stderr
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("Failed to capture stdout"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| io::Error::other("Failed to capture stderr"))?;

        // Spawn reader threads
        let stdout_handle = self.spawn_reader_thread(stdout);
        let stderr_handle = self.spawn_reader_thread(stderr);

        // Wait for the child process to complete
        let status = child.wait()?;

        // Wait for reader threads to finish
        let _ = stdout_handle.join();
        let _ = stderr_handle.join();

        Ok(status)
    }

    /// Spawn a thread to read output chunks and stream them to the progress sink.
    fn spawn_reader_thread<R: io::Read + Send + 'static>(
        &self,
        reader: R,
    ) -> thread::JoinHandle<()> {
        let step_id = self.step_id.clone();
        let sink = Arc::clone(&self.sink);

        thread::spawn(move || {
            let mut buf_reader = BufReader::new(reader);
            let mut buffer = [0u8; 4096];

            loop {
                match buf_reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(n) => sink.append_output_chunk(&step_id, &buffer[..n]),
                    Err(e) => {
                        sink.append_output(&step_id, format!("[Error reading output: {e}]"));
                        break;
                    }
                }
            }

            sink.flush_output(&step_id);
        })
    }
}
