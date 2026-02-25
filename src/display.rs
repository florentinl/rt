use crossterm::{
    cursor,
    style::{Color, Stylize},
    terminal::{self as crossterm_terminal, ClearType},
    ExecutableCommand, QueueableCommand,
};
use indexmap::IndexMap;
use std::{
    collections::VecDeque,
    convert::TryFrom,
    fmt::Write as FmtWrite,
    io::{self, stderr, Write},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

/// Strip ANSI escape codes from a string to measure visual width.
fn visual_width(text: &str) -> usize {
    let mut width = 0;
    let mut chars = text.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            // Skip ANSI escape sequence
            if chars.peek() == Some(&'[') {
                chars.next(); // consume '['
                              // Skip until we hit a letter (the command character)
                while let Some(&next_ch) = chars.peek() {
                    chars.next();
                    if next_ch.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
        } else {
            // Count visible character
            width += 1;
        }
    }

    width
}

/// Truncate a line to fit within the given width, preserving ANSI codes and adding ellipsis.
fn truncate_line(line: &str, max_width: usize) -> String {
    let visual_len = visual_width(line);

    if visual_len <= max_width {
        return line.to_string();
    }

    // Need to truncate - reserve 1 char for "…" (Unicode ellipsis)
    let target_width = max_width.saturating_sub(1);

    let mut result = String::new();
    let mut current_width = 0;
    let mut chars = line.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            // Copy ANSI escape sequence
            result.push(ch);
            if chars.peek() == Some(&'[') {
                result.push(chars.next().unwrap()); // '['
                while let Some(&next_ch) = chars.peek() {
                    result.push(chars.next().unwrap());
                    if next_ch.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
        } else {
            // Count and copy visible character
            if current_width >= target_width {
                break;
            }
            result.push(ch);
            current_width += 1;
        }
    }

    result.push('…'); // Unicode ellipsis (U+2026)
    result
}

/// Status of a build step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepStatus {
    Pending,
    Running,
    Done,
    Cached,
    Failed,
}

/// Format a status icon with appropriate color and styling.
#[must_use]
fn status_icon(status: StepStatus) -> String {
    match status {
        StepStatus::Pending => "[pending]".with(Color::DarkGrey).to_string(),
        StepStatus::Running => "[running]".with(Color::Cyan).to_string(),
        StepStatus::Done => "[done]".with(Color::Green).to_string(),
        StepStatus::Cached => "[cached]".with(Color::Yellow).to_string(),
        StepStatus::Failed => "[failed]".with(Color::Red).to_string(),
    }
}

/// Apply dim attribute to text, preserving any existing colors.
/// This wraps the text with dim mode and ensures dim persists through color resets.
#[must_use]
fn dim(text: &str) -> String {
    // Start with dim attribute
    let mut result = String::from("\x1b[2m");

    // Replace any reset codes with reset+dim to maintain dimness
    let processed = text.replace("\x1b[0m", "\x1b[0m\x1b[2m");
    result.push_str(&processed);

    // End with reset
    result.push_str("\x1b[0m");

    result
}

/// Prefix output line with a vertical bar (for nested output display).
#[must_use]
fn output_prefix() -> String {
    "  │ ".to_string()
}

const RUNNING_MIN_LINES: usize = 6;
const FAILED_BLOCK_LINES: usize = 5;
const COLLAPSED_LINE_COST: usize = 1;

#[derive(Clone, Copy)]
enum FrameMode {
    Final,
    Active {
        terminal_width: usize,
        step_area_height: usize,
        lines_per_running: usize,
    },
}

#[derive(Clone, Copy)]
enum StepRenderStyle {
    Final,
    Active {
        terminal_width: usize,
        lines_per_running: usize,
        remaining_height: usize,
    },
}

/// A single build step with its output buffer.
#[derive(Debug, Clone)]
pub struct BuildStep {
    pub description: String,
    pub status: StepStatus,
    pub output_lines: VecDeque<String>,
    pub start_time: Option<Instant>,
    pub end_time: Option<Instant>,
    pub max_output_lines: usize,
}

impl BuildStep {
    /// Create a new build step in Pending state.
    #[must_use]
    pub const fn new(description: String) -> Self {
        Self {
            description,
            status: StepStatus::Pending,
            output_lines: VecDeque::new(),
            start_time: None,
            end_time: None,
            max_output_lines: 100, // Keep last 100 lines in buffer
        }
    }

    /// Append a line of output to this step's buffer.
    pub fn append_output(&mut self, line: String) {
        if self.output_lines.len() >= self.max_output_lines {
            self.output_lines.pop_front();
        }
        self.output_lines.push_back(line);
    }

    /// Update the status and record timestamps.
    pub fn update_status(&mut self, status: StepStatus) {
        self.status = status;
        match status {
            StepStatus::Running => {
                if self.start_time.is_none() {
                    self.start_time = Some(Instant::now());
                }
            }
            StepStatus::Done | StepStatus::Cached | StepStatus::Failed => {
                if self.end_time.is_none() {
                    self.end_time = Some(Instant::now());
                }
            }
            StepStatus::Pending => {}
        }
    }

    /// Check if this step should show as fully collapsed (no expansion).
    #[must_use]
    pub const fn is_fully_collapsed(&self) -> bool {
        matches!(
            self.status,
            StepStatus::Pending | StepStatus::Done | StepStatus::Cached
        )
    }

    /// Render this step as a collapsed single line.
    #[must_use]
    pub fn render_collapsed(&self) -> String {
        let icon = status_icon(self.status);
        format!("{} {}", icon, self.description)
    }

    /// Render this step as expanded (with output lines).
    ///
    /// Parameters:
    /// - `max_output_lines`: Maximum number of output lines to show
    /// - `terminal_width`: If Some, truncate lines to fit; if None, no truncation (allows wrapping)
    /// - `apply_dim`: Whether to apply dimming to the output
    #[must_use]
    pub fn render_expanded(
        &self,
        max_output_lines: usize,
        terminal_width: Option<usize>,
        apply_dim: bool,
    ) -> Vec<String> {
        let mut lines = Vec::new();

        // Header line
        let icon = status_icon(self.status);
        lines.push(format!("{} {}", icon, self.description));

        // Output lines (show last N lines)
        let output_to_show = self
            .output_lines
            .iter()
            .rev()
            .take(max_output_lines)
            .rev()
            .collect::<Vec<_>>();

        for line in output_to_show {
            let processed_line = terminal_width.map_or_else(
                || line.clone(),
                |width| {
                    // Truncate line based on current terminal width
                    let available_width = width.saturating_sub(4);
                    truncate_line(line, available_width)
                },
            );

            let formatted = if apply_dim {
                format!("{}{}", output_prefix(), dim(&processed_line))
            } else {
                format!("{}{}", output_prefix(), processed_line)
            };
            lines.push(formatted);
        }

        lines
    }
}

/// Manages the multiplexed display of parallel build steps.
pub struct DisplayManager {
    steps: Arc<Mutex<IndexMap<String, BuildStep>>>,
    stderr: Arc<Mutex<io::Stderr>>,
    refresh_handle: Mutex<Option<JoinHandle<()>>>,
    shutdown: Arc<AtomicBool>,
    final_rendered: Arc<AtomicBool>,
    refresh_rate: Duration,
    lines_last_render: Arc<Mutex<usize>>,
    start_time: Instant,
}

impl DisplayManager {
    const GROUP_ORDER: &[(StepStatus, usize)] = &[
        (StepStatus::Failed, FAILED_BLOCK_LINES),
        (StepStatus::Pending, COLLAPSED_LINE_COST),
        (StepStatus::Done, COLLAPSED_LINE_COST),
        (StepStatus::Cached, COLLAPSED_LINE_COST),
    ];

    /// Create a new `DisplayManager`.
    ///
    /// # Errors
    ///
    /// Returns an error if the terminal cannot be initialized.
    #[must_use = "The manager must stay alive to render progress updates"]
    pub fn new() -> io::Result<Self> {
        let mut stderr = stderr();
        stderr.execute(cursor::Hide)?;
        let display = Self {
            steps: Arc::new(Mutex::new(IndexMap::new())),
            stderr: Arc::new(Mutex::new(stderr)),
            refresh_handle: Mutex::new(None),
            shutdown: Arc::new(AtomicBool::new(false)),
            final_rendered: Arc::new(AtomicBool::new(false)),
            refresh_rate: Duration::from_millis(33), // 30 FPS
            lines_last_render: Arc::new(Mutex::new(0)),
            start_time: Instant::now(),
        };
        install_panic_hook();
        display.start_refresh_loop();
        Ok(display)
    }

    /// Register a new build step.
    ///
    /// # Panics
    ///
    /// Panics if the internal mutex is poisoned.
    pub fn register_step(&self, id: &str, description: &str) {
        self.steps
            .lock()
            .unwrap()
            .insert(id.to_string(), BuildStep::new(description.to_string()));
        self.final_rendered.store(false, Ordering::Relaxed);
    }

    /// Update the status of a build step.
    ///
    /// # Panics
    ///
    /// Panics if the internal mutex is poisoned.
    pub fn update_step_status(&self, id: &str, status: StepStatus) {
        let mut steps = self.steps.lock().unwrap();
        if let Some(step) = steps.get_mut(id) {
            step.update_status(status);
            self.final_rendered.store(false, Ordering::Relaxed);
        }
    }

    /// Append a line of output to a build step.
    ///
    /// # Panics
    ///
    /// Panics if the internal mutex is poisoned.
    pub fn append_output(&self, id: &str, line: String) {
        // Store the full line - truncation will happen during rendering
        // based on current terminal width to support terminal resizing
        let mut steps = self.steps.lock().unwrap();
        if let Some(step) = steps.get_mut(id) {
            step.append_output(line);
            self.final_rendered.store(false, Ordering::Relaxed);
        }
    }

    /// Start the refresh loop in a background thread.
    ///
    /// # Panics
    ///
    /// Panics if locking the stderr or steps mutexes fails.
    pub fn start_refresh_loop(&self) {
        let steps = Arc::clone(&self.steps);
        let stderr = Arc::clone(&self.stderr);
        let shutdown = Arc::clone(&self.shutdown);
        let final_rendered = Arc::clone(&self.final_rendered);
        let lines_last_render = Arc::clone(&self.lines_last_render);
        let refresh_rate = self.refresh_rate;
        let start_time = self.start_time;

        let handle = thread::spawn(move || loop {
            let shutting_down = shutdown.load(Ordering::Relaxed);
            let should_render = shutting_down || !final_rendered.load(Ordering::Relaxed);

            if should_render {
                let steps = steps.lock().unwrap();
                let mut lines_count = lines_last_render.lock().unwrap();
                let mut stderr = stderr.lock().unwrap();
                if let Err(e) = Self::render_locked(
                    &steps,
                    &mut stderr,
                    &mut lines_count,
                    &final_rendered,
                    start_time,
                ) {
                    eprintln!("Display render error: {e}");
                }
            }

            if shutting_down {
                break;
            }

            thread::sleep(refresh_rate);
        });

        *self.refresh_handle.lock().unwrap() = Some(handle);
    }

    /// Render the current state to the terminal.
    fn render_locked(
        steps: &IndexMap<String, BuildStep>,
        stderr: &mut io::Stderr,
        lines_last_render: &mut usize,
        final_rendered: &AtomicBool,
        start_time: Instant,
    ) -> io::Result<()> {
        let (terminal_width, available_height) = Self::terminal_dimensions()?;
        let summary_line = Self::build_summary_line(steps, terminal_width, start_time);
        let step_area_height = available_height.saturating_sub(1);
        let visible_steps = Self::select_visible_steps(steps, step_area_height);
        let all_terminal = Self::all_terminal(steps);

        if steps.is_empty() {
            Self::rewind_cursor(stderr, *lines_last_render)?;
            stderr.queue(crossterm_terminal::Clear(ClearType::FromCursorDown))?;
            let mut buffer = String::new();
            buffer.push_str(&summary_line);
            buffer.push('\n');
            write!(stderr, "{buffer}")?;
            stderr.flush()?;
            *lines_last_render = 1;
            final_rendered.store(true, Ordering::Relaxed);
            return Ok(());
        }

        Self::rewind_cursor(stderr, *lines_last_render)?;
        stderr.queue(crossterm_terminal::Clear(ClearType::FromCursorDown))?;

        let mode = if all_terminal {
            FrameMode::Final
        } else {
            let lines_per_running = Self::lines_per_running(&visible_steps, step_area_height);
            FrameMode::Active {
                terminal_width,
                step_area_height,
                lines_per_running,
            }
        };

        let rendered_lines = Self::render_frame(&visible_steps, stderr, &summary_line, mode)?;
        *lines_last_render = rendered_lines;
        final_rendered.store(matches!(mode, FrameMode::Final), Ordering::Relaxed);
        Ok(())
    }

    fn terminal_dimensions() -> io::Result<(usize, usize)> {
        let (width, height) = crossterm_terminal::size()?;
        Ok((width as usize, height.saturating_sub(2) as usize))
    }

    fn rewind_cursor(stderr: &mut io::Stderr, lines_last_render: usize) -> io::Result<()> {
        if lines_last_render == 0 {
            return Ok(());
        }
        if let Ok(lines) = u16::try_from(lines_last_render) {
            stderr.queue(cursor::MoveUp(lines))?;
        }
        Ok(())
    }

    fn all_terminal(steps: &IndexMap<String, BuildStep>) -> bool {
        steps.values().all(|s| {
            matches!(
                s.status,
                StepStatus::Done | StepStatus::Cached | StepStatus::Failed | StepStatus::Pending
            )
        })
    }

    fn select_visible_steps(
        steps: &IndexMap<String, BuildStep>,
        step_area_height: usize,
    ) -> Vec<&BuildStep> {
        if step_area_height == 0 {
            return Vec::new();
        }

        let running: Vec<&BuildStep> = steps
            .values()
            .filter(|s| s.status == StepStatus::Running)
            .collect();
        let running_count = running.len();
        let mut visible: Vec<&BuildStep> = Vec::with_capacity(steps.len());
        visible.extend(running.iter().copied());

        let mut budget = step_area_height.saturating_sub(running_count * RUNNING_MIN_LINES);

        for (status, cost) in Self::GROUP_ORDER {
            if budget == 0 {
                break;
            }
            for step in steps.values().filter(|s| s.status == *status) {
                if budget == 0 {
                    break;
                }
                visible.push(step);
                budget = budget.saturating_sub(*cost);
            }
        }

        visible
    }

    fn render_frame(
        steps: &[&BuildStep],
        stderr: &mut io::Stderr,
        summary_line: &str,
        mode: FrameMode,
    ) -> io::Result<usize> {
        let mut buffer = String::new();
        buffer.push_str(summary_line);
        buffer.push('\n');
        let mut step_lines_rendered = 0usize;

        match mode {
            FrameMode::Final => {
                for step in steps {
                    let lines = Self::render_step_lines(step, StepRenderStyle::Final);
                    step_lines_rendered += lines.len();
                    for line in lines {
                        buffer.push_str(&line);
                        buffer.push('\n');
                    }
                }
            }
            FrameMode::Active {
                terminal_width,
                step_area_height,
                lines_per_running,
            } => {
                let mut remaining_height = step_area_height;
                for step in steps {
                    if remaining_height == 0 {
                        break;
                    }

                    let lines = Self::render_step_lines(
                        step,
                        StepRenderStyle::Active {
                            terminal_width,
                            lines_per_running,
                            remaining_height,
                        },
                    );

                    for line in lines {
                        if remaining_height == 0 {
                            break;
                        }
                        buffer.push_str(&line);
                        buffer.push('\n');
                        step_lines_rendered += 1;
                        remaining_height = remaining_height.saturating_sub(1);
                    }
                }
            }
        }

        let rendered_lines = step_lines_rendered + 1;

        write!(stderr, "{buffer}")?;
        stderr.flush()?;
        Ok(rendered_lines)
    }

    fn render_step_lines(step: &BuildStep, style: StepRenderStyle) -> Vec<String> {
        if step.is_fully_collapsed() {
            return vec![step.render_collapsed()];
        }

        match (step.status, style) {
            (
                StepStatus::Failed,
                StepRenderStyle::Active {
                    terminal_width,
                    remaining_height,
                    ..
                },
            ) => {
                let max_lines = remaining_height.min(FAILED_BLOCK_LINES);
                let max_output = max_lines.saturating_sub(1);
                step.render_expanded(max_output, Some(terminal_width), true)
            }
            (StepStatus::Failed, StepRenderStyle::Final) => step.render_expanded(30, None, false),
            (
                StepStatus::Running,
                StepRenderStyle::Active {
                    terminal_width,
                    lines_per_running,
                    remaining_height,
                },
            ) => {
                let max_lines = remaining_height.min(lines_per_running + 1);
                let max_output = max_lines.saturating_sub(1);
                step.render_expanded(max_output, Some(terminal_width), true)
            }
            (StepStatus::Running, StepRenderStyle::Final) | (_, _) => vec![step.render_collapsed()],
        }
    }

    fn lines_per_running(steps: &[&BuildStep], available_height: usize) -> usize {
        let failed_count = steps
            .iter()
            .filter(|s| s.status == StepStatus::Failed)
            .count();
        let running_count = steps
            .iter()
            .filter(|s| s.status == StepStatus::Running)
            .count();

        let lines_for_failed = failed_count * FAILED_BLOCK_LINES;
        let remaining_lines = available_height.saturating_sub(lines_for_failed);
        if running_count > 0 {
            (remaining_lines / running_count).max(RUNNING_MIN_LINES)
        } else {
            0
        }
    }

    fn format_duration(duration: Duration) -> String {
        let total_seconds = duration.as_secs();
        let hours = total_seconds / 3600;
        let minutes = (total_seconds % 3600) / 60;
        let seconds = total_seconds % 60;

        if hours > 0 {
            format!("{hours}:{minutes:02}:{seconds:02}")
        } else {
            format!("{minutes}:{seconds:02}")
        }
    }

    fn build_summary_line(
        steps: &IndexMap<String, BuildStep>,
        terminal_width: usize,
        start_time: Instant,
    ) -> String {
        let mut counts = (0usize, 0usize, 0usize, 0usize, 0usize);
        for step in steps.values() {
            match step.status {
                StepStatus::Pending => counts.0 += 1,
                StepStatus::Running => counts.1 += 1,
                StepStatus::Done => counts.2 += 1,
                StepStatus::Cached => counts.3 += 1,
                StepStatus::Failed => counts.4 += 1,
            }
        }

        let (pending, running, done, cached, failed) = counts;
        let total_steps = steps.len();
        let completed = done + cached + failed;
        let parts = [
            (StepStatus::Running, running),
            (StepStatus::Pending, pending),
            (StepStatus::Done, done),
            (StepStatus::Cached, cached),
            (StepStatus::Failed, failed),
        ];

        let mut line = String::from("Summary: ");
        let mut first = true;
        for (status, count) in parts {
            if !first {
                line.push(' ');
                line.push(' ');
            }
            line.push_str(&status_icon(status));
            line.push(' ');
            line.push_str(&count.to_string());
            first = false;
        }

        line.push_str("  Progress: ");
        let _ = FmtWrite::write_fmt(&mut line, format_args!("{completed}/{total_steps}"));
        line.push_str("  Elapsed: ");
        let elapsed = Instant::now().saturating_duration_since(start_time);
        line.push_str(&Self::format_duration(elapsed));

        truncate_line(&line, terminal_width)
    }
}

impl Drop for DisplayManager {
    fn drop(&mut self) {
        // Ensure terminal is restored even if panic occurs
        self.shutdown.store(true, Ordering::Relaxed);

        // Wait briefly for refresh thread to finish
        let value = self.refresh_handle.lock().unwrap().take();
        if let Some(handle) = value {
            let _ = handle.join();
        }
        // Just add a newline and show cursor - the refresh thread already positioned us correctly
        let mut stderr = self.stderr.lock().unwrap();
        let _ = writeln!(stderr);
        let _ = stderr.execute(cursor::Show);
        let _ = stderr.flush();
    }
}

/// Install a panic hook to ensure terminal cleanup on panic.
fn install_panic_hook() {
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        // Try to restore terminal
        use crossterm::cursor;
        use crossterm::ExecutableCommand;
        let _ = std::io::stdout().execute(cursor::Show);
        let _ = std::io::stderr().execute(cursor::Show);

        // Call the original panic hook
        original_hook(panic_info);
    }));
}
