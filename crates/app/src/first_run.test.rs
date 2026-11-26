use super::*;

#[test]
fn each_state_reads_back_as_it_was_written() {
    let root = std::env::temp_dir().join(format!("dotori-first-run-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let path = root.join("first-run");

    assert_eq!(read(&path), FirstRun::Unbegun);
    for state in Step::ALL
        .into_iter()
        .map(FirstRun::At)
        .chain([FirstRun::Done])
    {
        write(&path, state).expect("written");
        assert_eq!(read(&path), state);
    }

    fs::write(&path, "somewhere else\n").expect("written");
    assert_eq!(read(&path), FirstRun::At(Step::Welcome));
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn the_steps_run_from_the_greeting_to_the_other_places() {
    assert_eq!(Step::Welcome.previous(), None);
    assert_eq!(Step::Welcome.next(), Some(Step::Key));
    assert_eq!(Step::Look.next(), Some(Step::Elsewhere));
    assert_eq!(Step::Elsewhere.next(), None);
}
