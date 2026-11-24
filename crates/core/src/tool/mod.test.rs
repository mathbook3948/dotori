use super::*;

#[test]
fn a_tool_is_named_as_a_verb_and_what_it_acts_on() {
    for good in [
        "add_routine",
        "read_page",
        "say",
        "run_in_browser",
        "search_memory",
    ] {
        assert!(well_named(good), "{good}");
    }
    for bad in ["routine_add", "webfetch", "Add_routine", "add_", "memory"] {
        assert!(!well_named(bad), "{bad}");
    }
    assert!(
        well_named("github__create_issue"),
        "relayed tools keep their names"
    );
}
