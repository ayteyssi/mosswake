//! [`ScriptHost`]: compiles a level's scripts in a sandboxed engine and calls their hooks.

use std::sync::{Arc, Mutex};

use rhai::packages::{Package, StandardPackage};
use rhai::{CallFnOptions, Dynamic, Engine, FuncArgs, ImmutableString, Map, Scope, AST};

use crate::command::{dynamic_to_json, ScriptCommand, DEFAULT_HUD_SECONDS};
use crate::error::{ScriptError, ScriptErrorKind};

/// Sandbox limits, applied to every hook call (and to top-level code at load).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Rhai operations per call; a runaway loop is stopped at this point.
    pub max_operations: u64,
    /// Function call nesting.
    pub max_call_levels: usize,
    /// Expression nesting (global code, inside functions).
    pub max_expr_depth: (usize, usize),
    /// Longest string, in bytes.
    pub max_string_size: usize,
    /// Most elements in an array.
    pub max_array_size: usize,
    /// Most properties in an object map.
    pub max_map_size: usize,
    /// Most variables alive at once.
    pub max_variables: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            // ~1 ms of script per hook call: hooks run up to 60 times a second.
            max_operations: 100_000,
            max_call_levels: 32,
            max_expr_depth: (64, 32),
            max_string_size: 4096,
            max_array_size: 4096,
            max_map_size: 1024,
            max_variables: 512,
        }
    }
}

/// Name of the persistent global object visible to every script and hook.
pub const STATE_VAR: &str = "state";

struct Script {
    path: String,
    ast: AST,
}

/// Result of calling a hook on every script that defines it.
#[derive(Debug, Default, PartialEq)]
pub struct HookOutcome {
    /// Commands emitted by successful calls, in script order.
    pub commands: Vec<ScriptCommand>,
    /// One error per failing script (the other scripts still ran).
    pub errors: Vec<ScriptError>,
}

/// The scripts of one level, compiled once, sharing one persistent `state` object.
///
/// Each script file is independent (its functions are not visible to the others); they all
/// see the same `state`. Hooks are optional: a script defines only those it needs. Top-level
/// code of each script runs once at load, which is the place to initialize `state`.
pub struct ScriptHost {
    engine: Engine,
    scripts: Vec<Script>,
    state: Dynamic,
    commands: Arc<Mutex<Vec<ScriptCommand>>>,
}

impl ScriptHost {
    /// Compiles `scripts` (`(path, source)`, in execution order) and runs their top-level
    /// code. Returns every compile or load error.
    pub fn new(scripts: &[(String, String)], limits: Limits) -> Result<Self, Vec<ScriptError>> {
        let commands = Arc::new(Mutex::new(Vec::new()));
        let state = Dynamic::from_map(Map::new()).into_shared();
        let engine = sandboxed_engine(limits, &state, &commands);

        let mut compiled = Vec::new();
        let mut errors = Vec::new();
        for (path, source) in scripts {
            match engine.compile(source) {
                Ok(mut ast) => {
                    ast.set_source(path.as_str());
                    compiled.push(Script {
                        path: path.clone(),
                        ast,
                    });
                }
                Err(e) => errors.push(ScriptError::from_parse(path, &e)),
            }
        }
        if !errors.is_empty() {
            return Err(errors);
        }
        let host = Self {
            engine,
            scripts: compiled,
            state,
            commands,
        };
        for script in &host.scripts {
            if let Err(e) = host
                .engine
                .run_ast_with_scope(&mut Scope::new(), &script.ast)
            {
                errors.push(ScriptError::from_eval(&script.path, "<load>", &e));
            }
        }
        if !errors.is_empty() {
            return Err(errors);
        }
        host.take_commands(0); // load-time output is not a hook result
        Ok(host)
    }

    /// Compiles the scripts of a level bundle, in the order of `level.scripts`.
    pub fn from_bundle(
        bundle: &mw_level::LevelBundle,
        limits: Limits,
    ) -> Result<Self, Vec<ScriptError>> {
        let mut scripts = Vec::new();
        let mut missing = Vec::new();
        for path in &bundle.level.scripts {
            match bundle.scripts.get(path) {
                Some(source) => scripts.push((path.clone(), source.clone())),
                None => missing.push(ScriptError {
                    script: path.clone(),
                    context: "<load>".into(),
                    kind: ScriptErrorKind::Runtime,
                    line: None,
                    column: None,
                    message: "script source is missing".into(),
                }),
            }
        }
        if !missing.is_empty() {
            return Err(missing);
        }
        Self::new(&scripts, limits)
    }

    /// Whether any script defines `name` with `arity` parameters.
    pub fn has_hook(&self, name: &str, arity: usize) -> bool {
        self.scripts.iter().any(|s| defines(&s.ast, name, arity))
    }

    /// Calls `name(args)` on every script defining it with that arity, in order. Scripts
    /// without it are skipped. A failing call is reported and its commands discarded; the
    /// other scripts still run. Never panics.
    pub fn call_hook(&mut self, name: &str, args: impl FuncArgs + Clone) -> HookOutcome {
        let mut arg_values = Vec::new();
        args.clone().parse(&mut arg_values);
        let arity = arg_values.len();
        let mut outcome = HookOutcome::default();
        for script in &self.scripts {
            if !defines(&script.ast, name, arity) {
                continue;
            }
            let before = self.commands.lock().map(|c| c.len()).unwrap_or(0);
            let options = CallFnOptions::new().eval_ast(false).rewind_scope(true);
            let result = self.engine.call_fn_with_options::<Dynamic>(
                options,
                &mut Scope::new(),
                &script.ast,
                name,
                args.clone(),
            );
            if let Err(e) = result {
                self.take_commands(before); // drop what the failed call emitted
                outcome
                    .errors
                    .push(ScriptError::from_eval(&script.path, name, &e));
            }
        }
        outcome.commands = self.take_commands(0);
        outcome
    }

    /// `on_level_start()`: once, after loading.
    pub fn on_level_start(&mut self) -> HookOutcome {
        self.call_hook("on_level_start", ())
    }

    /// `on_tick(dt)`: every simulation tick, `dt` in seconds.
    pub fn on_tick(&mut self, dt: f64) -> HookOutcome {
        self.call_hook("on_tick", (dt,))
    }

    /// A snapshot of the persistent `state` object (for tests and the debug console).
    pub fn state(&self) -> Map {
        self.state.clone().flatten().cast::<Map>()
    }

    /// Removes and returns the commands queued after index `from`.
    fn take_commands(&self, from: usize) -> Vec<ScriptCommand> {
        // A panic while holding the lock cannot leave the Vec inconsistent: recover it.
        let mut queue = self
            .commands
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let at = from.min(queue.len());
        queue.split_off(at)
    }
}

fn defines(ast: &AST, name: &str, arity: usize) -> bool {
    ast.iter_functions()
        .any(|f| f.name == name && f.params.len() == arity)
}

/// An engine with only the standard language (no time, no modules, no `eval`), the sandbox
/// `limits`, the `state` variable and the command-emitting API.
fn sandboxed_engine(
    limits: Limits,
    state: &Dynamic,
    commands: &Arc<Mutex<Vec<ScriptCommand>>>,
) -> Engine {
    let mut engine = Engine::new_raw();
    engine.register_global_module(StandardPackage::new().as_shared_module());
    engine
        .disable_symbol("eval")
        .set_max_operations(limits.max_operations)
        .set_max_call_levels(limits.max_call_levels)
        .set_max_expr_depths(limits.max_expr_depth.0, limits.max_expr_depth.1)
        .set_max_string_size(limits.max_string_size)
        .set_max_array_size(limits.max_array_size)
        .set_max_map_size(limits.max_map_size)
        .set_max_variables(limits.max_variables);

    // `state` must be visible inside functions, which cannot see the global scope: resolve
    // it explicitly. It is a *shared* value, so writes through it persist between calls.
    let shared = state.clone();
    #[allow(deprecated)] // `on_var` is stable but flagged "volatile" by Rhai
    engine.on_var(move |name, _, _| Ok((name == STATE_VAR).then(|| shared.clone())));

    let push = |commands: &Arc<Mutex<Vec<ScriptCommand>>>| {
        let commands = Arc::clone(commands);
        move |command: ScriptCommand| {
            if let Ok(mut queue) = commands.lock() {
                queue.push(command);
            }
        }
    };
    let log = push(commands);
    engine.on_print(move |text| {
        log(ScriptCommand::Log {
            message: text.to_owned(),
        })
    });
    let log = push(commands);
    engine.on_debug(move |text, source, pos| {
        log(ScriptCommand::Log {
            message: format!("[{}:{pos}] {text}", source.unwrap_or("?")),
        })
    });
    let hud = push(commands);
    engine.register_fn("hud", move |text: ImmutableString| {
        hud(ScriptCommand::Hud {
            text: text.to_string(),
            seconds: DEFAULT_HUD_SECONDS,
        })
    });
    let hud = push(commands);
    engine.register_fn("hud", move |text: ImmutableString, seconds: rhai::FLOAT| {
        hud(ScriptCommand::Hud {
            text: text.to_string(),
            seconds: seconds.max(0.0),
        })
    });
    let rule = push(commands);
    engine.register_fn(
        "set_rule",
        move |key: ImmutableString, value: Dynamic| -> Result<(), Box<rhai::EvalAltResult>> {
            let value = dynamic_to_json(&value)
                .ok_or_else(|| format!("set_rule(\"{key}\", …): value cannot be a rule value"))?;
            rule(ScriptCommand::SetRule {
                key: key.to_string(),
                value,
            });
            Ok(())
        },
    );
    engine
}
