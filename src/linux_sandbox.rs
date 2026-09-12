use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::io;
use std::os::unix::fs::{DirBuilderExt, FileTypeExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use landlock::{
    ABI, Access, AccessFs, CompatLevel, Compatible, Ruleset, RulesetAttr, RulesetCreatedAttr,
    RulesetStatus, path_beneath_rules,
};

pub const HELPER_ARG: &str = "__catdesk_landlock_exec";

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

fn insert_ssh_read_paths(paths: &mut BTreeSet<PathBuf>, home: &Path) {
    let ssh_dir = home.join(".ssh");
    // Only regular metadata/public-key files are exposed. A *.pub directory
    // or symlink to a private key must not broaden the read allowlist.
    let mut insert_regular = |path: PathBuf| {
        if std::fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.is_file()) {
            insert_existing(paths, path);
        }
    };
    for name in ["config", "known_hosts", "known_hosts2"] {
        insert_regular(ssh_dir.join(name));
    }
    if let Ok(entries) = std::fs::read_dir(&ssh_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|extension| extension == "pub") {
                insert_regular(path);
            }
        }
    }
}

fn existing_unix_socket(path: &Path) -> Option<PathBuf> {
    let canonical = path.canonicalize().ok()?;
    std::fs::metadata(&canonical)
        .ok()?
        .file_type()
        .is_socket()
        .then_some(canonical)
}

fn ssh_agent_socket() -> Option<PathBuf> {
    let path = PathBuf::from(std::env::var_os("SSH_AUTH_SOCK")?);
    existing_unix_socket(&path)
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

        // Preserve the fork's explicit HTTPS Git credential files without
        // granting the rest of HOME. Read-only credentials remain sensitive;
        // the sandbox does not make untrusted commands safe to give credentials.
        insert_existing(&mut paths, home.join(".gitconfig"));
        insert_existing(&mut paths, home.join(".config/git/config"));
        insert_existing(&mut paths, home.join(".git-credentials"));
        insert_existing(&mut paths, home.join(".config/gh/hosts.yml"));
        insert_ssh_read_paths(&mut paths, &home);
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

fn bubblewrap_executable_in_paths(
    paths: impl IntoIterator<Item = PathBuf>,
    workspace: &Path,
) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;

    let workspace = workspace.canonicalize().ok()?;
    paths
        .into_iter()
        .map(|dir| dir.join("bwrap"))
        .find_map(|candidate| {
            let metadata = std::fs::metadata(&candidate).ok()?;
            if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 {
                return None;
            }
            let canonical = candidate.canonicalize().ok()?;
            if canonical.starts_with(&workspace) {
                None
            } else {
                Some(canonical)
            }
        })
}

fn bubblewrap_executable(workspace: &Path) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    bubblewrap_executable_in_paths(std::env::split_paths(&path), workspace)
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

fn bubblewrap_command(
    bwrap: &Path,
    command: &str,
    workspace: &Path,
    cwd: &Path,
    scratch: &Path,
) -> io::Result<Command> {
    let workspace = canonical_existing(workspace)?;
    let cwd = canonical_existing(cwd)?;
    let scratch = canonical_existing(scratch)?;
    if !workspace.is_dir() || !cwd.is_dir() || !scratch.is_dir() {
        return Err(io::Error::other(
            "Sandbox workspace, cwd and scratch must be directories",
        ));
    }
    if !cwd.starts_with(&workspace) {
        return Err(io::Error::other(format!(
            "Bubblewrap cwd escapes workspace: {}",
            cwd.display()
        )));
    }
    // A persistent shell owns bwrap until completion. Binding bwrap directly
    // to a Tokio spawn_blocking thread would kill long jobs when that thread
    // retires; omitting parent-death handling leaves the PID namespace alive
    // after ProcessTreeGuard kills its monitor. This stable owner supports both.
    // -p disables imported functions and BASH_ENV/SHELLOPTS startup behavior in
    // the host-side parent. All command/path arguments are forwarded literally.
    // Keep the trailing exit: replacing this script with exec removes the owner.
    let mut bwrap_command = Command::new("/bin/bash");
    bwrap_command
        .args([
            "--noprofile",
            "--norc",
            "-p",
            "-c",
            r#""$@"; exit "$?""#,
            "catdesk-bwrap-parent",
        ])
        .arg(bwrap)
        .arg("--die-with-parent")
        .arg("--unshare-user")
        .arg("--unshare-pid")
        .arg("--unshare-ipc")
        .arg("--unshare-uts")
        .arg("--new-session")
        .arg("--proc")
        .arg("/proc")
        .arg("--dev")
        .arg("/dev")
        .arg("--tmpfs")
        .arg("/tmp");

    for path in runtime_read_paths() {
        if path.starts_with(&workspace) {
            continue;
        }
        bwrap_command.arg("--ro-bind-try").arg(&path).arg(&path);
    }

    // Root-owned SSH client config appears as uid 65534 inside the unprivileged
    // user namespace, which OpenSSH rejects before authentication. Hide the
    // system config and let OpenSSH use the read-only user config/defaults.
    if Path::new("/etc/ssh").is_dir() {
        bwrap_command.arg("--tmpfs").arg("/etc/ssh");
    }

    let ssh_agent_socket = ssh_agent_socket();
    if let Some(socket) = &ssh_agent_socket {
        // Forward only the agent socket. Private key files remain outside the
        // sandbox while Git/SSH can authenticate and perform SSH signing.
        bwrap_command.arg("--bind").arg(socket).arg(socket);
    }

    // Replicate merged-/usr symlinks. runtime_read_paths canonicalises, so on
    // distributions where /bin, /sbin, /lib and /lib64 are symlinks into /usr
    // it yields only the /usr targets. Bubblewrap builds a fresh namespace:
    // without these links /bin/bash does not exist and every sandboxed command
    // fails with "execvp /bin/bash: No such file or directory".
    for link in ["/bin", "/sbin", "/lib", "/lib64"] {
        let link = Path::new(link);
        if let Ok(target) = std::fs::read_link(link) {
            bwrap_command.arg("--symlink").arg(target).arg(link);
        }
    }

    for path in [&workspace, &scratch] {
        bwrap_command.arg("--bind").arg(path).arg(path);
    }

    bwrap_command.arg("--chdir").arg(&cwd);
    if let Some(socket) = &ssh_agent_socket {
        bwrap_command
            .arg("--setenv")
            .arg("SSH_AUTH_SOCK")
            .arg(socket);
    } else {
        bwrap_command.arg("--unsetenv").arg("SSH_AUTH_SOCK");
    }
    bwrap_command
        .arg("--setenv")
        .arg("TMPDIR")
        .arg(&scratch)
        .arg("--setenv")
        .arg("TMP")
        .arg(&scratch)
        .arg("--setenv")
        .arg("TEMP")
        .arg(&scratch)
        .arg("/bin/bash")
        .arg("-c")
        .arg(command);

    Ok(bwrap_command)
}

pub fn helper_command(
    command: &str,
    workspace: &Path,
    cwd: &Path,
) -> io::Result<(Command, PathBuf)> {
    let workspace = canonical_existing(workspace)?;
    let cwd = canonical_existing(cwd)?;
    if !workspace.is_dir() || !cwd.is_dir() || !cwd.starts_with(&workspace) {
        return Err(io::Error::other(
            "Sandbox cwd must be a directory inside the workspace",
        ));
    }
    let scratch_dir = create_private_scratch_dir()?;
    if let Some(bwrap) = bubblewrap_executable(&workspace) {
        match bubblewrap_command(&bwrap, command, &workspace, &cwd, &scratch_dir) {
            Ok(helper) => return Ok((helper, scratch_dir)),
            Err(error) => {
                let _ = std::fs::remove_dir_all(&scratch_dir);
                return Err(error);
            }
        }
    }

    // Retain the explicitly requested Landlock backend only when no trusted
    // bwrap is available. A failed bwrap command never retries unconfined.
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
        .arg(&workspace)
        .arg(&scratch_dir)
        .arg(command)
        .current_dir(&cwd);
    Ok((helper, scratch_dir))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct SandboxFixture(PathBuf);

    impl SandboxFixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "cd-{}",
                &uuid::Uuid::new_v4().simple().to_string()[..8]
            ));
            std::fs::create_dir_all(root.join("workspace")).expect("create fixture");
            std::fs::create_dir_all(root.join("home/.ssh")).expect("create SSH fixture");
            Self(root.canonicalize().expect("canonical fixture"))
        }

        fn probe(&self, case: &str, path: &OsStr, socket: Option<&Path>) {
            let mut command = Command::new(std::env::current_exe().expect("test binary"));
            command
                .args([
                    "--exact",
                    "linux_sandbox::tests::sandbox_configuration_probe",
                    "--nocapture",
                ])
                .env("CATDESK_SANDBOX_TEST_CASE", case)
                .env("CATDESK_SANDBOX_TEST_ROOT", &self.0)
                .env("HOME", self.0.join("home"))
                .env("PATH", path)
                .env_remove("SSH_AUTH_SOCK");
            if let Some(socket) = socket {
                command.env("SSH_AUTH_SOCK", socket);
            }
            let result = command.output().expect("run isolated sandbox test");
            assert!(
                result.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&result.stdout),
                String::from_utf8_lossy(&result.stderr)
            );
        }
    }

    impl Drop for SandboxFixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn sandbox_configuration_probe() {
        let Ok(case) = std::env::var("CATDESK_SANDBOX_TEST_CASE") else {
            return;
        };
        let root = PathBuf::from(std::env::var_os("CATDESK_SANDBOX_TEST_ROOT").expect("test root"));
        let workspace = root.join("workspace");
        match case.as_str() {
            "lookup" => {
                let (command, scratch) =
                    helper_command("true", &workspace, &workspace).expect("helper");
                std::fs::remove_dir_all(scratch).expect("cleanup scratch");
                assert_eq!(command.get_program(), OsStr::new("/bin/bash"));
                assert_eq!(
                    command.get_args().nth(6),
                    Some(root.join("trusted/bwrap").as_os_str())
                );
            }
            "ssh" => {
                let paths = runtime_read_paths();
                for name in [
                    ".ssh/config",
                    ".ssh/known_hosts",
                    ".ssh/known_hosts2",
                    ".ssh/id_ed25519.pub",
                    ".git-credentials",
                    ".config/gh/hosts.yml",
                ] {
                    assert!(
                        paths.contains(&root.join("home").join(name)),
                        "missing allowed file: {name}"
                    );
                }
                for name in ["", ".ssh", ".ssh/id_ed25519", ".ssh/key-directory.pub"] {
                    assert!(
                        !paths.contains(&root.join("home").join(name)),
                        "exposed secret/directory: {name}"
                    );
                }
            }
            "socket" => {
                let (command, scratch) =
                    helper_command("true", &workspace, &workspace).expect("helper");
                std::fs::remove_dir_all(scratch).expect("cleanup scratch");
                let args: Vec<_> = command.get_args().map(|a| a.to_os_string()).collect();
                let socket = root.join("agent.sock");
                assert!(args.windows(3).any(|a| a[0] == "--bind"
                    && a[1] == socket.as_os_str()
                    && a[2] == socket.as_os_str()));
                assert!(args.windows(3).any(|a| a[0] == "--setenv"
                    && a[1] == "SSH_AUTH_SOCK"
                    && a[2] == socket.as_os_str()));
            }
            "invalid-socket" => {
                let (command, scratch) =
                    helper_command("true", &workspace, &workspace).expect("helper");
                std::fs::remove_dir_all(scratch).expect("cleanup scratch");
                assert!(
                    !command
                        .get_args()
                        .any(|a| a == root.join("not-a-socket").as_os_str())
                );
            }
            "landlock" => {
                let (command, scratch) =
                    helper_command("true", &workspace, &workspace).expect("Landlock helper");
                std::fs::remove_dir_all(scratch).expect("cleanup scratch");
                assert!(command.get_args().any(|arg| arg == HELPER_ARG));
                assert_eq!(
                    PathBuf::from(command.get_program()),
                    std::env::current_exe().expect("current exe")
                );
            }
            _ => panic!("unknown sandbox test case"),
        }
    }

    #[test]
    fn upstream_sandbox_rejects_workspace_and_nonexecutable_bwrap_candidates() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let fixture = SandboxFixture::new();
        for dir in ["workspace/bin", "alias", "not-executable", "trusted"] {
            std::fs::create_dir_all(fixture.0.join(dir)).expect("create bin dir");
        }
        for dir in ["workspace/bin", "not-executable", "trusted"] {
            let file = fixture.0.join(dir).join("bwrap");
            std::fs::write(&file, "#!/bin/sh\nexit 0\n").expect("write fake executable");
            std::fs::set_permissions(
                &file,
                std::fs::Permissions::from_mode(if dir == "not-executable" {
                    0o644
                } else {
                    0o755
                }),
            )
            .expect("permissions");
        }
        symlink(
            fixture.0.join("workspace/bin/bwrap"),
            fixture.0.join("alias/bwrap"),
        )
        .expect("alias");
        let path = std::env::join_paths(
            ["workspace/bin", "alias", "not-executable", "trusted"].map(|p| fixture.0.join(p)),
        )
        .expect("PATH");
        fixture.probe("lookup", &path, None);
    }

    #[test]
    fn upstream_sandbox_exposes_ssh_metadata_and_preserves_https_credentials() {
        use std::os::unix::fs::symlink;
        let fixture = SandboxFixture::new();
        std::fs::create_dir_all(fixture.0.join("home/.config/gh")).expect("gh fixture");
        for name in [
            ".ssh/config",
            ".ssh/known_hosts",
            ".ssh/known_hosts2",
            ".ssh/id_ed25519.pub",
            ".ssh/id_ed25519",
            ".git-credentials",
            ".config/gh/hosts.yml",
        ] {
            std::fs::write(fixture.0.join("home").join(name), "synthetic test data\n")
                .expect("fixture file");
        }
        std::fs::create_dir(fixture.0.join("home/.ssh/key-directory.pub"))
            .expect("directory must not be exposed");
        symlink("id_ed25519", fixture.0.join("home/.ssh/private-alias.pub"))
            .expect("private key alias");
        fixture.probe("ssh", OsStr::new("/usr/bin:/bin"), None);
    }

    #[test]
    fn upstream_sandbox_forwards_only_a_valid_ssh_agent_socket() {
        use std::os::unix::net::UnixListener;
        let fixture = SandboxFixture::new();
        let socket = fixture.0.join("agent.sock");
        let _listener = UnixListener::bind(&socket).expect("synthetic agent socket");
        fixture.probe("socket", OsStr::new("/usr/bin:/bin"), Some(&socket));
        let regular = fixture.0.join("not-a-socket");
        std::fs::write(&regular, "not a socket").expect("regular fixture");
        fixture.probe(
            "invalid-socket",
            OsStr::new("/usr/bin:/bin"),
            Some(&regular),
        );
    }

    #[test]
    fn upstream_sandbox_uses_extra_namespaces_without_parent_thread_lifetime() {
        let fixture = SandboxFixture::new();
        let workspace = fixture.0.join("workspace");
        let command = bubblewrap_command(
            Path::new("/usr/bin/bwrap"),
            "true",
            &workspace,
            &workspace,
            &fixture.0,
        )
        .expect("command");
        let args: Vec<_> = command.get_args().collect();
        for flag in [
            "--unshare-user",
            "--unshare-pid",
            "--unshare-ipc",
            "--unshare-uts",
            "--new-session",
        ] {
            assert!(
                args.contains(&OsStr::new(flag)),
                "missing namespace flag {flag}"
            );
        }
        assert_eq!(command.get_program(), OsStr::new("/bin/bash"));
        assert!(args.contains(&OsStr::new("--die-with-parent")));
        assert!(!args.contains(&OsStr::new("--unshare-net")));
    }

    #[test]
    fn bubblewrap_survives_spawning_thread_but_not_its_command_owner() {
        use std::process::Stdio;
        use std::time::Duration;

        let fixture = SandboxFixture::new();
        let workspace = fixture.0.join("workspace");
        let marker = workspace.join("orphan.txt");
        let (mut command, scratch) = helper_command(
            "sleep 1; printf survived > orphan.txt",
            &workspace,
            &workspace,
        )
        .expect("prepare sandbox command");
        command
            .process_group(0)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        // Deterministically retire the spawning thread, rather than relying on
        // Tokio's idle-thread timeout. The command must remain owned and alive.
        let mut child = std::thread::spawn(move || command.spawn().expect("spawn sandbox"))
            .join()
            .expect("join spawning thread");
        std::thread::sleep(Duration::from_millis(200));
        let survived_thread = child.try_wait().expect("check command").is_none();
        unsafe {
            libc::kill(-(child.id() as i32), libc::SIGKILL);
        }
        let _ = child.wait();
        std::thread::sleep(Duration::from_millis(1100));
        let orphan_wrote = marker.exists();
        let _ = std::fs::remove_dir_all(scratch);
        assert!(
            survived_thread,
            "command died when its transient spawning thread exited"
        );
        assert!(
            !orphan_wrote,
            "sandbox descendant survived termination of the command owner"
        );
    }

    #[test]
    fn upstream_sandbox_preserves_landlock_when_bwrap_is_absent() {
        let fixture = SandboxFixture::new();
        let empty_bin = fixture.0.join("empty-bin");
        std::fs::create_dir(&empty_bin).expect("empty PATH");
        fixture.probe("landlock", empty_bin.as_os_str(), None);
    }

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
        let Some(bwrap) = bubblewrap_executable(Path::new(".")) else {
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

        assert_eq!(command.get_program(), OsStr::new("/bin/bash"));
        assert_eq!(command.get_args().nth(6), Some(bwrap.as_os_str()));
        assert_eq!(args[4], r#""$@"; exit "$?""#);
        assert_eq!(args[5], "catdesk-bwrap-parent");
        assert!(args.iter().any(|arg| arg == "--die-with-parent"));
        assert!(
            args.iter().any(|arg| arg == "-p"),
            "host parent must not load untrusted shell startup files"
        );

        std::fs::remove_dir_all(scratch).expect("remove scratch directory");
        std::fs::remove_dir_all(workspace).expect("remove workspace");
    }

    #[test]
    fn helper_command_prefers_bubblewrap_when_available() {
        let Some(bwrap) = bubblewrap_executable(Path::new(".")) else {
            return;
        };
        let workspace =
            std::env::temp_dir().join(format!("catdesk-bwrap-workspace-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&workspace).expect("create workspace");

        let (command, scratch) =
            helper_command("true", &workspace, &workspace).expect("prepare sandbox command");

        assert_eq!(command.get_program(), OsStr::new("/bin/bash"));
        assert_eq!(command.get_args().nth(6), Some(bwrap.as_os_str()));
        std::fs::remove_dir_all(scratch).expect("remove scratch directory");
        std::fs::remove_dir_all(workspace).expect("remove workspace");
    }

    #[test]
    fn bubblewrap_hides_host_tmp_and_keeps_workspace_writable() {
        if bubblewrap_executable(Path::new(".")).is_none() {
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
