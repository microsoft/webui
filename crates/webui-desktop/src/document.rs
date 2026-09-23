// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use crate::ipc::DocumentActivation;

// Shared epoch policy only. Platform owners retain native navigation IDs,
// proof/session storage, timers and FFI objects, and retire them before advancing.
#[derive(Clone, Copy, Default)]
pub(crate) struct DocumentEpoch {
    pub(crate) navigation: u64,
    pub(crate) committed: bool,
    pub(crate) closed: bool,
}

impl DocumentEpoch {
    pub(crate) fn advance(&mut self) -> Option<u64> {
        if self.closed {
            return None;
        }
        let Some(next) = self.navigation.checked_add(1) else {
            self.closed = true;
            return None;
        };
        self.navigation = next;
        self.committed = false;
        Some(next)
    }

    pub(crate) fn commit(&mut self) -> Option<u64> {
        if self.closed || self.committed || self.navigation == 0 {
            return None;
        }
        self.committed = true;
        Some(self.navigation)
    }

    pub(crate) fn current(&self, navigation: u64) -> bool {
        !self.closed && self.committed && self.navigation == navigation
    }

    pub(crate) fn accepts(
        &self,
        pending: Option<&DocumentActivation>,
        received: &DocumentActivation,
    ) -> bool {
        self.current(received.navigation)
            && pending.is_some_and(|pending| proof_matches(pending, received))
    }
}

pub(crate) fn proof_matches(pending: &DocumentActivation, received: &DocumentActivation) -> bool {
    let difference = pending
        .document_nonce
        .iter()
        .chain(&pending.challenge)
        .zip(received.document_nonce.iter().chain(&received.challenge))
        .fold(0_u8, |difference, (a, b)| difference | (a ^ b));
    pending.navigation == received.navigation && difference == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_epochs_cannot_commit_twice_revive_or_wrap() {
        let mut epoch = DocumentEpoch::default();
        assert_eq!(epoch.commit(), None);
        assert_eq!(epoch.advance(), Some(1));
        assert!(!epoch.current(1));
        assert_eq!(epoch.commit(), Some(1));
        assert_eq!(epoch.commit(), None);
        assert!(epoch.current(1));
        assert_eq!(epoch.advance(), Some(2));
        assert!(!epoch.current(1));
        epoch.navigation = u64::MAX;
        assert_eq!(epoch.advance(), None);
        assert!(epoch.closed);
        assert_eq!(epoch.commit(), None);
        assert_eq!(epoch.advance(), None);
    }
}
