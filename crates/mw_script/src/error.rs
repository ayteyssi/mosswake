//! Structured script errors. Scripts are written by level authors (humans or agents): an
//! error must say which file, where, and why — and must never crash the game.

use rhai::{EvalAltResult, ParseError, Position};
use serde::Serialize;

/// What went wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScriptErrorKind {
    /// The script does not compile.
    Syntax,
    /// The script failed while running (type error, unknown function, `throw`…).
    Runtime,
    /// The script hit a sandbox limit (operations, call depth, data size) and was stopped.
    LimitExceeded,
}

/// A script error with its location.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, thiserror::Error)]
#[error("{script}{}: {kind:?}: {message}", location(*.line, *.column))]
pub struct ScriptError {
    /// Script path as listed by the level (e.g. `scripts/main.rhai`).
    pub script: String,
    /// What was running: `<load>` for top-level code, otherwise the hook name.
    pub context: String,
    pub kind: ScriptErrorKind,
    pub line: Option<usize>,
    pub column: Option<usize>,
    pub message: String,
}

fn location(line: Option<usize>, column: Option<usize>) -> String {
    match (line, column) {
        (Some(l), Some(c)) => format!(":{l}:{c}"),
        (Some(l), None) => format!(":{l}"),
        _ => String::new(),
    }
}

impl ScriptError {
    pub(crate) fn from_parse(script: &str, err: &ParseError) -> Self {
        let (line, column) = split(err.position());
        Self {
            script: script.into(),
            context: "<compile>".into(),
            kind: ScriptErrorKind::Syntax,
            line,
            column,
            message: err.err_type().to_string(),
        }
    }

    pub(crate) fn from_eval(script: &str, context: &str, err: &EvalAltResult) -> Self {
        // Errors inside functions are wrapped once per call level: report the innermost one,
        // which carries the real location.
        let inner = err.unwrap_inner();
        let kind = match inner {
            EvalAltResult::ErrorTooManyOperations(_)
            | EvalAltResult::ErrorStackOverflow(_)
            | EvalAltResult::ErrorDataTooLarge(..)
            | EvalAltResult::ErrorTooManyVariables(_) => ScriptErrorKind::LimitExceeded,
            _ => ScriptErrorKind::Runtime,
        };
        // Limit errors carry no position of their own (Rhai stops wherever the budget runs
        // out): fall back to the outermost known position of the call chain, i.e. the call
        // made from the hook (more useful than a line deep inside a runaway recursion).
        let mut position = inner.position();
        let mut layer = err;
        while position.is_none() {
            if !layer.position().is_none() {
                position = layer.position();
            }
            match layer {
                EvalAltResult::ErrorInFunctionCall(_, _, next, _) => layer = next,
                _ => break,
            }
        }
        let (line, column) = split(position);
        let mut message = inner.to_string();
        // Rhai appends " (line X, position Y)"; the location already has its own fields.
        if let Some(cut) = message.rfind(" (line ") {
            message.truncate(cut);
        }
        Self {
            script: script.into(),
            context: context.into(),
            kind,
            line,
            column,
            message,
        }
    }
}

fn split(position: Position) -> (Option<usize>, Option<usize>) {
    (position.line(), position.position())
}
