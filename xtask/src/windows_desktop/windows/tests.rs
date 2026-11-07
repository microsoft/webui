// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::*;
use windows_sys::Win32::Foundation::{GetLastError, ERROR_INVALID_PARAMETER};
use windows_sys::Win32::System::StationsAndDesktops::{
    GetThreadDesktop, GetUserObjectInformationW, OpenInputDesktop, UOI_NAME,
};
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetFocus, SetFocus};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, IsWindowVisible, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
};

fn desktop_name(handle: HDESK) -> String {
    let mut buffer = [0_u16; 256];
    let mut needed = 0;
    // SAFETY: The desktop is live and the output buffer has the specified size.
    unsafe {
        assert_ne!(
            GetUserObjectInformationW(
                handle,
                UOI_NAME,
                buffer.as_mut_ptr().cast(),
                u32::try_from(size_of_val(&buffer)).unwrap(),
                &mut needed,
            ),
            0
        );
    }
    let length = buffer.iter().position(|unit| *unit == 0).unwrap();
    String::from_utf16(&buffer[..length]).unwrap()
}

#[test]
fn isolates_real_visible_windows_and_focus_without_switching_input_desktops() {
    // SAFETY: Queries only the current thread and input desktop.
    let original = unsafe { GetThreadDesktop(GetCurrentThreadId()) };
    let original_name = desktop_name(original);
    let before = input_desktop_name();
    let binary = std::env::current_exe().unwrap();
    let args = [
        binary.to_str().unwrap(),
        "--exact",
        "windows_desktop::windows::tests::isolated_window_probe",
        "--ignored",
        "--nocapture",
    ]
    .map(String::from);
    assert_eq!(
        execute(Request {
            command: &args,
            timeout_ms: 30_000,
        })
        .unwrap(),
        0
    );
    assert_eq!(input_desktop_name(), before);
    // SAFETY: Queries only this thread's desktop after child termination.
    assert_eq!(
        desktop_name(unsafe { GetThreadDesktop(GetCurrentThreadId()) }),
        original_name
    );
}

fn input_desktop_name() -> String {
    // SAFETY: Opens the current input desktop only for reading, never switches it.
    let handle = unsafe { OpenInputDesktop(0, 0, DESKTOP_READOBJECTS) };
    assert!(!handle.is_null());
    let name = desktop_name(handle);
    // SAFETY: This function owns the OpenInputDesktop reference.
    assert_ne!(unsafe { CloseDesktop(handle) }, 0);
    name
}

#[test]
#[ignore = "child-only probe; run by the desktop isolation regression"]
fn isolated_window_probe() {
    assert_isolated_window();
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "windows_desktop::windows::tests::grandchild_window_probe",
            "--ignored",
        ])
        .status()
        .unwrap();
    assert!(
        status.success(),
        "grandchild must inherit the isolated desktop"
    );
}

#[test]
#[ignore = "child-only probe; run by the desktop isolation regression"]
fn grandchild_window_probe() {
    assert_isolated_window();
}

fn assert_isolated_window() {
    assert!(super::is_current().unwrap());
    // Check isolation before creating anything; direct invocation cannot show a window.
    // SAFETY: Queries the desktop assigned to the current test thread.
    let name = desktop_name(unsafe { GetThreadDesktop(GetCurrentThreadId()) });
    assert!(name.starts_with("WebUI-Tests-"), "{name}");
    assert_ne!(name, input_desktop_name());
    let class: Vec<_> = "STATIC".encode_utf16().chain(Some(0)).collect();
    // SAFETY: Uses the system STATIC class, valid strings, and no borrowed window state.
    let window = unsafe {
        CreateWindowExW(
            0,
            class.as_ptr(),
            class.as_ptr(),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            0,
            0,
            200,
            100,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null(),
        )
    };
    assert!(!window.is_null());
    // SAFETY: The probe owns this window on its creating UI thread.
    unsafe {
        let visible = IsWindowVisible(window);
        SetFocus(window);
        let focused = GetFocus() == window;
        assert_ne!(DestroyWindow(window), 0);
        assert_ne!(visible, 0);
        assert!(focused);
    }
}

#[test]
fn kills_orphaned_descendants_when_the_command_exits() {
    let desktop = Desktop::new().unwrap();
    let name = desktop_name(desktop.handle);
    let record = std::env::temp_dir().join(format!("{name}.pid"));
    let binary = std::env::current_exe().unwrap();
    let args = [
        binary.to_str().unwrap(),
        "--exact",
        "windows_desktop::windows::tests::orphan_parent_probe",
        "--ignored",
    ]
    .map(String::from);
    assert_eq!(
        launch(
            Request {
                command: &args,
                timeout_ms: 30_000,
            },
            &desktop,
        )
        .unwrap(),
        0
    );
    let pid = std::fs::read_to_string(&record)
        .unwrap()
        .parse::<u32>()
        .unwrap();
    std::fs::remove_file(record).unwrap();
    // SAFETY: Opens the recorded fixture PID only for waiting; it may already be gone.
    let process = unsafe {
        windows_sys::Win32::System::Threading::OpenProcess(
            windows_sys::Win32::System::Threading::PROCESS_SYNCHRONIZE,
            0,
            pid,
        )
    };
    if !process.is_null() {
        let process = Handle(process);
        // SAFETY: The guard owns this process handle; allow asynchronous job teardown.
        assert_eq!(
            unsafe { WaitForSingleObject(process.0, 5_000) },
            WAIT_OBJECT_0
        );
    } else {
        // SAFETY: Reads the immediately preceding OpenProcess failure.
        assert_eq!(unsafe { GetLastError() }, ERROR_INVALID_PARAMETER);
    }
}

#[test]
#[ignore = "child-only probe; run by the process-tree cleanup regression"]
fn orphan_parent_probe() {
    // SAFETY: Reads the desktop name before launching the controlled child.
    let name = desktop_name(unsafe { GetThreadDesktop(GetCurrentThreadId()) });
    assert!(name.starts_with("WebUI-Tests-"));
    let child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "windows_desktop::windows::tests::orphan_child_probe",
            "--ignored",
        ])
        .spawn()
        .unwrap();
    let pid = child.id();
    drop(child);
    std::fs::write(
        std::env::temp_dir().join(format!("{name}.pid")),
        pid.to_string(),
    )
    .unwrap();
}

#[test]
#[ignore = "child-only probe; run by the process-tree cleanup regression"]
fn orphan_child_probe() {
    // SAFETY: Direct invocation on the user's desktop fails before sleeping.
    let name = desktop_name(unsafe { GetThreadDesktop(GetCurrentThreadId()) });
    assert!(name.starts_with("WebUI-Tests-"));
    std::thread::sleep(std::time::Duration::from_secs(60));
}

#[test]
fn preserves_failure_exit_codes_and_reports_timeouts() {
    for (script, expected) in [("exit 37", 37), ("exit -1", u32::MAX)] {
        let command = ["cmd.exe", "/d", "/c", script].map(String::from);
        assert_eq!(
            execute(Request {
                command: &command,
                timeout_ms: 30_000,
            })
            .unwrap(),
            expected
        );
    }
    let binary = std::env::current_exe().unwrap();
    let command = [
        binary.to_str().unwrap(),
        "--exact",
        "windows_desktop::windows::tests::orphan_child_probe",
        "--ignored",
    ]
    .map(String::from);
    assert!(execute(Request {
        command: &command,
        timeout_ms: 1,
    })
    .unwrap_err()
    .contains("timed out"));
}
