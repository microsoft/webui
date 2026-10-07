# Host-owned directory picker

The separate `native-picker` feature grants no renderer authority. It admits
at most one native folder selection per verified local-server window, with
host-authored bounded title and absolute starting directory. macOS and
Windows use OS-owned directory UIs, not a browser or screen-capture fallback.
If `native-dialogs` is also enabled, its per-window modal reservation excludes
overlapping picker and alert sheets; either feature compiles independently.

Windows runs `IFileOpenDialog` on a new COM STA only after picker admission.
The dialog uses `FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST`,
the exact current owner HWND and its generation cookie. A hidden message-only
window belongs to the same worker STA and accepts only the matching
payload-free cancel wake; its procedure calls `IFileDialog::Close` on that
STA, never through an unsafe cross-thread COM interface or WebView2.
This is best-effort native dismissal: Shell's modal loop may delay delivery,
so no hard OS teardown is promised. Setup, `Show`, and selection failures are
typed errors; an early or refused `Close` is logged numerically without
retrying or converting a stale request into a selection. Busy remains until
the native call returns.

Selected `SIGDN_FILESYSPATH` bytes are read from bounded CoTaskMem UTF-16 and
converted without Unicode replacement to an absolute path. Canonicalization
and directory metadata checks run only on the worker. The 120-second deadline
completes the future logically; one Busy reservation survives navigation,
host revocation, Future drop and timeout until `Show` and worker validation
actually exit. COM objects, the cancellation HWND and the worker apartment
are released before the picker permit is released and the host completion
waker runs. Only a verified, current owner generation may deliver success.
No Windows picker GUI or cancellation was executed in this environment.
