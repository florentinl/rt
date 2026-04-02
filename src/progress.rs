use std::{
    collections::HashMap,
    fmt::Display,
    io::{self, IsTerminal, Write},
    sync::{Arc, Mutex},
    time::Instant,
};

use rayon::{ThreadPoolBuilder, iter::IntoParallelIterator, iter::ParallelIterator};

use crate::display::{DisplayManager, StepStatus, strip_ansi};

/// Identifier for a task/step displayed to the user.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct StepId(String);

impl StepId {
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Print a consistent error summary for task failures.
///
/// Returns `true` if any errors were reported.
pub fn summarize_errors<E: Display>(errors: &[(String, E)], phase: &str) -> bool {
    if errors.is_empty() {
        return false;
    }

    let count = errors.len();
    let plural = if count == 1 { "" } else { "s" };
    eprintln!("error summary: {count} failure{plural} during {phase}");
    for (idx, (label, err)) in errors.iter().enumerate() {
        let num = idx + 1;
        eprintln!("  {num}. {label}: {err}");
    }
    true
}

/// Outcome reported by a task.
#[derive(Clone, Copy, Debug)]
pub enum StepOutcome {
    Done,
    Cached,
}

/// Context passed to tasks so they can emit output through the configured sink.
#[derive(Clone)]
pub struct StepContext {
    pub sink: Arc<dyn ProgressLogger>,
    pub step_id: StepId,
}

/// Indicates how a logger wants command output to be delivered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputPolicy {
    /// Pipe child stdout/stderr directly to the parent terminal (no capture).
    Inherit,
    /// Capture output so it can be multiplexed or formatted.
    Capture,
}

/// Sink abstraction for progress reporting.
pub trait ProgressLogger: Send + Sync {
    fn register_step(&self, id: &StepId, label: &str);
    fn start(&self, id: &StepId);
    fn finish(&self, id: &StepId, status: StepStatus);
    fn append_output(&self, id: &StepId, line: String);
    fn append_output_chunk(&self, id: &StepId, chunk: &[u8]) {
        if chunk.is_empty() {
            return;
        }
        let text = String::from_utf8_lossy(chunk).into_owned();
        self.append_output(id, text);
    }
    fn flush_output(&self, _id: &StepId) {}
    fn output_policy(&self) -> OutputPolicy {
        OutputPolicy::Capture
    }
}

/// Guard that ensures a step ends with a terminal status.
pub struct StepGuard {
    sink: Arc<dyn ProgressLogger>,
    id: StepId,
    finished: bool,
}

impl StepGuard {
    pub fn new(sink: Arc<dyn ProgressLogger>, id: StepId) -> Self {
        Self {
            sink,
            id,
            finished: false,
        }
    }

    pub fn done(mut self) {
        self.finish_with(StepStatus::Done);
    }

    pub fn cached(mut self) {
        self.finish_with(StepStatus::Cached);
    }

    pub fn fail(mut self) {
        self.finish_with(StepStatus::Failed);
    }

    fn finish_with(&mut self, status: StepStatus) {
        if self.finished {
            return;
        }
        self.sink.finish(&self.id, status);
        self.finished = true;
    }
}

impl Drop for StepGuard {
    fn drop(&mut self) {
        if !self.finished {
            self.sink.finish(&self.id, StepStatus::Failed);
        }
    }
}

/// Progress sink backed by the interactive `DisplayManager`.
pub struct MultiplexedProgressLogger {
    display: Arc<DisplayManager>,
    partial_lines: Mutex<HashMap<StepId, String>>,
}

impl MultiplexedProgressLogger {
    /// Create a logger that renders tasks through the interactive display.
    ///
    /// # Errors
    ///
    /// Returns an error if the terminal display cannot be initialized.
    pub fn new() -> std::io::Result<Self> {
        Ok(Self {
            display: Arc::new(DisplayManager::new()?),
            partial_lines: Mutex::new(HashMap::new()),
        })
    }
}

impl ProgressLogger for MultiplexedProgressLogger {
    fn register_step(&self, id: &StepId, label: &str) {
        self.display.register_step(id.as_str(), label);
    }

    fn start(&self, id: &StepId) {
        self.display
            .update_step_status(id.as_str(), StepStatus::Running);
    }

    fn finish(&self, id: &StepId, status: StepStatus) {
        self.display.update_step_status(id.as_str(), status);
    }

    fn append_output(&self, id: &StepId, line: String) {
        self.display.append_output(id.as_str(), line);
    }

    fn append_output_chunk(&self, id: &StepId, chunk: &[u8]) {
        if chunk.is_empty() {
            return;
        }

        let text = String::from_utf8_lossy(chunk);
        let completed_lines = {
            let mut buffers = self.partial_lines.lock().unwrap();
            let mut completed_lines = Vec::new();

            let remove_buffer = {
                let buffer = buffers.entry(id.clone()).or_default();
                buffer.push_str(&text);

                let mut start = 0;
                while let Some(rel_idx) = buffer[start..].find('\n') {
                    let end = start + rel_idx;
                    completed_lines.push(buffer[start..end].to_string());
                    start = end + 1;
                }

                if start > 0 {
                    buffer.drain(..start);
                }

                buffer.is_empty()
            };

            if remove_buffer {
                buffers.remove(id);
            }

            completed_lines
        };

        self.display.append_outputs(id.as_str(), completed_lines);
    }

    fn flush_output(&self, id: &StepId) {
        let pending_line = {
            let mut buffers = self.partial_lines.lock().unwrap();
            buffers.remove(id)
        };

        if let Some(buffer) = pending_line
            && !buffer.is_empty()
        {
            self.display.append_output(id.as_str(), buffer);
        }
    }
}

/// Progress sink for plain, non-interactive output.
///
/// When stderr is a TTY, subprocess output is inherited directly (preserving
/// colors). When stderr is not a TTY (e.g. VS Code extension output, CI logs),
/// output is captured and ANSI escape codes are stripped.
pub struct PlainProgressLogger {
    is_tty: bool,
    steps: Mutex<HashMap<StepId, PlainStepState>>,
}

struct PlainStepState {
    label: String,
    started_at: Option<Instant>,
}

impl Default for PlainProgressLogger {
    fn default() -> Self {
        Self {
            is_tty: io::stderr().is_terminal(),
            steps: Mutex::new(HashMap::new()),
        }
    }
}

/// Format a duration as a human-readable string (e.g. "1.2s", "3m 04s").
fn format_duration(d: std::time::Duration) -> String {
    let secs = d.as_secs_f64();
    if secs < 60.0 {
        format!("{secs:.1}s")
    } else {
        let mins = d.as_secs() / 60;
        let remaining = d.as_secs() % 60;
        format!("{mins}m {remaining:02}s")
    }
}

impl ProgressLogger for PlainProgressLogger {
    fn register_step(&self, id: &StepId, label: &str) {
        self.steps.lock().unwrap().insert(
            id.clone(),
            PlainStepState {
                label: label.to_string(),
                started_at: None,
            },
        );
    }

    fn start(&self, id: &StepId) {
        let mut steps = self.steps.lock().unwrap();
        if let Some(state) = steps.get_mut(id) {
            state.started_at = Some(Instant::now());
            eprintln!("==> {}", state.label);
        }
    }

    fn finish(&self, id: &StepId, status: StepStatus) {
        let steps = self.steps.lock().unwrap();
        if let Some(state) = steps.get(id) {
            let elapsed = state
                .started_at
                .map(|t| format_duration(t.elapsed()));
            let status_str = match status {
                StepStatus::Done => "done",
                StepStatus::Cached => "cached",
                StepStatus::Failed => "FAILED",
                StepStatus::Running | StepStatus::Pending => return,
            };
            match elapsed {
                Some(dur) => eprintln!("    [{status_str}] {} ({dur})", state.label),
                None => eprintln!("    [{status_str}] {}", state.label),
            }
        }
    }

    fn append_output(&self, _id: &StepId, line: String) {
        if self.is_tty {
            eprintln!("{line}");
        } else {
            eprintln!("    {}", strip_ansi(&line));
        }
    }

    fn append_output_chunk(&self, id: &StepId, chunk: &[u8]) {
        if chunk.is_empty() {
            return;
        }
        if self.is_tty {
            let mut stderr = io::stderr();
            let _ = stderr.write_all(chunk);
            let _ = stderr.flush();
        } else {
            let text = String::from_utf8_lossy(chunk);
            let clean = strip_ansi(&text);
            for line in clean.lines() {
                self.append_output(id, line.to_string());
            }
        }
    }

    fn output_policy(&self) -> OutputPolicy {
        if self.is_tty {
            OutputPolicy::Inherit
        } else {
            OutputPolicy::Capture
        }
    }
}

/// A unit of work to be executed by the `TaskRunner`.
pub struct Task<'a, E> {
    pub id: StepId,
    pub label: String,
    pub exec: Box<dyn FnOnce(StepContext) -> Result<StepOutcome, E> + Send + 'a>,
}

impl<'a, E> Task<'a, E> {
    pub fn new<F>(id: StepId, label: impl Into<String>, exec: F) -> Self
    where
        F: FnOnce(StepContext) -> Result<StepOutcome, E> + Send + 'a,
    {
        Self {
            id,
            label: label.into(),
            exec: Box::new(exec),
        }
    }
}

/// Executes a batch of tasks, optionally in parallel, while reporting progress to the configured sink.
pub struct TaskRunner {
    sink: Arc<dyn ProgressLogger>,
    parallelism: Option<usize>,
}

impl TaskRunner {
    #[must_use]
    pub fn new(sink: Arc<dyn ProgressLogger>) -> Self {
        Self {
            sink,
            parallelism: None,
        }
    }

    #[must_use]
    pub const fn with_parallelism(mut self, parallelism: Option<usize>) -> Self {
        self.parallelism = parallelism;
        self
    }

    /// Run all provided tasks and collect failures.
    ///
    /// # Errors
    ///
    /// Returns an error if the Rayon thread pool cannot be constructed.
    pub fn run<'a, E>(
        &self,
        tasks: Vec<Task<'a, E>>,
    ) -> Result<Vec<(String, E)>, rayon::ThreadPoolBuildError>
    where
        E: Send + 'a,
    {
        // Ensure steps are visible before work begins.
        for task in &tasks {
            self.sink.register_step(&task.id, &task.label);
        }

        let sink = Arc::clone(&self.sink);

        let run_one = move |task: Task<'a, E>| -> Option<(String, E)> {
            sink.start(&task.id);
            let guard = StepGuard::new(Arc::clone(&sink), task.id.clone());
            let result = (task.exec)(StepContext {
                sink: Arc::clone(&sink),
                step_id: task.id.clone(),
            });

            match result {
                Ok(StepOutcome::Done) => {
                    guard.done();
                    None
                }
                Ok(StepOutcome::Cached) => {
                    guard.cached();
                    None
                }
                Err(err) => {
                    guard.fail();
                    Some((task.label, err))
                }
            }
        };

        match self.parallelism {
            Some(threads) => {
                let pool = ThreadPoolBuilder::new().num_threads(threads).build()?;
                Ok(pool.install(|| {
                    tasks
                        .into_par_iter()
                        .filter_map(run_one)
                        .collect::<Vec<_>>()
                }))
            }
            None => Ok(tasks.into_iter().filter_map(run_one).collect::<Vec<_>>()),
        }
    }
}
