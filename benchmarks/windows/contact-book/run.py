# Copyright (c) Microsoft Corporation.
# Licensed under the MIT license.

import argparse
import ctypes
import ctypes.wintypes
import json
import os
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

CREATE_NEW_PROCESS_GROUP = 0x00000200
JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE = 0x00002000
JOB_OBJECT_EXTENDED_LIMIT_INFORMATION = 9
PROCESS_QUERY_INFORMATION = 0x0400
PROCESS_QUERY_LIMITED_INFORMATION = 0x1000
PROCESS_VM_READ = 0x0010
PROCESS_SET_QUOTA = 0x0100
PROCESS_TERMINATE = 0x0001
STILL_ACTIVE = 259
TH32CS_SNAPPROCESS = 0x00000002
WM_CLOSE = 0x0010

kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
psapi = ctypes.WinDLL("psapi", use_last_error=True)
user32 = ctypes.WinDLL("user32", use_last_error=True)


class BasicLimitInformation(ctypes.Structure):
    _fields_ = [
        ("PerProcessUserTime", ctypes.c_longlong),
        ("PerJobUserTime", ctypes.c_longlong),
        ("LimitFlags", ctypes.wintypes.DWORD),
        ("MinimumWorkingSetSize", ctypes.c_size_t),
        ("MaximumWorkingSetSize", ctypes.c_size_t),
        ("ActiveProcessLimit", ctypes.wintypes.DWORD),
        ("Affinity", ctypes.c_size_t),
        ("PriorityClass", ctypes.wintypes.DWORD),
        ("SchedulingClass", ctypes.wintypes.DWORD),
    ]


class IoCounters(ctypes.Structure):
    _fields_ = [("value", ctypes.c_longlong)] * 6


class ExtendedLimitInformation(ctypes.Structure):
    _fields_ = [
        ("BasicLimitInformation", BasicLimitInformation),
        ("IoInfo", IoCounters),
        ("ProcessMemoryLimit", ctypes.c_size_t),
        ("PeakProcessMemoryUsed", ctypes.c_size_t),
        ("JobMemoryLimit", ctypes.c_size_t),
        ("PeakJobMemoryUsed", ctypes.c_size_t),
    ]


class JobBasicAndIo(ctypes.Structure):
    _fields_ = [
        ("TotalUserTime", ctypes.c_longlong),
        ("TotalKernelTime", ctypes.c_longlong),
        ("ThisPeriodTotalUserTime", ctypes.c_longlong),
        ("ThisPeriodTotalKernelTime", ctypes.c_longlong),
        ("TotalPageFaultCount", ctypes.wintypes.DWORD),
        ("TotalProcesses", ctypes.wintypes.DWORD),
        ("ActiveProcesses", ctypes.wintypes.DWORD),
        ("TotalTerminatedProcesses", ctypes.wintypes.DWORD),
        ("read_operations", ctypes.c_longlong),
        ("write_operations", ctypes.c_longlong),
        ("other_operations", ctypes.c_longlong),
        ("read_bytes", ctypes.c_longlong),
        ("write_bytes", ctypes.c_longlong),
        ("other_bytes", ctypes.c_longlong),
    ]


class ProcessMemoryCounters(ctypes.Structure):
    _fields_ = [
        ("cb", ctypes.wintypes.DWORD),
        ("PageFaultCount", ctypes.wintypes.DWORD),
        ("PeakWorkingSetSize", ctypes.c_size_t),
        ("WorkingSetSize", ctypes.c_size_t),
        ("QuotaPeakPagedPoolUsage", ctypes.c_size_t),
        ("QuotaPagedPoolUsage", ctypes.c_size_t),
        ("QuotaPeakNonPagedPoolUsage", ctypes.c_size_t),
        ("QuotaNonPagedPoolUsage", ctypes.c_size_t),
        ("PagefileUsage", ctypes.c_size_t),
        ("PeakPagefileUsage", ctypes.c_size_t),
        ("PrivateUsage", ctypes.c_size_t),
    ]


class Rect(ctypes.Structure):
    _fields_ = [
        ("left", ctypes.wintypes.LONG),
        ("top", ctypes.wintypes.LONG),
        ("right", ctypes.wintypes.LONG),
        ("bottom", ctypes.wintypes.LONG),
    ]


class ProcessEntry32(ctypes.Structure):
    _fields_ = [
        ("dwSize", ctypes.wintypes.DWORD),
        ("cntUsage", ctypes.wintypes.DWORD),
        ("th32ProcessID", ctypes.wintypes.DWORD),
        ("th32DefaultHeapID", ctypes.c_size_t),
        ("th32ModuleID", ctypes.wintypes.DWORD),
        ("cntThreads", ctypes.wintypes.DWORD),
        ("th32ParentProcessID", ctypes.wintypes.DWORD),
        ("pcPriClassBase", ctypes.wintypes.LONG),
        ("dwFlags", ctypes.wintypes.DWORD),
        ("szExeFile", ctypes.wintypes.WCHAR * 260),
    ]


kernel32.CreateJobObjectW.restype = ctypes.wintypes.HANDLE
kernel32.AssignProcessToJobObject.argtypes = [
    ctypes.wintypes.HANDLE,
    ctypes.wintypes.HANDLE,
]
kernel32.SetInformationJobObject.argtypes = [
    ctypes.wintypes.HANDLE,
    ctypes.wintypes.INT,
    ctypes.c_void_p,
    ctypes.wintypes.DWORD,
]
kernel32.QueryInformationJobObject.argtypes = [
    ctypes.wintypes.HANDLE,
    ctypes.wintypes.INT,
    ctypes.c_void_p,
    ctypes.wintypes.DWORD,
    ctypes.POINTER(ctypes.wintypes.DWORD),
]
kernel32.OpenProcess.argtypes = [
    ctypes.wintypes.DWORD,
    ctypes.wintypes.BOOL,
    ctypes.wintypes.DWORD,
]
kernel32.OpenProcess.restype = ctypes.wintypes.HANDLE
kernel32.GetExitCodeProcess.argtypes = [
    ctypes.wintypes.HANDLE,
    ctypes.POINTER(ctypes.wintypes.DWORD),
]
kernel32.CloseHandle.argtypes = [ctypes.wintypes.HANDLE]
kernel32.CreateToolhelp32Snapshot.argtypes = [
    ctypes.wintypes.DWORD,
    ctypes.wintypes.DWORD,
]
kernel32.CreateToolhelp32Snapshot.restype = ctypes.wintypes.HANDLE
kernel32.Process32FirstW.argtypes = [
    ctypes.wintypes.HANDLE,
    ctypes.POINTER(ProcessEntry32),
]
kernel32.Process32NextW.argtypes = [
    ctypes.wintypes.HANDLE,
    ctypes.POINTER(ProcessEntry32),
]
psapi.GetProcessMemoryInfo.argtypes = [
    ctypes.wintypes.HANDLE,
    ctypes.POINTER(ProcessMemoryCounters),
    ctypes.wintypes.DWORD,
]
user32.EnumWindows.argtypes = [ctypes.c_void_p, ctypes.wintypes.LPARAM]
user32.EnumWindows.restype = ctypes.wintypes.BOOL
user32.GetWindowThreadProcessId.argtypes = [
    ctypes.wintypes.HWND,
    ctypes.POINTER(ctypes.wintypes.DWORD),
]
user32.IsWindowVisible.argtypes = [ctypes.wintypes.HWND]
user32.GetWindowTextW.argtypes = [
    ctypes.wintypes.HWND,
    ctypes.wintypes.LPWSTR,
    ctypes.c_int,
]
user32.PostMessageW.argtypes = [
    ctypes.wintypes.HWND,
    ctypes.wintypes.UINT,
    ctypes.wintypes.WPARAM,
    ctypes.wintypes.LPARAM,
]
user32.GetClientRect.argtypes = [ctypes.wintypes.HWND, ctypes.POINTER(Rect)]
user32.GetWindowRect.argtypes = [ctypes.wintypes.HWND, ctypes.POINTER(Rect)]
user32.SetWindowPos.argtypes = [
    ctypes.wintypes.HWND,
    ctypes.wintypes.HWND,
    ctypes.wintypes.INT,
    ctypes.wintypes.INT,
    ctypes.wintypes.INT,
    ctypes.wintypes.INT,
    ctypes.wintypes.UINT,
]

SWP_NOACTIVATE = 0x0010
SWP_NOZORDER = 0x0004


def monotonic_ms():
    return time.perf_counter_ns() / 1_000_000


def create_job():
    job = kernel32.CreateJobObjectW(None, None)
    if not job:
        raise ctypes.WinError(ctypes.get_last_error())
    limits = ExtendedLimitInformation()
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
    if not kernel32.SetInformationJobObject(
        job,
        9,
        ctypes.byref(limits),
        ctypes.sizeof(limits),
    ):
        kernel32.CloseHandle(job)
        raise ctypes.WinError(ctypes.get_last_error())
    return job


def query_job(job):
    info = JobBasicAndIo()
    returned = ctypes.wintypes.DWORD()
    if not kernel32.QueryInformationJobObject(
        job,
        8,
        ctypes.byref(info),
        ctypes.sizeof(info),
        ctypes.byref(returned),
    ):
        raise ctypes.WinError(ctypes.get_last_error())
    return {
        "cpu_ms": (info.TotalUserTime + info.TotalKernelTime) / 10_000,
    }


def process_children(root_pid):
    snapshot = kernel32.CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)
    if snapshot == ctypes.wintypes.HANDLE(-1).value:
        return []
    try:
        entry = ProcessEntry32()
        entry.dwSize = ctypes.sizeof(entry)
        processes = []
        if kernel32.Process32FirstW(snapshot, ctypes.byref(entry)):
            while True:
                processes.append(
                    (entry.th32ProcessID, entry.th32ParentProcessID)
                )
                if not kernel32.Process32NextW(snapshot, ctypes.byref(entry)):
                    break
        children = {root_pid}
        changed = True
        while changed:
            changed = False
            for pid, parent_pid in processes:
                if parent_pid in children and pid not in children:
                    children.add(pid)
                    changed = True
        return list(children)
    finally:
        kernel32.CloseHandle(snapshot)


def assign_process(job, pid):
    handle = kernel32.OpenProcess(
        PROCESS_SET_QUOTA | PROCESS_QUERY_INFORMATION | PROCESS_TERMINATE,
        False,
        pid,
    )
    if not handle:
        return
    try:
        kernel32.AssignProcessToJobObject(job, handle)
    finally:
        kernel32.CloseHandle(handle)


def assign_process_tree(job, root_pid):
    for pid in process_children(root_pid):
        assign_process(job, pid)


def process_rss(pid):
    handle = kernel32.OpenProcess(
        PROCESS_QUERY_INFORMATION | PROCESS_VM_READ,
        False,
        pid,
    )
    if not handle:
        return None
    try:
        counters = ProcessMemoryCounters()
        counters.cb = ctypes.sizeof(counters)
        if not psapi.GetProcessMemoryInfo(
            handle, ctypes.byref(counters), counters.cb
        ):
            return None
        return counters.WorkingSetSize
    finally:
        kernel32.CloseHandle(handle)


def process_is_running(pid):
    handle = kernel32.OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, False, pid)
    if not handle:
        return False
    try:
        exit_code = ctypes.wintypes.DWORD()
        return bool(
            kernel32.GetExitCodeProcess(handle, ctypes.byref(exit_code))
            and exit_code.value == STILL_ACTIVE
        )
    finally:
        kernel32.CloseHandle(handle)


def sample_peak_rss(root_pid, stop, result):
    peaks = {}
    while not stop.is_set():
        for pid in process_children(root_pid):
            rss = process_rss(pid)
            if rss is not None:
                peaks[pid] = max(peaks.get(pid, 0), rss)
        stop.wait(0.005)
    result["peaks"] = peaks


def find_window(root_pid):
    found = []
    titled = []
    process_ids = set(process_children(root_pid))

    @ctypes.WINFUNCTYPE(
        ctypes.wintypes.BOOL,
        ctypes.wintypes.HWND,
        ctypes.wintypes.LPARAM,
    )
    def callback(hwnd, _):
        if not user32.IsWindowVisible(hwnd):
            return True
        window_pid = ctypes.wintypes.DWORD()
        user32.GetWindowThreadProcessId(hwnd, ctypes.byref(window_pid))
        if window_pid.value not in process_ids:
            return True
        title = ctypes.create_unicode_buffer(256)
        user32.GetWindowTextW(hwnd, title, len(title))
        if title.value:
            titled.append((hwnd, window_pid.value))
        if "Contact Book" in title.value:
            found.append((hwnd, window_pid.value))
            return False
        return True

    user32.EnumWindows(callback, 0)
    return found[0] if found else (titled[0] if titled else None)


def wait_for_window(process, job, timeout_ms):
    deadline = monotonic_ms() + timeout_ms
    while monotonic_ms() < deadline:
        assign_process_tree(job, process.pid)
        window = find_window(process.pid)
        if window:
            return (*window, monotonic_ms())
        if process.poll() is not None and not any(
            process_is_running(pid) for pid in process_children(process.pid)
        ):
            stderr = process.stderr.read().decode(errors="replace") if process.stderr else ""
            raise RuntimeError(
                f"host exited before creating a window with {process.returncode}:\n{stderr}"
            )
        time.sleep(0.005)
    raise TimeoutError(f"Contact Book window was not created for pid {process.pid}")


def set_client_viewport(hwnd, width, height):
    client = Rect()
    outer = Rect()
    if not user32.GetClientRect(hwnd, ctypes.byref(client)):
        raise ctypes.WinError(ctypes.get_last_error())
    if not user32.GetWindowRect(hwnd, ctypes.byref(outer)):
        raise ctypes.WinError(ctypes.get_last_error())
    client_width = client.right - client.left
    client_height = client.bottom - client.top
    outer_width = outer.right - outer.left + width - client_width
    outer_height = outer.bottom - outer.top + height - client_height
    if not user32.SetWindowPos(
        hwnd,
        None,
        outer.left,
        outer.top,
        outer_width,
        outer_height,
        SWP_NOACTIVATE | SWP_NOZORDER,
    ):
        raise ctypes.WinError(ctypes.get_last_error())


def run_host(name, command, env, probe_path):
    launch_ms = monotonic_ms()
    launch_epoch_ms = time.time() * 1000
    job = create_job()
    process = None
    rss_stop = threading.Event()
    rss_result = {"peaks": {}}
    rss_sampler = None
    try:
        process = subprocess.Popen(
            command,
            cwd=ROOT,
            env=env,
            creationflags=CREATE_NEW_PROCESS_GROUP,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.PIPE,
        )
        rss_sampler = threading.Thread(
            target=sample_peak_rss,
            args=(process.pid, rss_stop, rss_result),
            daemon=True,
        )
        rss_sampler.start()
        process_handle = kernel32.OpenProcess(
            PROCESS_SET_QUOTA | PROCESS_QUERY_INFORMATION | PROCESS_TERMINATE,
            False,
            process.pid,
        )
        if not process_handle:
            raise ctypes.WinError(ctypes.get_last_error())
        try:
            if not kernel32.AssignProcessToJobObject(job, process_handle):
                raise ctypes.WinError(ctypes.get_last_error())
        finally:
            kernel32.CloseHandle(process_handle)
        assign_process_tree(job, process.pid)

        hwnd, host_pid, main_ms = wait_for_window(process, job, 30_000)
        set_client_viewport(hwnd, 1200, 800)
        probe_result = subprocess.run(
            ["node", str(probe_path), str(env["WEBUI_BENCHMARK_CDP_PORT"])],
            cwd=ROOT,
            env=env,
            capture_output=True,
            text=True,
            timeout=45,
            check=False,
        )
        if probe_result.returncode:
            raise RuntimeError(
                f"{name} readiness probe failed:\n{probe_result.stderr}"
            )
        readiness = json.loads(probe_result.stdout)
        dashboard_tti = readiness["readyEpochMs"] - launch_epoch_ms

        close_start = monotonic_ms()
        if not user32.PostMessageW(hwnd, WM_CLOSE, 0, 0):
            raise ctypes.WinError(ctypes.get_last_error())
        while process_is_running(host_pid):
            assign_process_tree(job, process.pid)
            time.sleep(0.005)
        exit_ms = monotonic_ms()
        rss_stop.set()
        rss_sampler.join()
        if process.poll() is None:
            process.wait(timeout=5)
        stderr = process.stderr.read().decode(errors="replace") if process.stderr else ""
        if process.returncode != 0:
            raise RuntimeError(f"{name} exited with {process.returncode}:\n{stderr}")
        metrics = query_job(job)
        return {
            "host": name,
            "dashboard_tti_ms": dashboard_tti,
            "launcher_to_host_main_ms": main_ms - launch_ms,
            "host_process_peak_rss_bytes": rss_result["peaks"].get(host_pid, 0),
            "process_tree_cpu_ms": metrics["cpu_ms"],
            "close_to_exit_ms": exit_ms - close_start,
            "readiness": readiness,
        }
    finally:
        if process and process.poll() is None:
            process.kill()
            process.wait()
            assign_process_tree(job, process.pid)
        rss_stop.set()
        if rss_sampler is not None and rss_sampler.is_alive():
            rss_sampler.join()
        kernel32.CloseHandle(job)
        time.sleep(0.5)


def build_commands(root, port, profile):
    app_dist = root / "examples" / "app" / "contact-book-manager" / "dist"
    state = root / "examples" / "app" / "contact-book-manager" / "data" / "state.json"
    theme = root / "packages" / "webui-examples-theme" / "tokens.json"
    electron = (
        root
        / "examples"
        / "integration"
        / "electron"
        / "node_modules"
        / "electron"
        / "dist"
        / "electron.exe"
    )
    electron_main = root / "examples" / "integration" / "electron" / "dist" / "main.js"
    electron_entry = root / "benchmarks" / "windows" / "contact-book" / "electron_entry.mjs"
    native = (
        root
        / "target"
        / "contact-book-benchmark-package"
        / "Contact-Book-Manager-windows-portable"
        / "contact-book-desktop.exe"
    )
    addon = root / "target" / "release" / "webui_node.dll"
    electron_state = profile / "electron-state.json"
    write_themed_state(state, theme, electron_state)
    common = os.environ.copy()
    common["WEBUI_BENCHMARK_CDP_PORT"] = str(port)
    common["WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS"] = (
        f"--remote-debugging-port={port}"
    )
    electron_env = common | {
        "WEBUI_ADDON_PATH": str(addon),
        "ELECTRON_NO_ATTACH_CONSOLE": "1",
        "WEBUI_BENCHMARK_APP_DIST": str(app_dist),
        "WEBUI_BENCHMARK_STATE_PATH": str(electron_state),
        "WEBUI_BENCHMARK_ELECTRON_MAIN": str(electron_main),
    }
    webui_env = common | {
        "LOCALAPPDATA": str(profile / "localappdata"),
        "TEMP": str(profile / "temp"),
        "TMP": str(profile / "temp"),
    }
    (profile / "localappdata").mkdir(parents=True)
    (profile / "temp").mkdir(parents=True)
    return (
        [
            str(electron),
            f"--remote-debugging-port={port}",
            f"--user-data-dir={profile / 'electron'}",
            str(electron_entry),
        ],
        electron_env,
        [str(native)],
        webui_env,
    )


def write_themed_state(state_path, theme_path, output_path):
    state = json.loads(state_path.read_text(encoding="utf-8"))
    themes = json.loads(theme_path.read_text(encoding="utf-8"))["themes"]
    state["tokens"] = {
        name: "\n".join(
            f"--{token}: {values[token]};" for token in sorted(values)
        )
        for name, values in themes.items()
    }
    output_path.write_text(json.dumps(state), encoding="utf-8")


def run():
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--output-dir",
        type=Path,
        default=ROOT / "benchmarks" / "windows" / "contact-book" / "results",
    )
    parser.add_argument("--pairs", type=int, default=20)
    args = parser.parse_args()
    args.output_dir.mkdir(parents=True, exist_ok=True)
    probe_path = ROOT / "benchmarks" / "windows" / "contact-book" / "cdp_probe.mjs"
    rows = []
    with tempfile.TemporaryDirectory(
        prefix="webui-contact-book-", ignore_cleanup_errors=True
    ) as temp:
        temp_root = Path(temp)
        for warmup_index, warmup_host in enumerate(("electron", "webui")):
            profile = temp_root / f"warmup-{warmup_host}"
            profile.mkdir()
            port = 9400 + warmup_index
            electron_cmd, electron_env, webui_cmd, webui_env = build_commands(
                ROOT, port, profile
            )
            command, env = (
                (electron_cmd, electron_env)
                if warmup_host == "electron"
                else (webui_cmd, webui_env)
            )
            run_host(warmup_host, command, env, probe_path)

        for pair_number in range(1, args.pairs + 1):
            order = ("electron", "webui") if pair_number % 2 else ("webui", "electron")
            pair = {"pair": pair_number}
            for offset, host in enumerate(order):
                profile = temp_root / f"pair-{pair_number}-{host}"
                profile.mkdir()
                port = 9500 + (pair_number * 2) + offset
                electron_cmd, electron_env, webui_cmd, webui_env = build_commands(
                    ROOT, port, profile
                )
                command, env = (
                    (electron_cmd, electron_env)
                    if host == "electron"
                    else (webui_cmd, webui_env)
                )
                pair[host] = run_host(host, command, env, probe_path)
            rows.append(pair)
            print(f"completed pair {pair_number}/{args.pairs}", flush=True)

    raw = {
        "schema": 1,
        "source": "benchmarks/windows/contact-book",
        "host": "Windows",
        "pairs": rows,
        "method": {
            "warmups_per_host": 1,
            "pairs": args.pairs,
            "viewport": [1200, 800],
            "outlier_rule": "paired metric modified z-score absolute score > 3.5",
            "rss": "main host process working set sampled during lifecycle",
            "cpu": "Windows Job Object process-tree user plus kernel time",
        },
    }
    raw_path = args.output_dir / "windows-raw.json"
    summary_path = args.output_dir / "windows-summary.json"
    raw_path.write_text(json.dumps(raw, indent=2) + "\n", encoding="utf-8")
    subprocess.run(
        [sys.executable, str(ROOT / "benchmarks/windows/contact-book/analyze.py"), str(raw_path), str(summary_path)],
        check=True,
    )
    print(summary_path)


ROOT = Path(__file__).resolve().parents[3]


if __name__ == "__main__":
    run()
