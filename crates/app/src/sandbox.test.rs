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
fn the_profile_lets_only_dotori_s_copy_of_bubblewrap_make_namespaces() {
    let profile = profile();

    assert!(profile.starts_with("abi <abi/4.0>,"), "{profile}");
    assert!(
        profile.contains(&format!(
            "profile dotori-bwrap {HELPER} flags=(unconfined) {{"
        )),
        "{profile}"
    );
    assert!(profile.contains("  userns,\n"), "{profile}");
}

#[test]
fn the_command_to_run_by_hand_names_this_program() {
    let line = by_hand();

    assert!(line.starts_with("sudo "), "{line}");
    assert!(line.ends_with(" sandbox install"), "{line}");
    assert!(line.contains("dotori"), "{line}");
}
