use super::*;

#[test]
fn each_failure_comes_back_from_the_code_it_ends_with() {
    for failure in [
        Failure::NoBubblewrap,
        Failure::NoAppArmor,
        Failure::NoNamespaces,
        Failure::Other,
    ] {
        assert_eq!(Failure::of_exit(Some(i32::from(failure.code()))), failure);
    }
    assert_eq!(Failure::of_exit(Some(126)), Failure::Refused);
    assert_eq!(Failure::of_exit(Some(127)), Failure::Refused);
    assert_eq!(Failure::of_exit(Some(9)), Failure::Other);
    assert_eq!(Failure::of_exit(None), Failure::Other);
}

#[test]
fn the_profile_lets_only_dotori_s_copy_make_namespaces_and_not_what_it_runs() {
    let profile = profile();
    let (helper, child) = profile
        .split_once("profile dotori-bwrap-child ")
        .expect("a profile for what it runs");

    assert!(profile.starts_with("abi <abi/4.0>,"), "{profile}");
    assert!(
        helper.contains(&format!("profile dotori-bwrap {HELPER} flags=(")),
        "{profile}"
    );
    assert!(helper.contains("  allow userns,\n"), "{profile}");
    assert!(
        helper.contains("  allow px /** -> dotori-bwrap-child,\n"),
        "{profile}"
    );
    assert!(child.contains("  deny capability,\n"), "{profile}");
    assert!(child.contains("  audit deny userns,\n"), "{profile}");
    assert!(!child.contains("allow userns"), "{profile}");
    assert_eq!(profile.matches('{').count(), profile.matches('}').count());
}

#[test]
fn the_command_to_run_by_hand_names_this_program() {
    let line = by_hand();

    assert!(line.starts_with("sudo '"), "{line}");
    assert!(line.ends_with("' sandbox install"), "{line}");
    assert!(line.ends_with(" sandbox install"), "{line}");
    assert!(line.contains("dotori"), "{line}");
}
