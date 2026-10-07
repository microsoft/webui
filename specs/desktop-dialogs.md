# Host-owned live native dialogs

The opt-in `native-dialogs` capability belongs to the exact live local-server
window, independently of the pre-frame startup-failure alert and application
IPC. All visible title, message and button labels are bounded host-authored
single-line UTF-8. No native error description or page-provided copy is
automatically displayed.

One modal reservation covers error/confirmation dialogs per window. Admission is
not acknowledgement. The Mac adapter retains the exact owner `NSWindow`,
displays an asynchronous `NSAlert` sheet, and schedules cancellation on the
main queue. The Windows adapter runs `TaskDialogIndirect` on a separate COM
STA with the owning HWND; the WebView2 STA continues pumping. Its native timer
callback posts a cancel click only to the observed live dialog HWND on that
same worker thread. Neither path kills a thread or reports user success after
the ten-second logical deadline, navigation, host retirement or window close.
The first NSAlert button and Task Dialog default are Cancel for confirmations,
so Return never selects the affirmative action. Error dialogs show only one
acknowledgement button, which remains the default. Mac sheet cancellation
uses `endSheet:returnCode:`; Windows uses `WM_CLOSE` with Task Dialog's
allow-cancellation flag when no visible Cancel button exists. Either path
maps non-acknowledged dismissal to `Cancelled`, not `Acknowledged`.
Busy remains until native completion; the native callback releases the modal
reservation before waking host futures. OS refusal and unexpected button
responses are explicit errors, not an inferred confirmation. An already
visible dialog may remain briefly after cancellation if its OS message loop
has not acknowledged dismissal. Mac/Windows GUI runtime modality and owner
teardown have not been exercised in this environment.
