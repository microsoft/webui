// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use rolldown_error::{BuildDiagnostic, EventKind};
use serde::Serialize;

use super::ClientBuildError;

/// One actionable, nonfatal native bundler warning.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildWarning {
    /// Stable native warning code.
    pub code: String,
    /// Full native explanation, without terminal colors.
    pub message: String,
}

pub(super) fn collect(
    warnings: Vec<BuildDiagnostic>,
) -> Result<Vec<BuildWarning>, ClientBuildError> {
    // A module intentionally available statically and dynamically is valid.
    // Preserve this advisory; do not broaden the policy to unexpected warnings.
    if warnings
        .iter()
        .any(|warning| !matches!(warning.kind(), EventKind::IneffectiveDynamicImport))
    {
        return Err(ClientBuildError::Bundle(
            warnings
                .iter()
                .map(|warning| warning.to_diagnostic().convert_to_string(false))
                .collect::<Vec<_>>()
                .join("\n"),
        ));
    }
    let mut warnings: Vec<_> = warnings
        .into_iter()
        .map(|warning| BuildWarning {
            code: warning.kind().to_string(),
            message: warning.to_diagnostic().convert_to_string(false),
        })
        .collect();
    warnings.sort_unstable_by(|left, right| {
        (&left.code, &left.message).cmp(&(&right.code, &right.message))
    });
    Ok(warnings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unexpected_native_warnings_still_fail_in_a_mixed_report() {
        let warnings = vec![
            BuildDiagnostic::ineffective_dynamic_import(
                "target.js".into(),
                vec!["static.js".into()],
                vec!["dynamic.js".into()],
            )
            .with_severity_warning(),
            BuildDiagnostic::missing_name_option_for_iife_export(false).with_severity_warning(),
        ];
        assert!(collect(warnings).is_err());
    }
}
