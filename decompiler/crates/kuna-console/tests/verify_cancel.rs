//! (kuna cancel) A host takes back work it no longer wants.
//!
//! An embedding viewer starts a decompile, the user clicks on to another
//! function, and the first answer is worth nothing; or the window closes while
//! the binary is still being analysed. `kuna_cancel` lets the host stop both
//! from another thread: the drive stops at the watchdog's probes, the analysis
//! walk at its next function, and neither leaves the program unusable.

use std::path::PathBuf;

use kuna_console::engine::{bootstrap_from_object, ConsoleProgram};
use kuna_decomp::decompile_drive::{
    build_and_follow_flow_within_budget, decompile_func_full_with_override_dyn_prefollowed,
    is_budget_stop, DrivePurpose,
};
use kuna_decomp::kuna_cancel::{Cancel, CancelScope, CANCELLED};

fn bootstrap() -> ConsoleProgram {
    // Not canonicalized: on Windows that makes a `\\?\` path, which the spec
    // search does not resolve.
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let spec_roots = vec![root.join("specs").to_str().unwrap().to_string()];
    let bin = root.join("decompiler/crates/kuna-analysis/tests/fixtures/fmt_x86_64");
    bootstrap_from_object(bin.to_str().unwrap(), "", &spec_roots)
        .expect("bootstrap fixture with built processor specs")
}

fn main_entry(prog: &ConsoleProgram) -> kuna_base::address::Address {
    prog.function_entries_canonical()
        .into_iter()
        .find(|e| e.name == "main")
        .expect("the fixture exports main")
        .addr
}

fn decompile_main(prog: &mut ConsoleProgram) -> kuna_base::error::KunaResult<()> {
    let entry = main_entry(prog);
    let arch = prog.arch_mut();
    let fd = build_and_follow_flow_within_budget(arch, "main", entry.clone(), 0, &[])?;
    decompile_func_full_with_override_dyn_prefollowed(
        arch,
        "main",
        entry,
        0,
        &[],
        &[],
        &[],
        None,
        &[],
        &[],
        &[],
        Some(fd),
        DrivePurpose::SingleFunction,
    )
    .map(|_| ())
}

#[test]
fn a_cancelled_drive_stops_and_the_next_one_runs() {
    let mut prog = bootstrap();
    prog.commit_pending_analysis()
        .expect("analysis commit succeeds");
    // No budget: the token alone arms the probes.
    assert!(prog.arch().kuna_fn_budget.is_none());

    let token = Cancel::new();
    token.cancel();
    {
        let _scope = CancelScope::enter(Some(token));
        let Err(error) = decompile_main(&mut prog) else {
            panic!("a cancelled drive must not finish");
        };
        assert!(
            error.explain().starts_with(CANCELLED),
            "{}",
            error.explain()
        );
        assert!(
            is_budget_stop(&error),
            "a cancel stops a drive as its budget would"
        );
    }

    let _scope = CancelScope::enter(Some(Cancel::new()));
    decompile_main(&mut prog).expect("the program is usable after a cancelled drive");
}

#[test]
fn a_cancelled_analysis_commits_nothing() {
    let mut prog = bootstrap();
    let token = Cancel::new();
    token.cancel();
    let _scope = CancelScope::enter(Some(token));
    let Err(error) = prog.commit_pending_analysis() else {
        panic!("a cancelled analysis must not commit");
    };
    assert_eq!(error.explain(), CANCELLED);
}

#[test]
fn an_analysis_under_a_live_token_commits() {
    let mut prog = bootstrap();
    let _scope = CancelScope::enter(Some(Cancel::new()));
    prog.commit_pending_analysis()
        .expect("a token nobody cancels changes nothing");
    main_entry(&prog);
}
