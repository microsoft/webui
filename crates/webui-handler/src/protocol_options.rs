// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/// How a loaded protocol evaluates conditional fragments and boolean attributes.
///
/// Both modes resolve the current render's state and preserve identical
/// short-circuiting and error behavior. Neither caches results or request state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ConditionEvaluation {
    /// Prepare conditions once for faster repeated rendering, retaining their
    /// instructions, paths, and literal values for the protocol's lifetime.
    #[default]
    Prepared,
    /// Evaluate the existing condition trees directly without retaining
    /// prepared instructions, copied paths, or parsed literals.
    Direct,
}

/// Process-local options applied once when loading a runtime [`crate::Protocol`].
///
/// Options are not serialized into the build protocol and cannot be changed
/// on a loaded protocol. Construct a new protocol to change the policy.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProtocolOptions {
    /// Whether to trade retained condition storage for faster rendering.
    ///
    /// Defaults to [`ConditionEvaluation::Prepared`].
    pub condition_evaluation: ConditionEvaluation,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_options_preserve_prepared_evaluation() {
        assert_eq!(
            ConditionEvaluation::default(),
            ConditionEvaluation::Prepared
        );
        assert_eq!(
            ProtocolOptions::default().condition_evaluation,
            ConditionEvaluation::Prepared
        );
    }
}
