//! Read-only operation classifier.
//!
//! In the pure-Rust client the read-only guarantee is *structural*: every AWS
//! call must be in the compiled registry and pass this read-only classifier
//! before policy.yaml is evaluated. No mutating operation is compiled in.

/// Operation-name prefixes that are read-only by convention across AWS.
pub const READ_ONLY_PREFIXES: &[&str] = &[
    "Get", "List", "Describe", "Search", "Scan", "Filter", "Query", "Head", "Lookup", "Read",
    "View", "Show", "Check", "Validate", "Detect", "Discover", "Test", "Estimate", "Simulate",
    "Verify", "Preview", "Resolve", "Compare", "Trace", "Sample", "Match", "Diff", "Decode",
    "Locate", "Find", "Batch",
];

/// Operations whose prefix is read-only but which actually mutate. Always blocked.
const EXPLICIT_BLOCKED: &[&str] = &[
    "BatchWriteItem",
    "BatchExecuteStatement",
    "ExecuteStatement",
    "ExecuteTransaction",
    "GenerateDataKey",
    "GenerateDataKeyPair",
    "GenerateDataKeyPairWithoutPlaintext",
    "GenerateDataKeyWithoutPlaintext",
    "GenerateRandom",
];

/// Read-only operations that are important enough to document explicitly.
const ALLOWED_OPERATIONS: &[&str] = &[
    "StartQuery",
    "StopQuery",
    "AssumeRole",
    "AssumeRoleWithSAML",
    "AssumeRoleWithWebIdentity",
    "GetSessionToken",
    "GetFederationToken",
    "GetCallerIdentity",
    "GetRoleCredentials",
];

/// True iff `operation` is read-only by policy.
pub fn is_read_only(operation: &str) -> bool {
    if operation.is_empty() {
        return false;
    }
    if EXPLICIT_BLOCKED.contains(&operation) {
        return false;
    }
    if ALLOWED_OPERATIONS.contains(&operation) {
        return true;
    }
    READ_ONLY_PREFIXES.iter().any(|p| operation.starts_with(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_allowed_writes_blocked() {
        assert!(is_read_only("DescribeStacks"));
        assert!(is_read_only("ListPipelines"));
        assert!(is_read_only("StartQuery")); // explicit allow
        assert!(ALLOWED_OPERATIONS.contains(&"GetCallerIdentity"));
        assert!(!is_read_only("BatchWriteItem")); // explicit block despite Batch prefix
        assert!(!is_read_only("CreateStack"));
        assert!(!is_read_only("DeleteObject"));
        assert!(!is_read_only(""));
    }
}
