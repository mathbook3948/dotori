//! `dotori sandbox install`, which sets Linux up for the sandbox the shell commands of dotori's
//! model run in: bubblewrap, and on a system that keeps user namespaces from ordinary programs, a
//! copy of it at a fixed path with an AppArmor profile that lets that copy make them. It runs as
//! root, from the settings window through `pkexec` or by hand with `sudo`.

use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::ExitCode;
use std::process::Stdio;

use dotori_connectors::sandbox::HELPER;
use dotori_connectors::sandbox::root_owned;

const PROFILE_PATH: &str = "/etc/apparmor.d/dotori-bwrap";
const RESTRICTED: &str = "/proc/sys/kernel/apparmor_restrict_unprivileged_userns";
const NAMESPACES: [&str; 2] = [
    "/proc/sys/user/max_user_namespaces",
    "/proc/sys/kernel/unprivileged_userns_clone",
];
const MANAGERS: [(&str, &[&str]); 4] = [
    ("apt-get", &["install", "-y", "bubblewrap"]),
    ("dnf", &["install", "-y", "bubblewrap"]),
    ("zypper", &["--non-interactive", "install", "bubblewrap"]),
    ("pacman", &["-S", "--noconfirm", "--needed", "bubblewrap"]),
];

/// Why the sandbox could not be set up. `dotori sandbox install` ends with the code of its
/// failure, so the settings window can say what went wrong.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Failure {
    /// The owner did not allow it in the admin prompt.
    Refused,
    /// No admin prompt could be shown, as `pkexec` is not there.
    NoPrompt,
    /// bubblewrap is not there and could not be installed.
    NoBubblewrap,
    /// The AppArmor profile for dotori's copy of bubblewrap could not be loaded.
    NoAppArmor,
    /// The kernel lets no ordinary program make a user namespace.
    NoNamespaces,
    /// Everything was installed, but a sandboxed command still does not run.
    StillOff,
    /// dotori's program, or a folder above it, can be changed by someone other than root, so
    /// running it as root through the admin prompt would run whatever was put there.
    UnsafePlace,
    /// Something else, such as not running as root.
    Other,
}

const CODES: [(Failure, u8); 4] = [
    (Failure::NoBubblewrap, 3),
    (Failure::NoAppArmor, 4),
    (Failure::NoNamespaces, 5),
    (Failure::Other, 1),
];

impl Failure {
    /// What came of `dotori sandbox install` that ended with `code`, run through `pkexec`, which
    /// ends with 126 when the owner dismissed its prompt and 127 when it was not allowed.
    pub(crate) fn of_exit(code: Option<i32>) -> Self {
        match code {
            Some(126 | 127) => Self::Refused,
            Some(code) => CODES
                .iter()
                .find(|(_, known)| i32::from(*known) == code)
                .map_or(Self::Other, |(failure, _)| *failure),
            None => Self::Other,
        }
    }

    fn code(self) -> u8 {
        CODES
            .iter()
            .find(|(failure, _)| *failure == self)
            .map_or(1, |(_, code)| *code)
    }

    /// What the settings window says about it.
    pub(crate) fn sentence(self) -> &'static str {
        match self {
            Self::Refused => "관리자 확인을 받지 못했어요.",
            Self::NoPrompt => "관리자 확인 창을 띄울 수 없었어요.",
            Self::NoBubblewrap => "bubblewrap을 설치하지 못했어요.",
            Self::NoAppArmor => "AppArmor에 샌드박스를 허락하는 설정을 넣지 못했어요.",
            Self::NoNamespaces => {
                "이 PC는 사용자 네임스페이스가 꺼져 있어서 샌드박스를 만들 수 없어요."
            }
            Self::StillOff => "설치는 끝났지만 샌드박스가 아직 돌지 않아요.",
            Self::UnsafePlace => {
                "dotori 실행 파일이나 그 위 폴더를 관리자가 아닌 계정도 바꿀 수 있어서, 관리자 확인 창을 띄우지 않았어요."
            }
            Self::Other => "설치하는 중에 문제가 생겼어요.",
        }
    }
}

/// The command that sets the sandbox up, for the owner to run in a terminal.
pub(crate) fn by_hand() -> String {
    let program = std::env::current_exe()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|_| String::from("dotori"));
    format!("sudo '{}' sandbox install", program.replace('\'', r"'\''"))
}

/// Whether setting the sandbox up needs the admin prompt, as dotori does not run as root.
pub(crate) fn needs_admin() -> bool {
    #[cfg(target_os = "linux")]
    let root = rustix::process::geteuid().is_root();
    #[cfg(not(target_os = "linux"))]
    let root = false;
    !root
}

/// Runs `dotori sandbox install` as root: through `pkexec` unless dotori is root already. It
/// waits until that ends.
pub(crate) fn set_up() -> Result<(), Failure> {
    let program = std::env::current_exe().map_err(|_| Failure::Other)?;
    if needs_admin() && !root_owned(&program) {
        return Err(Failure::UnsafePlace);
    }
    let mut command = if needs_admin() {
        let mut command = Command::new("pkexec");
        command.arg(program);
        command
    } else {
        Command::new(program)
    };
    let ended = command
        .args(["sandbox", "install"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .status();
    match ended {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(Failure::of_exit(status.code())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Err(Failure::NoPrompt),
        Err(_) => Err(Failure::Other),
    }
}

/// `dotori sandbox <what>`.
pub(crate) fn run(what: Option<&str>) -> ExitCode {
    if what != Some("install") {
        eprintln!("usage: dotori sandbox install");
        return ExitCode::FAILURE;
    }
    match install() {
        Ok(done) => {
            println!("dotori: {done}");
            ExitCode::SUCCESS
        }
        Err((failure, why)) => {
            eprintln!("dotori: the sandbox is not set up: {why}");
            ExitCode::from(failure.code())
        }
    }
}

fn install() -> Result<String, (Failure, String)> {
    if !cfg!(target_os = "linux") {
        return Err((
            Failure::Other,
            String::from("only Linux needs this; macOS has its sandbox already"),
        ));
    }
    if needs_admin() {
        return Err((Failure::Other, format!("run it as root: {}", by_hand())));
    }
    let bwrap = match program("bwrap") {
        Some(bwrap) => bwrap,
        None => {
            install_bubblewrap()?;
            program("bwrap").ok_or((
                Failure::NoBubblewrap,
                String::from("bubblewrap was installed but bwrap is not on the PATH"),
            ))?
        }
    };
    let off = NAMESPACES
        .iter()
        .any(|setting| read(setting).is_some_and(|value| value == "0"));
    if off {
        return Err((
            Failure::NoNamespaces,
            String::from("the kernel lets no ordinary program make a user namespace"),
        ));
    }
    if read(RESTRICTED).is_none_or(|value| value != "1") {
        return Ok(format!("the sandbox uses {}", bwrap.display()));
    }
    let apparmor = |why: String| (Failure::NoAppArmor, why);
    let helper = Path::new(HELPER);
    if let Some(folder) = helper.parent() {
        std::fs::create_dir_all(folder)
            .map_err(|error| apparmor(format!("{}: {error}", folder.display())))?;
    }
    let group = owner_group().map_err(apparmor)?;
    let staged = helper.with_extension("new");
    std::fs::copy(&bwrap, &staged)
        .and_then(|_| owned_program(&staged, group))
        .and_then(|()| std::fs::rename(&staged, helper))
        .map_err(|error| apparmor(format!("{HELPER}: {error}")))?;
    std::fs::write(PROFILE_PATH, profile())
        .map_err(|error| apparmor(format!("{PROFILE_PATH}: {error}")))?;
    let parser = program("apparmor_parser")
        .ok_or_else(|| apparmor(String::from("apparmor_parser is not there")))?;
    let loaded = Command::new(parser)
        .args(["-r", PROFILE_PATH])
        .stdin(Stdio::null())
        .status()
        .map_err(|error| apparmor(error.to_string()))?;
    if !loaded.success() {
        return Err(apparmor(format!("apparmor_parser ended with {loaded}")));
    }
    Ok(format!(
        "the sandbox uses {HELPER}, which {PROFILE_PATH} lets make user namespaces"
    ))
}

fn install_bubblewrap() -> Result<(), (Failure, String)> {
    let Some((manager, args)) = MANAGERS
        .iter()
        .find_map(|(name, args)| program(name).map(|found| (found, *args)))
    else {
        return Err((
            Failure::NoBubblewrap,
            String::from("no package manager dotori knows is here; install bubblewrap"),
        ));
    };
    let installed = Command::new(&manager)
        .args(args)
        .env("DEBIAN_FRONTEND", "noninteractive")
        .stdin(Stdio::null())
        .status()
        .map_err(|error| (Failure::NoBubblewrap, error.to_string()))?;
    if installed.success() {
        Ok(())
    } else {
        Err((
            Failure::NoBubblewrap,
            format!("{} ended with {installed}", manager.display()),
        ))
    }
}

/// The AppArmor profile for dotori's copy of bubblewrap, shaped like Ubuntu's
/// `bwrap-userns-restrict`: the copy may make a user namespace and set the sandbox up in it, and
/// every program it starts moves to a profile with no capabilities and no user namespaces.
pub(crate) fn profile() -> String {
    let rules = "  allow network,\n  allow unix,\n  allow ptrace,\n  allow signal,\n  \
                 allow mqueue,\n  allow io_uring,\n  allow mount,\n  allow umount,\n  \
                 allow pivot_root,\n  allow dbus,\n";
    format!(
        "abi <abi/4.0>,\n\
         include <tunables/global>\n\
         \n\
         profile dotori-bwrap {HELPER} flags=(attach_disconnected,mediate_deleted) {{\n  \
         allow capability,\n  \
         allow file rwlkm /{{**,}},\n\
         {rules}  \
         allow userns,\n  \
         allow px /** -> dotori-bwrap-child,\n\
         \n  \
         include if exists <local/dotori-bwrap>\n\
         }}\n\
         \n\
         profile dotori-bwrap-child flags=(attach_disconnected,mediate_deleted) {{\n  \
         allow file rwlkmix /{{**,}},\n\
         {rules}  \
         deny capability,\n  \
         audit deny userns,\n\
         \n  \
         include if exists <local/dotori-bwrap-child>\n\
         }}\n"
    )
}

fn owner_group() -> Result<u32, String> {
    let Some(owner) = ["PKEXEC_UID", "SUDO_UID"]
        .iter()
        .find_map(|name| std::env::var(name).ok())
    else {
        return Ok(0);
    };
    let found = Command::new("id")
        .args(["-g", owner.trim()])
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("cannot find the group of user {owner}: {error}"))?;
    String::from_utf8_lossy(&found.stdout)
        .trim()
        .parse()
        .map_err(|_| format!("cannot find the group of user {owner}"))
}

#[cfg(unix)]
fn owned_program(path: &Path, group: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::os::unix::fs::chown(path, Some(0), Some(group))?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o750))
}

#[cfg(not(unix))]
fn owned_program(_path: &Path, _group: u32) -> std::io::Result<()> {
    Ok(())
}

fn program(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .chain(["/usr/sbin", "/usr/bin", "/sbin", "/bin"].map(PathBuf::from))
        .map(|folder| folder.join(name))
        .find(|path| path.is_file())
}

fn read(path: &str) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|value| value.trim().to_owned())
}

#[cfg(test)]
#[path = "sandbox.test.rs"]
mod tests;
