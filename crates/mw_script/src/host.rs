//! [`ScriptHost`]: compiles a level's scripts in a sandboxed engine and calls their hooks.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use rhai::packages::{Package, StandardPackage};
use rhai::{CallFnOptions, Dynamic, Engine, FuncArgs, ImmutableString, Map, Scope, AST, INT};

use crate::command::{dynamic_to_json, ScriptCommand, DEFAULT_HUD_SECONDS};
use crate::error::{ScriptError, ScriptErrorKind};
use crate::world::{json_to_dynamic, Entity, EntityData, Shared, Sink, Timer, TimerBook};

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

/// Name of the gameplay rules object (`rules.gravity`…); edits become `SetRule` commands.
pub const RULES_VAR: &str = "rules";

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
/// see the same `state` and `rules`. Hooks are optional: a script defines only those it
/// needs. Top-level code of each script runs once at load, which is the place to initialize
/// `state`.
///
/// Before calling hooks, the simulation provides the world ([`ScriptHost::set_view`]) and the
/// rules ([`ScriptHost::set_rules`]); every change scripts make comes back as
/// [`ScriptCommand`]s in the [`HookOutcome`].
pub struct ScriptHost {
    engine: Engine,
    scripts: Vec<Script>,
    state: Dynamic,
    shared: Shared,
    /// Rules as last given or as left by the last successful call (JSON per key).
    rules_baseline: BTreeMap<String, serde_json::Value>,
    timers: Vec<Timer>,
}

impl ScriptHost {
    /// Compiles `scripts` (`(path, source)`, in execution order) and runs their top-level
    /// code. Returns every compile or load error.
    pub fn new(scripts: &[(String, String)], limits: Limits) -> Result<Self, Vec<ScriptError>> {
        let state = Dynamic::from_map(Map::new()).into_shared();
        let shared = Shared {
            sink: Arc::new(Mutex::new(Vec::new())),
            view: Arc::new(Mutex::new(Vec::new())),
            timers: Arc::new(Mutex::new(TimerBook {
                ticks_per_second: 60.0,
                ..TimerBook::default()
            })),
            rules: Dynamic::from_map(Map::new()).into_shared(),
        };
        let engine = sandboxed_engine(limits, &state, &shared);

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
        let mut host = Self {
            engine,
            scripts: compiled,
            state,
            shared,
            rules_baseline: BTreeMap::new(),
            timers: Vec::new(),
        };
        for index in 0..host.scripts.len() {
            host.book().script = index;
            let script = &host.scripts[index];
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
        host.collect_timers();
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

    /// Simulation ticks per second, to convert timer delays (default 60).
    pub fn set_tick_rate(&mut self, hz: f64) {
        self.book().ticks_per_second = hz;
    }

    /// The world scripts see during the next hooks (`players()`, `entities()`…).
    pub fn set_view(&mut self, bodies: Vec<EntityData>) {
        *self
            .shared
            .view
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = bodies;
    }

    /// The current gameplay rules (a JSON object), visible as `rules`. Changes scripts make
    /// to it are returned as `SetRule` commands.
    pub fn set_rules(&mut self, rules: &serde_json::Value) {
        self.rules_baseline = rules
            .as_object()
            .map(|o| o.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
            .unwrap_or_default();
        self.write_rules_from_baseline();
    }

    /// An [`Entity`] script value for hook arguments.
    pub fn entity(&self, data: EntityData) -> Dynamic {
        Dynamic::from(Entity::new(data, Arc::clone(&self.shared.sink)))
    }

    /// Whether any script defines `name` with `arity` parameters.
    pub fn has_hook(&self, name: &str, arity: usize) -> bool {
        self.scripts.iter().any(|s| defines(&s.ast, name, arity))
    }

    /// Calls `name(args)` on every script defining it with that arity, in order. Scripts
    /// without it are skipped. A failing call is reported and its commands (and rule edits)
    /// discarded; the other scripts still run. Never panics.
    pub fn call_hook(&mut self, name: &str, args: impl FuncArgs + Clone) -> HookOutcome {
        let mut values = Vec::new();
        args.parse(&mut values);
        let mut outcome = HookOutcome::default();
        self.call_each(name, &values, &mut outcome, |_| {});
        outcome.commands = self.take_commands(0);
        outcome
    }

    /// A hook whose last argument is a value each script may replace by returning an
    /// integer (e.g. `on_damage(target, source, amount)` returns the final amount). Scripts
    /// are chained in order; a script returning anything else leaves the value unchanged.
    pub fn filter_hook(
        &mut self,
        name: &str,
        mut args: Vec<Dynamic>,
        value: INT,
    ) -> (INT, HookOutcome) {
        let mut outcome = HookOutcome::default();
        let mut current = value;
        args.push(Dynamic::from_int(current));
        let arity = args.len();
        for index in 0..self.scripts.len() {
            if !defines(&self.scripts[index].ast, name, arity) {
                continue;
            }
            *args.last_mut().expect("value argument") = Dynamic::from_int(current);
            if let Some(result) = self.call_one(index, name, &args, &mut outcome) {
                if let Some(v) = result.try_cast::<INT>() {
                    current = v;
                }
            }
        }
        outcome.commands = self.take_commands(0);
        (current, outcome)
    }

    /// `on_level_start()`: once, after loading.
    pub fn on_level_start(&mut self) -> HookOutcome {
        self.call_hook("on_level_start", ())
    }

    /// `on_tick(dt)`: every simulation tick, `dt` in seconds; then the timers that are due.
    pub fn on_tick(&mut self, dt: f64) -> HookOutcome {
        self.book().now += 1;
        let mut outcome = HookOutcome::default();
        self.call_each("on_tick", &[Dynamic::from_float(dt)], &mut outcome, |_| {});
        self.run_timers(&mut outcome);
        outcome.commands = self.take_commands(0);
        outcome
    }

    /// A snapshot of the persistent `state` object (for tests and the debug console).
    pub fn state(&self) -> Map {
        self.state.clone().flatten().cast::<Map>()
    }

    /// Number of pending timers.
    pub fn timer_count(&self) -> usize {
        self.timers.len()
    }

    fn book(&self) -> std::sync::MutexGuard<'_, TimerBook> {
        self.shared
            .timers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn call_each(
        &mut self,
        name: &str,
        args: &[Dynamic],
        outcome: &mut HookOutcome,
        mut each: impl FnMut(Dynamic),
    ) {
        for index in 0..self.scripts.len() {
            if defines(&self.scripts[index].ast, name, args.len()) {
                if let Some(result) = self.call_one(index, name, args, outcome) {
                    each(result);
                }
            }
        }
    }

    /// Calls one script function; on error, rolls back its commands and rule edits.
    fn call_one(
        &mut self,
        index: usize,
        name: &str,
        args: &[Dynamic],
        outcome: &mut HookOutcome,
    ) -> Option<Dynamic> {
        let before = self.shared.sink.lock().map(|c| c.len()).unwrap_or(0);
        self.book().script = index;
        let script = &self.scripts[index];
        let options = CallFnOptions::new().eval_ast(false).rewind_scope(true);
        let result = self.engine.call_fn_with_options::<Dynamic>(
            options,
            &mut Scope::new(),
            &script.ast,
            name,
            args.to_vec(),
        );
        self.after_call(index, name, result, before, outcome)
    }

    fn after_call(
        &mut self,
        index: usize,
        context: &str,
        result: Result<Dynamic, Box<rhai::EvalAltResult>>,
        before: usize,
        outcome: &mut HookOutcome,
    ) -> Option<Dynamic> {
        match result {
            Ok(value) => {
                self.commit_rules();
                self.collect_timers();
                Some(value)
            }
            Err(e) => {
                self.take_commands(before);
                self.write_rules_from_baseline();
                self.book().added.clear();
                outcome.errors.push(ScriptError::from_eval(
                    &self.scripts[index].path,
                    context,
                    &e,
                ));
                None
            }
        }
    }

    /// Turns edits of `rules` since the baseline into `SetRule` commands.
    fn commit_rules(&mut self) {
        let current: BTreeMap<String, serde_json::Value> = self
            .shared
            .rules
            .read_lock::<Map>()
            .map(|m| {
                m.iter()
                    .filter_map(|(k, v)| dynamic_to_json(v).map(|j| (k.to_string(), j)))
                    .collect()
            })
            .unwrap_or_default();
        let mut sink = self
            .shared
            .sink
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        for (key, value) in &current {
            if self.rules_baseline.get(key) != Some(value) {
                sink.push(ScriptCommand::SetRule {
                    key: key.clone(),
                    value: value.clone(),
                });
            }
        }
        drop(sink);
        self.rules_baseline = current;
    }

    fn write_rules_from_baseline(&mut self) {
        let map: Map = self
            .rules_baseline
            .iter()
            .map(|(k, v)| (k.as_str().into(), json_to_dynamic(v)))
            .collect();
        if let Some(mut rules) = self.shared.rules.write_lock::<Map>() {
            *rules = map;
        }
    }

    fn collect_timers(&mut self) {
        let mut book = self.book();
        let added = std::mem::take(&mut book.added);
        let cancelled = std::mem::take(&mut book.cancelled);
        drop(book);
        self.timers.extend(added);
        self.timers.retain(|t| !cancelled.contains(&t.id));
    }

    /// Runs every due timer, oldest due first (then by id), rescheduling repeating ones.
    fn run_timers(&mut self, outcome: &mut HookOutcome) {
        let now = self.book().now;
        loop {
            let next = self
                .timers
                .iter()
                .enumerate()
                .filter(|(_, t)| t.due <= now)
                .min_by_key(|(_, t)| (t.due, t.id))
                .map(|(i, _)| i);
            let Some(i) = next else { break };
            let mut timer = self.timers.remove(i);
            let before = self.shared.sink.lock().map(|c| c.len()).unwrap_or(0);
            self.book().script = timer.script;
            let ast = &self.scripts[timer.script].ast;
            let result = timer.callback.call::<Dynamic>(&self.engine, ast, ());
            // Read before `after_call`, which consumes pending cancellations: the timer is
            // out of `self.timers` while it runs, so `cancel(own id)` must be seen here.
            let cancelled = self.book().cancelled.remove(&timer.id);
            if self
                .after_call(timer.script, "timer", result, before, outcome)
                .is_some()
            {
                if let Some(period) = timer.period.filter(|_| !cancelled) {
                    timer.due += period;
                    self.timers.push(timer);
                }
            }
        }
    }

    /// Removes and returns the commands queued after index `from`.
    fn take_commands(&self, from: usize) -> Vec<ScriptCommand> {
        // A panic while holding the lock cannot leave the Vec inconsistent: recover it.
        let mut queue = self
            .shared
            .sink
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let at = from.min(queue.len());
        queue.split_off(at)
    }
}

fn defines(ast: &AST, name: &str, arity: usize) -> bool {
    ast.iter_functions()
        .any(|f| f.name == name && f.params.len() == arity)
}

/// An engine with only the standard language (no time, no modules, no `eval`), the sandbox
/// `limits`, the `state` and `rules` variables and the world API.
fn sandboxed_engine(limits: Limits, state: &Dynamic, shared: &Shared) -> Engine {
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

    // `state` and `rules` must be visible inside functions, which cannot see the global
    // scope: resolve them explicitly. They are *shared* values, so writes through them
    // persist (state) or can be compared after the call (rules).
    let (state_ref, rules_ref) = (state.clone(), shared.rules.clone());
    #[allow(deprecated)] // `on_var` is stable but flagged "volatile" by Rhai
    engine.on_var(move |name, _, _| {
        Ok(match name {
            STATE_VAR => Some(state_ref.clone()),
            RULES_VAR => Some(rules_ref.clone()),
            _ => None,
        })
    });

    let push = |sink: &Sink| {
        let sink = Arc::clone(sink);
        move |command: ScriptCommand| {
            sink.lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(command);
        }
    };
    let log = push(&shared.sink);
    engine.on_print(move |text| {
        log(ScriptCommand::Log {
            message: text.to_owned(),
        })
    });
    let log = push(&shared.sink);
    engine.on_debug(move |text, source, pos| {
        log(ScriptCommand::Log {
            message: format!("[{}:{pos}] {text}", source.unwrap_or("?")),
        })
    });
    let hud = push(&shared.sink);
    engine.register_fn("hud", move |text: ImmutableString| {
        hud(ScriptCommand::Hud {
            text: text.to_string(),
            seconds: DEFAULT_HUD_SECONDS,
        })
    });
    let hud = push(&shared.sink);
    engine.register_fn("hud", move |text: ImmutableString, seconds: rhai::FLOAT| {
        hud(ScriptCommand::Hud {
            text: text.to_string(),
            seconds: seconds.max(0.0),
        })
    });
    let rule = push(&shared.sink);
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
    crate::world::register(&mut engine, shared);
    engine
}
