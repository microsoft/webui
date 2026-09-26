// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::cell::Cell;
use std::rc::Rc;

use crate::ipc::IpcErrorCode;

// A completion retains only its own task epoch's terminal reason, not the
// driver or the replacement document. Unexplained abandonment stays Transport.
#[derive(Clone, Default)]
pub(crate) struct NativeIpcRetirement(Rc<Cell<Option<IpcErrorCode>>>);

impl NativeIpcRetirement {
    pub(super) fn retire(&self, code: IpcErrorCode) {
        if self.0.get().is_none() {
            self.0.set(Some(code));
        }
    }

    pub(crate) fn code(&self) -> IpcErrorCode {
        self.0.get().unwrap_or(IpcErrorCode::Transport)
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use crate::ipc::{IpcError, IpcWake};
    use crate::native_ipc::NativeIpcTasks;
    use std::cell::RefCell;
    use std::sync::Arc;

    struct Wake;
    impl IpcWake for Wake {
        fn wake(&self) -> Result<(), IpcError> {
            Ok(())
        }
    }

    struct Response {
        retirement: NativeIpcRetirement,
        sent: Rc<RefCell<Vec<IpcErrorCode>>>,
    }
    impl Drop for Response {
        fn drop(&mut self) {
            self.sent.borrow_mut().push(self.retirement.code());
        }
    }

    #[test]
    fn native_request_drop_observes_retirement_before_delayed_renderer_control() {
        for code in [
            IpcErrorCode::Navigated,
            IpcErrorCode::Closed,
            IpcErrorCode::Transport,
        ] {
            let tasks = NativeIpcTasks::new(Arc::new(Wake), 1);
            let retired = tasks.retirement();
            let sent = Rc::new(RefCell::new(Vec::new()));
            let response = Response {
                retirement: retired.clone(),
                sent: Rc::clone(&sent),
            };
            tasks
                .spawn(async move {
                    let _response = response;
                    std::future::pending::<()>().await;
                })
                .unwrap();
            tasks.poll_ready();
            assert!(sent.borrow().is_empty());
            // Native task cancellation runs before the asynchronous renderer
            // control can evaluate. Its response must already carry the reason.
            tasks.retire(code);
            assert_eq!(*sent.borrow(), [code]);
            tasks.close();
            tasks.poll_ready();
            drop(tasks);
            assert_eq!(retired.code(), code);
            assert_eq!(*sent.borrow(), [code]);
            let replacement = NativeIpcTasks::new(Arc::new(Wake), 1);
            assert_eq!(replacement.retirement().code(), IpcErrorCode::Transport);
            replacement.close();
            assert_eq!(replacement.retirement().code(), IpcErrorCode::Closed);
            assert_eq!(retired.code(), code);
        }
    }
}
