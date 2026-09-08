use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::io;
use std::os::unix::fs::DirBuilderExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use landlock::{
    ABI, Access, AccessFs, CompatLevel, Compatible, Ruleset, RulesetAttr, RulesetCreatedAttr,
    RulesetStatus, path_beneath_rules,
};

pub const HELPER_ARG: &str = "__catdesk_landlock_exec";
const BUBBLEWRAP_EXECUTABLE: &str = "bwrap";

// ABI v3 is the minimum safe baseline for filesystem confinement because it
// adds control over truncate(2). Older ABIs could otherwise leave an outside
// file truncatable even when ordinary write opens are denied.
const LANDLOCK_ABI: ABI = ABI::V3;

pub fn is_helper_invocation() -> bool {
    std::env::args_os()
        .nth(1)
        .is_some_and(|arg| arg == OsStr::new(HELPER_ARG))
}

pub fn exec_helper() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os();
    let _program = args.next();
    let helper = args
        .next()
        .ok_or_else(|| io::Error::other("missing Landlock helper marker"))?;
    if helper != OsStr::new(HELPER_ARG) {
        return Err(io::Error::other("invalid Landlock helper invocation").into());
    }

    let workspace = PathBuf::from(
        args.next()
            .ok_or_else(|| io::Error::other("missing Landlock workspace path"))?,
    );
    let scratch = PathBuf::from(
        args.next()
            .ok_or_else(|| io::Error::other("missing Landlock scratch path"))?,
    );
    let command = args
        .next()
        .ok_or_else(|| io::Error::other("missing Landlock shell command"))?;
    if args.next().is_some() {
        return Err(io::Error::other("unexpected Landlock helper arguments").into());
    }

    apply_workspace_landlock(&workspace, &scratch)?;

    let error = Command::new("/bin/bash")
        .arg("-c")
        .arg(command)
        .env("TMPDIR", &scratch)
        .env("TMP", &scratch)
        .env("TEMP", &scratch)
        .exec();
    Err(io::Error::new(
        error.kind(),
        format!("failed to exec /bin/bash after applying Landlock: {error}"),
    )
    .into())
}

fn canonical_existing(path: &Path) -> io::Result<PathBuf> {
    path.canonicalize().map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("failed to canonicalize {}: {error}", path.display()),
        )
    })
}

fn insert_existing(paths: &mut BTreeSet<PathBuf>, path: impl AsRef<Path>) {
    let path = path.as_ref();
    if let Ok(canonical) = path.canonicalize() {
        paths.insert(canonical);
    }
}

fn insert_env_path_list(paths: &mut BTreeSet<PathBuf>, variable: &str) {
    let Some(value) = std::env::var_os(variable) else {
        return;
    };
    for path in std::env::split_paths(&value) {
        insert_existing(paths, path);
    }
}

fn insert_env_path(paths: &mut BTreeSet<PathBuf>, variable: &str) {
    if let Some(path) = std::env::var_os(variable) {
        insert_existing(paths, PathBuf::from(path));
    }
}

fn runtime_read_paths() -> BTreeSet<PathBuf> {
    let mut paths = BTreeSet::new();

    for path in ["/bin", "/sbin", "/usr", "/lib", "/lib64", "/etc", "/sys"] {
        insert_existing(&mut paths, path);
    }

    insert_existing(&mut paths, "/etc/resolv.conf");

    // Executables installed outside the standard system prefixes must remain
    // executable when their directory is explicitly present in PATH.
    insert_env_path_list(&mut paths, "PATH");

    // Rust toolchains are commonly installed under the user's home directory.
    // Expose only executable/cache trees from Cargo so registry credentials
    // remain outside the sandbox. Rustup does not store registry credentials.
    if let Some(cargo_home) = std::env::var_os("CARGO_HOME") {
        let cargo_home = PathBuf::from(cargo_home);
        insert_existing(&mut paths, cargo_home.join("bin"));
        insert_existing(&mut paths, cargo_home.join("registry"));
        insert_existing(&mut paths, cargo_home.join("git"));
    }
    insert_env_path(&mut paths, "RUSTUP_HOME");
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        let cargo_home = home.join(".cargo");
        insert_existing(&mut paths, cargo_home.join("bin"));
        insert_existing(&mut paths, cargo_home.join("registry"));
        insert_existing(&mut paths, cargo_home.join("git"));
        insert_existing(&mut paths, home.join(".rustup"));

        // Git treats an unreadable global config as fatal. Grant only the
        // configuration files, keeping credential stores and the rest of HOME
        // inaccessible.
        insert_existing(&mut paths, home.join(".gitconfig"));
        insert_existing(&mut paths, home.join(".config/git/config"));
        insert_existing(&mut paths, home.join(".git-credentials"));
        insert_existing(&mut paths, home.join(".config/gh/hosts.yml"));
        insert_existing(&mut paths, home.join(".ssh/known_hosts"));
    }

    paths
}

fn runtime_write_paths() -> BTreeSet<PathBuf> {
    let mut paths = BTreeSet::new();

    for path in [
        "/dev/null",
        "/dev/zero",
        "/dev/full",
        "/dev/random",
        "/dev/urandom",
        "/dev/tty",
    ] {
        insert_existing(&mut paths, path);
    }

    paths
}

fn validate_ruleset_status(status: RulesetStatus) -> io::Result<()> {
    match status {
        RulesetStatus::FullyEnforced => Ok(()),
        RulesetStatus::PartiallyEnforced => Err(io::Error::other(
            "Landlock sandbox was only partially enforced",
        )),
        RulesetStatus::NotEnforced => Err(io::Error::other(
            "Landlock sandbox is unavailable and CatDesk refuses unsandboxed Linux command execution",
        )),
    }
}

pub fn apply_workspace_landlock(
    workspace: &Path,
    scratch: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let workspace = canonical_existing(workspace)?;
    if !workspace.is_dir() {
        return Err(io::Error::other(format!(
            "Landlock workspace is not a directory: {}",
            workspace.display()
        ))
        .into());
    }
    let scratch = canonical_existing(scratch)?;
    if !scratch.is_dir() {
        return Err(io::Error::other(format!(
            "Landlock scratch path is not a directory: {}",
            scratch.display()
        ))
        .into());
    }

    let access_all = AccessFs::from_all(LANDLOCK_ABI);
    let access_read = AccessFs::from_read(LANDLOCK_ABI);

    let read_paths = runtime_read_paths();
    let mut write_paths = runtime_write_paths();
    write_paths.insert(workspace);
    write_paths.insert(scratch);

    let ruleset = Ruleset::default()
        .set_compatibility(CompatLevel::BestEffort)
        .handle_access(access_all)?
        .create()?
        .add_rules(path_beneath_rules(&read_paths, access_read))?
        .add_rules(path_beneath_rules(&write_paths, access_all))?
        .no_new_privs(true)
        .restrict_self()?;

    validate_ruleset_status(ruleset.ruleset)?;
    Ok(())
}

fn executable_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|directory| directory.join(name))
        .find(|candidate| candidate.is_file())
}

fn create_private_scratch_dir() -> io::Result<PathBuf> {
    let scratch_dir =
        std::env::temp_dir().join(format!("catdesk-sandbox-{}", uuid::Uuid::new_v4()));
    let mut dir_builder = std::fs::DirBuilder::new();
    dir_builder
        .mode(0o700)
        .create(&scratch_dir)
        .map_err(|error| {
            io::Error::new(
                error.kind(),
                format!(
                    "failed to create sandbox scratch directory {}: {error}",
                    scratch_dir.display()
                ),
            )
        })?;
    Ok(scratch_dir)
}

fn system_runtime_root(path: &Path) -> bool {
    ["/bin", "/sbin", "/usr", "/lib", "/lib64", "/etc", "/sys"]
        .iter()
        .map(Path::new)
        .any(|root| path == root || path.starts_with(root))
}

fn add_parent_directories(helper: &mut Command, path: &Path, created: &mut BTreeSet<PathBuf>) {
    let mut parents = Vec::new();
    let mut current = path.parent();
    while let Some(parent) = current {
        if parent == Path::new("/") {
            break;
        }
        parents.push(parent.to_path_buf());
        current = parent.parent();
    }
    parents.reverse();
    for parent in parents {
        if created.insert(parent.clone()) {
            helper.arg("--dir").arg(parent);
        }
    }
}

fn bubblewrap_command(
    bwrap: &Path,
    command: &str,
    workspace: &Path,
    cwd: &Path,
    scratch_dir: &Path,
) -> io::Result<Command> {
    let workspace = canonical_existing(workspace)?;
    let cwd = canonical_existing(cwd)?;
    if !cwd.starts_with(&workspace) {
        return Err(io::Error::other(format!(
            "Bubblewrap cwd escapes workspace: {}",
            cwd.display()
        )));
    }

    let mut helper = Command::new(bwrap);
    helper.arg("--new-session").arg("--unshare-pid");

    for path in ["/usr", "/bin", "/sbin", "/lib", "/lib64", "/etc", "/sys"] {
        let path = Path::new(path);
        if path.exists() {
            helper.arg("--ro-bind").arg(path).arg(path);
        }
    }

    helper
        .arg("--proc")
        .arg("/proc")
        .arg("--dev")
        .arg("/dev")
        .arg("--tmpfs")
        .arg("/tmp");

    let mut created_dirs = BTreeSet::new();
    for path in runtime_read_paths() {
        if system_runtime_root(&path) || path.starts_with(&workspace) {
            continue;
        }
        add_parent_directories(&mut helper, &path, &mut created_dirs);
        helper.arg("--ro-bind").arg(&path).arg(&path);
    }

    add_parent_directories(&mut helper, &workspace, &mut created_dirs);
    helper.arg("--bind").arg(&workspace).arg(&workspace);

    add_parent_directories(&mut helper, scratch_dir, &mut created_dirs);
    helper
        .arg("--bind")
        .arg(scratch_dir)
        .arg(scratch_dir)
        .arg("--chdir")
        .arg(&cwd)
        .arg("--setenv")
        .arg("TMPDIR")
        .arg(scratch_dir)
        .arg("--setenv")
        .arg("TMP")
        .arg(scratch_dir)
        .arg("--setenv")
        .arg("TEMP")
        .arg(scratch_dir)
        .arg("/bin/bash")
        .arg("-c")
        .arg(command);

    Ok(helper)
}

pub fn helper_command(
    command: &str,
    workspace: &Path,
    cwd: &Path,
) -> io::Result<(Command, PathBuf)> {
    let scratch_dir = create_private_scratch_dir()?;

    if let Some(bwrap) = executable_on_path(BUBBLEWRAP_EXECUTABLE) {
        match bubblewrap_command(&bwrap, command, workspace, cwd, &scratch_dir) {
            Ok(helper) => return Ok((helper, scratch_dir)),
            Err(error) => {
                let _ = std::fs::remove_dir_all(&scratch_dir);
                return Err(error);
            }
        }
    }

    let executable = std::env::current_exe().map_err(|error| {
        let _ = std::fs::remove_dir_all(&scratch_dir);
        io::Error::new(
            error.kind(),
            format!("failed to locate CatDesk executable for Landlock helper: {error}"),
        )
    })?;

    let mut helper = Command::new(executable);
    helper
        .arg(HELPER_ARG)
        .arg(workspace)
        .arg(&scratch_dir)
        .arg(command);
    Ok((helper, scratch_dir))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_read_paths_include_resolv_conf_target() {
        let resolv_conf = Path::new("/etc/resolv.conf")
            .canonicalize()
            .expect("canonical /etc/resolv.conf");
        assert!(runtime_read_paths().contains(&resolv_conf));
    }

    #[test]
    fn runtime_read_paths_include_git_credentials_without_exposing_home() {
        let Some(home) = std::env::var_os("HOME") else {
            return;
        };
        let home = PathBuf::from(home).canonicalize().expect("canonical HOME");
        let paths = runtime_read_paths();

        for credential in [
            home.join(".git-credentials"),
            home.join(".config/gh/hosts.yml"),
        ] {
            if credential.exists() {
                assert!(
                    paths.contains(
                        &credential
                            .canonicalize()
                            .expect("canonical credential file")
                    ),
                    "missing credential file: {}",
                    credential.display()
                );
            }
        }
        assert!(!paths.contains(&home));
        assert!(!paths.contains(&home.join(".config")));
        assert!(!paths.contains(&home.join(".config/gh")));
    }

    #[test]
    fn runtime_read_paths_include_ssh_known_hosts_target() {
        let Some(home) = std::env::var_os("HOME") else {
            return;
        };
        let known_hosts = PathBuf::from(home).join(".ssh/known_hosts");
        let Ok(known_hosts) = known_hosts.canonicalize() else {
            return;
        };
        assert!(runtime_read_paths().contains(&known_hosts));
    }

    #[test]
    fn runtime_read_paths_do_not_grant_the_home_directory_itself() {
        let Some(home) = std::env::var_os("HOME") else {
            return;
        };
        let home = PathBuf::from(home).canonicalize().expect("canonical HOME");
        assert!(!runtime_read_paths().contains(&home));
    }

    #[test]
    fn helper_marker_detection_is_exact() {
        assert_ne!(HELPER_ARG, "");
        assert!(!HELPER_ARG.contains(char::is_whitespace));
    }

    #[test]
    fn bubblewrap_does_not_bind_lifetime_to_spawn_blocking_worker_thread() {
        let Some(bwrap) = executable_on_path("bwrap") else {
            return;
        };
        let workspace =
            std::env::temp_dir().join(format!("catdesk-bwrap-lifetime-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&workspace).expect("create workspace");

        let (command, scratch) =
            helper_command("true", &workspace, &workspace).expect("prepare bubblewrap command");
        let args = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();

        assert_eq!(PathBuf::from(command.get_program()), bwrap);
        assert!(
            !args.iter().any(|arg| arg == "--die-with-parent"),
            "--die-with-parent kills long-lived jobs when Tokio retires the spawn_blocking worker thread"
        );

        std::fs::remove_dir_all(scratch).expect("remove scratch directory");
        std::fs::remove_dir_all(workspace).expect("remove workspace");
    }

    #[test]
    fn helper_command_prefers_bubblewrap_when_available() {
        let Some(bwrap) = executable_on_path("bwrap") else {
            return;
        };
        let workspace =
            std::env::temp_dir().join(format!("catdesk-bwrap-workspace-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&workspace).expect("create workspace");

        let (command, scratch) =
            helper_command("true", &workspace, &workspace).expect("prepare sandbox command");

        assert_eq!(PathBuf::from(command.get_program()), bwrap);
        std::fs::remove_dir_all(scratch).expect("remove scratch directory");
        std::fs::remove_dir_all(workspace).expect("remove workspace");
    }

    #[test]
    fn bubblewrap_hides_host_tmp_and_keeps_workspace_writable() {
        if executable_on_path(BUBBLEWRAP_EXECUTABLE).is_none() {
            return;
        }

        let workspace = std::env::temp_dir().join(format!(
            "catdesk-bwrap-integration-workspace-{}",
            uuid::Uuid::new_v4()
        ));
        let outside = std::env::temp_dir().join(format!(
            "catdesk-bwrap-outside-secret-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&workspace).expect("create workspace");
        std::fs::write(&outside, "secret").expect("create outside secret");

        let marker = workspace.join("sandbox-marker.txt");
        let shell = format!(
            "test ! -e '{}' && printf sandboxed > '{}'",
            outside.display(),
            marker.display()
        );
        let (mut command, scratch) =
            helper_command(&shell, &workspace, &workspace).expect("prepare bubblewrap command");
        let status = command.status().expect("execute bubblewrap command");

        assert!(status.success(), "bubblewrap command failed: {status}");
        assert_eq!(
            std::fs::read_to_string(&marker).expect("read workspace marker"),
            "sandboxed"
        );

        let _ = std::fs::remove_file(outside);
        let _ = std::fs::remove_dir_all(scratch);
        let _ = std::fs::remove_dir_all(workspace);
    }

    #[test]
    fn unavailable_landlock_is_never_allowed_unsandboxed() {
        let error = validate_ruleset_status(RulesetStatus::NotEnforced)
            .expect_err("unavailable Landlock must fail closed");
        assert!(error.to_string().contains("unavailable"));
    }

    #[test]
    fn helper_command_creates_private_scratch_directory() {
        use std::os::unix::fs::PermissionsExt;

        let (_command, scratch) = helper_command("true", Path::new("."), Path::new("."))
            .expect("prepare sandbox command");
        let mode = std::fs::metadata(&scratch)
            .expect("scratch metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o700);
        std::fs::remove_dir_all(scratch).expect("remove scratch directory");
    }

    #[test]
    fn runtime_write_paths_do_not_grant_global_tmp() {
        let tmp = Path::new("/tmp").canonicalize().expect("canonical /tmp");
        assert!(!runtime_write_paths().contains(&tmp));
    }

    #[test]
    fn fully_enforced_ruleset_is_accepted() {
        validate_ruleset_status(RulesetStatus::FullyEnforced).expect("fully enforced ruleset");
    }

    #[test]
    fn partially_enforced_ruleset_is_rejected() {
        let error = validate_ruleset_status(RulesetStatus::PartiallyEnforced)
            .expect_err("partially enforced ruleset must be rejected");
        assert!(error.to_string().contains("partially enforced"));
    }

    #[test]
    fn unavailable_ruleset_is_rejected() {
        let error = validate_ruleset_status(RulesetStatus::NotEnforced)
            .expect_err("unavailable Landlock must be rejected");
        assert!(error.to_string().contains("refuses unsandboxed"));
    }
}
