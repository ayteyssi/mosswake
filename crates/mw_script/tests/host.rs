//! ScriptHost behaviour: hooks, persistent state, commands, sandbox and error reports.

use mw_script::{Limits, ScriptCommand, ScriptErrorKind, ScriptHost};
use serde_json::json;

fn host(scripts: &[(&str, &str)]) -> ScriptHost {
    try_host(scripts).unwrap_or_else(|e| panic!("scripts should load: {e:#?}"))
}

fn try_host(scripts: &[(&str, &str)]) -> Result<ScriptHost, Vec<mw_script::ScriptError>> {
    let owned: Vec<(String, String)> = scripts
        .iter()
        .map(|(p, s)| (p.to_string(), s.to_string()))
        .collect();
    ScriptHost::new(&owned, Limits::default())
}

#[test]
fn hooks_are_called_and_state_persists() {
    let mut h = host(&[(
        "scripts/main.rhai",
        r#"
        state.lava_y = 0.0;                 // top-level code: runs once at load
        fn on_tick(dt) {
            state.lava_y += 20.0 * dt;
            if state.lava_y >= 1.0 { hud("The lava rises!"); }
        }
        "#,
    )]);
    assert!(h.has_hook("on_tick", 1));
    let first = h.on_tick(0.025);
    assert!(first.errors.is_empty() && first.commands.is_empty());
    let second = h.on_tick(0.025);
    assert_eq!(
        second.commands,
        [ScriptCommand::Hud {
            text: "The lava rises!".into(),
            seconds: mw_script::DEFAULT_HUD_SECONDS
        }]
    );
    assert_eq!(h.state()["lava_y"].as_float().unwrap(), 1.0);
}

#[test]
fn absent_hooks_are_ignored() {
    let mut h = host(&[("scripts/a.rhai", "fn on_tick(dt) { }")]);
    assert!(!h.has_hook("on_level_start", 0));
    assert_eq!(h.on_level_start(), Default::default());
    // A hook with another arity does not count either.
    let mut h = host(&[("scripts/a.rhai", "fn on_tick() { hud(\"wrong arity\"); }")]);
    assert!(!h.has_hook("on_tick", 1));
    assert!(h.on_tick(0.1).commands.is_empty());
}

#[test]
fn infinite_loop_is_interrupted_and_the_host_survives() {
    let mut h = host(&[
        (
            "scripts/bad.rhai",
            "fn on_tick(dt) {\n  hud(\"before\");\n  loop { }\n}",
        ),
        (
            "scripts/good.rhai",
            "fn on_tick(dt) { hud(\"still running\"); }",
        ),
    ]);
    let outcome = h.on_tick(0.1);
    assert_eq!(outcome.errors.len(), 1);
    let err = &outcome.errors[0];
    assert_eq!(err.kind, ScriptErrorKind::LimitExceeded);
    assert_eq!(err.script, "scripts/bad.rhai");
    assert_eq!(err.context, "on_tick");
    // Rhai records no position for an exhausted operation budget: the report names the
    // script and the hook instead of a line.
    assert!(err.message.contains("operations"), "{}", err.message);
    // The failed call's commands are dropped; the other script ran.
    assert_eq!(
        outcome.commands,
        [ScriptCommand::Hud {
            text: "still running".into(),
            seconds: mw_script::DEFAULT_HUD_SECONDS
        }]
    );
    // Next tick works the same way: the limit is per call.
    assert_eq!(h.on_tick(0.1).errors.len(), 1);
}

#[test]
fn syntax_errors_report_file_and_line() {
    let errors = try_host(&[
        ("scripts/ok.rhai", "fn on_tick(dt) { }"),
        ("scripts/broken.rhai", "fn on_tick(dt) {\n  let x = ;\n}"),
    ])
    .err()
    .expect("compile error");
    assert_eq!(errors.len(), 1);
    let e = &errors[0];
    assert_eq!(
        (e.script.as_str(), e.kind, e.line),
        ("scripts/broken.rhai", ScriptErrorKind::Syntax, Some(2))
    );
    assert!(e.column.is_some());
    assert!(e.to_string().starts_with("scripts/broken.rhai:2:"), "{e}");
}

#[test]
fn runtime_errors_report_location_and_drop_partial_commands() {
    let mut h = host(&[(
        "scripts/main.rhai",
        "fn on_level_start() {\n  hud(\"hello\");\n  explode(42);\n}",
    )]);
    let outcome = h.on_level_start();
    assert!(outcome.commands.is_empty(), "partial commands discarded");
    let e = &outcome.errors[0];
    assert_eq!((e.kind, e.line), (ScriptErrorKind::Runtime, Some(3)));
    assert!(e.message.contains("explode"), "{}", e.message);
}

#[test]
fn load_errors_are_reported() {
    let errors = try_host(&[("scripts/main.rhai", "let x = 1;\nthrow \"boom\";")])
        .err()
        .expect("load error");
    assert_eq!(errors[0].context, "<load>");
    assert_eq!(errors[0].line, Some(2));
}

#[test]
fn sandbox_limits_recursion_and_data_size() {
    let mut h = host(&[(
        "scripts/main.rhai",
        r#"
        fn deep(n) { deep(n + 1) }
        fn on_tick(dt) { deep(0) }
        fn on_level_start() { let s = "x"; loop { s += s; } }
        "#,
    )]);
    assert_eq!(
        h.on_tick(0.1).errors[0].kind,
        ScriptErrorKind::LimitExceeded
    );
    assert_eq!(
        h.on_level_start().errors[0].kind,
        ScriptErrorKind::LimitExceeded,
        "string growth stopped"
    );
}

#[test]
fn sandbox_has_no_eval_import_or_clock() {
    for (source, what) in [
        ("fn on_tick(dt) { eval(\"1\") }", "eval"),
        ("import \"secrets\" as s;", "import"),
    ] {
        let errors = try_host(&[("scripts/x.rhai", source)]).err();
        assert!(
            matches!(errors.as_deref(), Some([e]) if e.kind == ScriptErrorKind::Syntax),
            "{what} must not compile: {errors:?}"
        );
    }
    let mut h = host(&[("scripts/x.rhai", "fn on_tick(dt) { timestamp() }")]);
    assert_eq!(h.on_tick(0.1).errors[0].kind, ScriptErrorKind::Runtime);
}

#[test]
fn scripts_share_state_but_not_functions() {
    let mut h = host(&[
        (
            "scripts/a.rhai",
            "state.n = 1;\nfn helper() { 10 }\nfn on_tick(dt) { state.n += helper(); }",
        ),
        ("scripts/b.rhai", "fn on_tick(dt) { state.n *= 2; }"),
    ]);
    assert!(h.on_tick(0.1).errors.is_empty());
    assert_eq!(h.state()["n"].as_int().unwrap(), 22, "a then b, in order");

    let mut h = host(&[
        ("scripts/a.rhai", "fn helper() { 10 }"),
        ("scripts/b.rhai", "fn on_tick(dt) { helper() }"),
    ]);
    assert_eq!(
        h.on_tick(0.1).errors.len(),
        1,
        "a's helper is not visible to b"
    );
}

#[test]
fn commands_set_rule_and_print() {
    let mut h = host(&[(
        "scripts/main.rhai",
        r#"fn on_level_start() {
            print("inverting gravity");
            set_rule("gravity", 1800);
            set_rule("mechanics", ["movement"]);
            hud("Up is down", 5.0);
        }"#,
    )]);
    let outcome = h.on_level_start();
    assert!(outcome.errors.is_empty(), "{:?}", outcome.errors);
    assert_eq!(
        outcome.commands,
        [
            ScriptCommand::Log {
                message: "inverting gravity".into()
            },
            ScriptCommand::SetRule {
                key: "gravity".into(),
                value: json!(1800)
            },
            ScriptCommand::SetRule {
                key: "mechanics".into(),
                value: json!(["movement"])
            },
            ScriptCommand::Hud {
                text: "Up is down".into(),
                seconds: 5.0
            },
        ]
    );

    let mut h = host(&[(
        "scripts/main.rhai",
        "fn on_level_start() { set_rule(\"gravity\", || 1) }",
    )]);
    let e = &h.on_level_start().errors[0];
    assert!(
        e.message.contains("cannot be a rule value"),
        "{}",
        e.message
    );
}

#[test]
fn hosts_are_deterministic() {
    let run = || {
        let mut h = host(&[(
            "scripts/main.rhai",
            "state.t = 0.0;\nfn on_tick(dt) { state.t += dt * 3.3; if state.t % 1.0 < 0.1 { hud(`t=${state.t}`); } }",
        )]);
        let mut commands = Vec::new();
        for _ in 0..600 {
            commands.extend(h.on_tick(1.0 / 60.0).commands);
        }
        (commands, format!("{:?}", h.state()))
    };
    assert_eq!(run(), run());
}

#[test]
fn bundle_scripts_load_in_level_order() {
    let mut level = mw_level::Level::new("t", "T", mw_level::Size { w: 1, h: 1 });
    level.scripts = vec!["scripts/b.rhai".into(), "scripts/a.rhai".into()];
    let mut bundle = mw_level::LevelBundle {
        level,
        scripts: Default::default(),
    };
    bundle
        .scripts
        .insert("scripts/a.rhai".into(), "state.order += \"a\";".into());
    bundle
        .scripts
        .insert("scripts/b.rhai".into(), "state.order = \"b\";".into());
    let h = ScriptHost::from_bundle(&bundle, Limits::default()).unwrap();
    assert_eq!(h.state()["order"].clone().into_string().unwrap(), "ba");

    bundle.scripts.remove("scripts/a.rhai");
    let errors = ScriptHost::from_bundle(&bundle, Limits::default())
        .err()
        .unwrap();
    assert_eq!(errors[0].script, "scripts/a.rhai");
}
