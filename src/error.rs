use std::{
    fmt::{self, Display, Formatter},
    process::ExitStatus,
};

pub type RtResult<T> = Result<T, RtError>;

#[derive(Debug, Clone)]
pub struct RtError {
    exit_code: u8,
    message: Option<String>,
}

impl RtError {
    #[must_use]
    pub fn message(message: impl Into<String>) -> Self {
        Self {
            exit_code: 1,
            message: Some(message.into()),
        }
    }

    #[must_use]
    pub fn with_code(exit_code: u8, message: impl Into<String>) -> Self {
        Self {
            exit_code,
            message: Some(message.into()),
        }
    }

    #[must_use]
    pub const fn silent(exit_code: u8) -> Self {
        Self {
            exit_code,
            message: None,
        }
    }

    #[must_use]
    pub fn silent_from_status(status: ExitStatus) -> Self {
        Self::silent(status_exit_code(status))
    }

    #[must_use]
    pub const fn exit_code(&self) -> u8 {
        self.exit_code
    }

    pub fn report(&self) {
        if let Some(message) = &self.message {
            eprintln!("{message}");
        }
    }
}

impl Display for RtError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        if let Some(message) = &self.message {
            f.write_str(message)
        } else {
            write!(f, "process exited with status {}", self.exit_code)
        }
    }
}

impl std::error::Error for RtError {}

impl From<std::io::Error> for RtError {
    fn from(value: std::io::Error) -> Self {
        Self::message(format!("error: {value}"))
    }
}

impl From<std::fmt::Error> for RtError {
    fn from(value: std::fmt::Error) -> Self {
        Self::message(format!("error: {value}"))
    }
}

impl From<std::ffi::NulError> for RtError {
    fn from(value: std::ffi::NulError) -> Self {
        Self::message(format!("error: {value}"))
    }
}

impl From<pyo3::PyErr> for RtError {
    fn from(value: pyo3::PyErr) -> Self {
        Self::message(format!("error: {value}"))
    }
}

fn status_exit_code(status: ExitStatus) -> u8 {
    status
        .code()
        .and_then(|code| u8::try_from(code).ok())
        .unwrap_or(1)
}
