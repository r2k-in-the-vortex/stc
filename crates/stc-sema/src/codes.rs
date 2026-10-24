//! Stable M3 diagnostic codes (D-062 and `M3-Plan.md` §7).

// Names and scopes.
/// Unknown name.
pub const NAME_UNKNOWN: &str = "STC-E-NAME-001";
/// A user POU shadows a standard name.
pub const NAME_SHADOWS_STANDARD: &str = "STC-W-NAME-002";
/// Duplicate definition in one scope.
pub const NAME_DUPLICATE: &str = "STC-E-NAME-003";
/// Definition is not accessible here.
pub const NAME_INACCESSIBLE: &str = "STC-E-NAME-004";
/// Name used in an invalid context.
pub const NAME_INVALID_CONTEXT: &str = "STC-E-NAME-005";
/// Ambiguous name from multiple imports.
pub const NAME_AMBIGUOUS: &str = "STC-E-NAME-006";

// Types.
/// General type mismatch.
pub const TYPE_MISMATCH: &str = "STC-E-TYPE-001";
/// Invalid operand or expression category.
pub const TYPE_INVALID_OPERAND: &str = "STC-E-TYPE-002";
/// Invalid or unsupported type.
pub const TYPE_INVALID: &str = "STC-E-TYPE-003";
/// Assignment mismatch (preserved example code).
pub const TYPE_ASSIGN: &str = "STC-E-TYPE-012";

// Constant evaluation.
/// Expression is not constant.
pub const CONST_NOT_CONSTANT: &str = "STC-E-CONST-001";
/// Constant evaluation failed (overflow, division by zero, bounds).
pub const CONST_FAILURE: &str = "STC-E-CONST-002";
/// Duplicate or overlapping case label.
pub const CONST_DUPLICATE_LABEL: &str = "STC-E-CONST-003";

// Conversions.
/// No conversion exists.
pub const CONV_NONE: &str = "STC-E-CONV-001";
/// Explicit conversion is required.
pub const CONV_EXPLICIT: &str = "STC-E-CONV-002";

// Calls.
/// Unknown or invalid callee.
pub const CALL_UNKNOWN: &str = "STC-E-CALL-001";
/// Argument count or missing required argument.
pub const CALL_ARITY: &str = "STC-E-CALL-002";
/// Named or output argument error.
pub const CALL_ARGUMENT: &str = "STC-E-CALL-003";
/// Ambiguous overload.
pub const CALL_AMBIGUOUS: &str = "STC-E-CALL-004";

// OOP.
/// Inheritance cycle.
pub const OOP_CYCLE: &str = "STC-E-OOP-001";
/// Invalid override or hiding.
pub const OOP_OVERRIDE: &str = "STC-E-OOP-002";
/// Incomplete interface implementation.
pub const OOP_INTERFACE: &str = "STC-E-OOP-003";
/// Abstract rule violation.
pub const OOP_ABSTRACT: &str = "STC-E-OOP-004";
/// Final rule violation.
pub const OOP_FINAL: &str = "STC-E-OOP-005";
/// Invalid assignment target or assignment-attempt relation.
pub const OOP_ASSIGN_TARGET: &str = "STC-E-OOP-006";

// Arrays.
/// Array bound or index error.
pub const ARRAY_BOUND: &str = "STC-E-ARRAY-001";
/// Conformant array placement error.
pub const ARRAY_CONFORMANT: &str = "STC-E-ARRAY-002";

// References.
/// Reference rule violation.
pub const REF_RULE: &str = "STC-E-REF-001";
/// Null or invalid reference use.
pub const REF_NULL: &str = "STC-E-REF-002";
/// Invalid assignment attempt.
pub const REF_ATTEMPT: &str = "STC-E-REF-003";

// Initializers and configuration.
/// Initializer shape or type error.
pub const INIT_SHAPE: &str = "STC-E-INIT-001";
/// Configuration/task/program binding error.
pub const CONFIG_ERROR: &str = "STC-E-CONFIG-001";

// I/O and retention.
/// `AT` address/size/type mismatch.
pub const IO_ADDRESS: &str = "STC-E-IO-001";
/// Write to a read-only `%I` variable.
pub const IO_INPUT_WRITE: &str = "STC-E-IO-002";
/// A retained leaf is not persistable.
pub const RETAIN_NOT_PERSISTABLE: &str = "STC-E-RETAIN-001";
/// Invalid RETAIN location.
pub const RETAIN_INVALID_LOCATION: &str = "STC-E-RETAIN-002";
/// RETAIN on a direct `AT` variable is ignored.
pub const RETAIN_DIRECT: &str = "STC-W-RETAIN-001";

// Exports.
/// Invalid or duplicate `{stc:export}`/`{stc:no_mangle}` symbol.
pub const EXPORT_INVALID: &str = "STC-E-EXPORT-001";

// Test-only units.
/// Production code references a test-only unit.
pub const TEST_REFERENCE: &str = "STC-E-TEST-001";
