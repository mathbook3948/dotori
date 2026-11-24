use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use super::*;

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn root() -> PathBuf {
    let n = NEXT.fetch_add(1, Ordering::SeqCst);
    let root = std::env::temp_dir().join(format!("dotori-folder-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    root
}

fn asked(path: &Path) -> Option<String> {
    Some(path.display().to_string())
}

#[test]
fn a_job_without_a_folder_works_in_the_work_folder_which_is_made_for_it() {
    let root = root();
    let work = root.join("dotori").join("work");

    let folder = working_folder(&work, None);

    assert_eq!(folder, Ok(work.clone()));
    assert!(work.is_dir());
    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[test]
fn a_project_folder_and_a_folder_inside_the_work_folder_are_worked_in() {
    let root = root();
    let work = root.join("dotori").join("work");
    let project = root.join("project");
    let scratch = work.join("scratch");
    std::fs::create_dir_all(&project).expect("made");
    std::fs::create_dir_all(&scratch).expect("made");

    let there = working_folder(&work, asked(&project).as_deref());
    let inside = working_folder(&work, asked(&scratch).as_deref());

    assert_eq!(there, Ok(project.canonicalize().expect("exists")));
    assert_eq!(inside, Ok(scratch.canonicalize().expect("exists")));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_folder_that_is_not_on_this_pc_is_refused() {
    let root = root();
    let work = root.join("dotori").join("work");
    std::fs::create_dir_all(root.join("project")).expect("made");
    std::fs::write(root.join("file.txt"), "a file").expect("written");

    for refused in [
        asked(&root.join("nowhere")),
        asked(&root.join("file.txt")),
        Some(String::from("project")),
    ] {
        let folder = working_folder(&work, refused.as_deref());
        assert!(
            folder
                .as_ref()
                .is_err_and(|error| error.contains("is not a folder on this PC")),
            "{refused:?} gave {folder:?}"
        );
    }
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn hidden_folders_dotori_its_own_folders_home_and_the_root_are_refused() {
    let root = root();
    let dotori = root.join("dotori");
    let work = dotori.join("work");
    let hidden = root.join(".secret").join("keys");
    let record = dotori.join("record");
    std::fs::create_dir_all(&work).expect("made");
    std::fs::create_dir_all(&hidden).expect("made");
    std::fs::create_dir_all(&record).expect("made");
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .expect("a home folder");
    let top = home.ancestors().last().expect("a root").to_path_buf();

    for refused in [hidden, dotori, record, home, top] {
        let folder = working_folder(&work, asked(&refused).as_deref());
        assert!(
            folder
                .as_ref()
                .is_err_and(|error| error.contains("not a folder a sub-session may work in")),
            "{refused:?} gave {folder:?}"
        );
    }
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_verbatim_windows_path_is_worked_in_as_a_plain_one() {
    assert_eq!(
        plain(PathBuf::from(r"\\?\C:\Users\me\project")),
        PathBuf::from(r"C:\Users\me\project")
    );
    assert_eq!(
        plain(PathBuf::from(r"\\?\UNC\server\share")),
        PathBuf::from(r"\\?\UNC\server\share")
    );
    assert_eq!(
        plain(PathBuf::from("/home/me/project")),
        PathBuf::from("/home/me/project")
    );
}
