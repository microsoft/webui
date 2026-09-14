// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::*;

fn symlink(target: &Path, link: &Path, directory: bool) -> io::Result<bool> {
    #[cfg(unix)]
    {
        let _ = directory;
        std::os::unix::fs::symlink(target, link)?;
        Ok(true)
    }
    #[cfg(windows)]
    {
        let result = if directory {
            std::os::windows::fs::symlink_dir(target, link)
        } else {
            std::os::windows::fs::symlink_file(target, link)
        };
        match result {
            Ok(()) => Ok(true),
            Err(error) if error.raw_os_error() == Some(1314) => {
                eprintln!("Symlink creation requires Windows developer mode or elevation");
                Ok(false)
            }
            Err(error) => Err(error),
        }
    }
}

fn remove_directory_link(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        fs::remove_file(path)
    }
    #[cfg(windows)]
    {
        fs::remove_dir(path)
    }
}

#[test]
fn symlinked_component_roots_include_targets_and_retargeting() -> Result<()> {
    let mut fixture = Fixture::new()?;
    let first = fixture.sibling("first");
    let second = fixture.sibling("second");
    fs::create_dir(&first)?;
    fs::create_dir(&second)?;
    fs::write(first.join("external-card.html"), "first")?;
    fs::write(second.join("external-card.html"), "second")?;
    let link = fixture.sibling("workspace-link");
    if !symlink(&first, &link, true)? {
        return Ok(());
    }
    fixture
        .config
        .app_args
        .components
        .push(link.to_string_lossy().into_owned());
    let inputs = fixture.inputs()?;
    let before = snapshot(&inputs)?;
    fs::write(first.join("external-card.html"), "changed")?;
    let changed = snapshot(&inputs)?;
    assert_ne!(before, changed);
    remove_directory_link(&link)?;
    assert!(inputs.capture()?.is_none());
    symlink(&second, &link, true)?;
    assert_ne!(changed, snapshot(&inputs)?);
    Ok(())
}

#[test]
fn symlinked_files_include_contents_and_broken_links_disable_cache() -> Result<()> {
    let fixture = Fixture::new()?;
    let target = fixture.sibling("external.html");
    fs::write(&target, "external")?;
    let link = fixture.config.app_dir.join("linked-card.html");
    if !symlink(&target, &link, false)? {
        return Ok(());
    }
    let inputs = fixture.inputs()?;
    let before = snapshot(&inputs)?;
    fs::write(&target, "changed")?;
    assert_ne!(before, snapshot(&inputs)?);
    fs::remove_file(&target)?;
    assert!(inputs.capture()?.is_none());
    Ok(())
}

#[test]
fn directory_symlink_cycles_terminate_deterministically() -> Result<()> {
    let fixture = Fixture::new()?;
    let nested = fixture.config.app_dir.join("nested");
    fs::create_dir(&nested)?;
    if !symlink(&fixture.config.app_dir, &nested.join("cycle"), true)? {
        return Ok(());
    }
    let inputs = fixture.inputs()?;
    let before = snapshot(&inputs)?;
    assert_eq!(before, snapshot(&inputs)?);
    fs::write(nested.join("child.html"), "new")?;
    assert_ne!(before, snapshot(&inputs)?);
    Ok(())
}

#[test]
fn output_cannot_hide_an_explicit_component_root() -> Result<()> {
    let mut fixture = Fixture::new()?;
    fixture
        .config
        .app_args
        .components
        .push(fixture.output.to_string_lossy().into_owned());
    assert!(fixture.inputs()?.capture()?.is_none());
    Ok(())
}

#[cfg(windows)]
#[test]
fn sharing_denied_file_never_becomes_a_cached_success() -> Result<()> {
    use std::os::windows::fs::OpenOptionsExt;

    let fixture = Fixture::new()?;
    let inputs = fixture.inputs()?;
    for name in ["sample-card.html", "sample-card.css"] {
        let path = fixture.config.app_dir.join(name);
        snapshot(&inputs)?;
        let lock = fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&path)?;
        assert!(inputs.capture()?.is_none());
        drop(lock);
        snapshot(&inputs)?;
    }
    Ok(())
}

#[cfg(windows)]
#[test]
fn filename_only_script_does_not_open_a_sharing_denied_file() -> Result<()> {
    use std::os::windows::fs::OpenOptionsExt;

    let fixture = Fixture::new()?;
    let inputs = fixture.inputs()?;
    let path = fixture.config.app_dir.join("sample-card.ts");
    let before = snapshot(&inputs)?;
    let _lock = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&path)?;
    assert!(File::open(&path).is_err());
    assert_eq!(before, snapshot(&inputs)?);
    Ok(())
}

#[cfg(unix)]
#[test]
fn unreadable_file_never_becomes_a_cached_success() -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let fixture = Fixture::new()?;
    let path = fixture.config.app_dir.join("sample-card.html");
    let original = fs::metadata(&path)?.permissions();
    fs::set_permissions(&path, fs::Permissions::from_mode(0))?;
    let capture = fixture.inputs()?.capture();
    let readable_by_privileged_user = File::open(&path).is_ok();
    fs::set_permissions(&path, original)?;
    if !readable_by_privileged_user {
        assert!(capture?.is_none());
    }
    Ok(())
}

#[cfg(unix)]
#[test]
fn unknown_non_regular_input_is_not_silently_skipped() -> Result<()> {
    let fixture = Fixture::new()?;
    let _socket =
        std::os::unix::net::UnixListener::bind(fixture.config.app_dir.join("unknown.socket"))?;
    assert!(fixture.inputs()?.capture()?.is_none());
    Ok(())
}
