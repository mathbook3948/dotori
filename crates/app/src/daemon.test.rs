#[cfg(target_os = "linux")]
#[test]
fn the_process_is_closed_to_other_programs_of_its_user() {
    use std::os::unix::fs::MetadataExt;

    use super::keep_to_itself;

    keep_to_itself();

    assert_eq!(
        rustix::process::dumpable_behavior().expect("read"),
        rustix::process::DumpableBehavior::NotDumpable
    );
    let environment = std::fs::metadata("/proc/self/environ").expect("there");
    assert_eq!(environment.uid(), 0, "only root may read its environment");
}
