use std::fmt;

/// Errors returned by reusable Quilt operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuiltError {
    Validation {
        diagnostics: Vec<String>,
    },
    Usage {
        message: String,
    },
    Schema {
        operation: String,
        column: Option<String>,
        message: String,
    },
    Conversion {
        operation: String,
        column: Option<String>,
        message: String,
    },
    Io {
        operation: String,
        path: Option<String>,
        message: String,
    },
    Operation {
        operation: String,
        message: String,
    },
    Finalizer {
        operation: String,
        message: String,
    },
    Automation {
        stage: String,
        step: Option<String>,
        message: String,
        source: Option<Box<QuiltError>>,
    },
}

impl QuiltError {
    pub fn usage(message: impl Into<String>) -> Self {
        Self::Usage {
            message: message.into(),
        }
    }
    pub fn validation(diagnostics: Vec<String>) -> Self {
        Self::Validation { diagnostics }
    }
    pub fn schema(
        operation: impl Into<String>,
        column: Option<impl Into<String>>,
        message: impl Into<String>,
    ) -> Self {
        Self::Schema {
            operation: operation.into(),
            column: column.map(Into::into),
            message: message.into(),
        }
    }
    pub fn conversion(
        operation: impl Into<String>,
        column: Option<impl Into<String>>,
        message: impl Into<String>,
    ) -> Self {
        Self::Conversion {
            operation: operation.into(),
            column: column.map(Into::into),
            message: message.into(),
        }
    }
    pub fn operation(operation: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Operation {
            operation: operation.into(),
            message: message.into(),
        }
    }
    pub fn finalizer(operation: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Finalizer {
            operation: operation.into(),
            message: message.into(),
        }
    }
    pub fn io(
        operation: impl Into<String>,
        path: Option<impl Into<String>>,
        message: impl Into<String>,
    ) -> Self {
        Self::Io {
            operation: operation.into(),
            path: path.map(Into::into),
            message: message.into(),
        }
    }
    pub fn automation(
        stage: impl Into<String>,
        step: Option<String>,
        message: impl Into<String>,
    ) -> Self {
        Self::Automation {
            stage: stage.into(),
            step,
            message: message.into(),
            source: None,
        }
    }
    pub fn automation_with_source(
        stage: impl Into<String>,
        step: Option<String>,
        source: QuiltError,
    ) -> Self {
        Self::Automation {
            stage: stage.into(),
            step,
            message: source.to_string(),
            source: Some(Box::new(source)),
        }
    }
    pub fn class(&self) -> ErrorClass {
        match self {
            Self::Validation { .. } => ErrorClass::Usage,
            Self::Usage { .. } => ErrorClass::Usage,
            Self::Schema { .. } => ErrorClass::Schema,
            Self::Conversion { .. } => ErrorClass::Conversion,
            Self::Io { .. } => ErrorClass::Io,
            Self::Operation { .. } => ErrorClass::Operation,
            Self::Finalizer { .. } => ErrorClass::Finalizer,
            Self::Automation {
                source: Some(source),
                ..
            } => source.class(),
            Self::Automation { .. } => ErrorClass::Automation,
        }
    }
    pub fn source_error(&self) -> Option<&QuiltError> {
        match self {
            Self::Automation {
                source: Some(source),
                ..
            } => Some(source),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorClass {
    Usage,
    Schema,
    Conversion,
    Io,
    Operation,
    Finalizer,
    Automation,
}

impl fmt::Display for QuiltError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Validation { diagnostics } => {
                for (index, diagnostic) in diagnostics.iter().enumerate() {
                    if index > 0 {
                        writeln!(f)?;
                    }
                    write!(f, "{diagnostic}")?;
                }
                Ok(())
            }
            Self::Usage { message } => write!(f, "{message}"),
            Self::Schema {
                operation,
                column,
                message,
            } => write_context(f, operation, column.as_deref(), message),
            Self::Conversion {
                operation,
                column,
                message,
            } => write_context(f, operation, column.as_deref(), message),
            Self::Io {
                operation,
                path,
                message,
            } => write_context(f, operation, path.as_deref(), message),
            Self::Operation { operation, message } => write!(f, "{operation}: {message}"),
            Self::Finalizer { operation, message } => write!(f, "{operation}: {message}"),
            Self::Automation {
                stage,
                step,
                message,
                ..
            } => {
                if let Some(step) = step {
                    write!(f, "automation stage '{stage}', step '{step}': {message}")
                } else {
                    write!(f, "automation stage '{stage}': {message}")
                }
            }
        }
    }
}

fn write_context(
    f: &mut fmt::Formatter<'_>,
    operation: &str,
    context: Option<&str>,
    message: &str,
) -> fmt::Result {
    if let Some(context) = context {
        write!(f, "{operation} ({context}): {message}")
    } else {
        write!(f, "{operation}: {message}")
    }
}

impl std::error::Error for QuiltError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source_error()
            .map(|source| source as &(dyn std::error::Error + 'static))
    }
}

impl From<std::io::Error> for QuiltError {
    fn from(error: std::io::Error) -> Self {
        Self::Io {
            operation: "I/O".to_string(),
            path: None,
            message: error.to_string(),
        }
    }
}

impl From<polars::error::PolarsError> for QuiltError {
    fn from(error: polars::error::PolarsError) -> Self {
        Self::operation("Polars", error.to_string())
    }
}
