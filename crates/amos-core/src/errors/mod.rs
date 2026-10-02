//! AMOS error messages (taken from the editor configuration, as in the original).

pub mod messages;

/// Message of runtime error `n` (as raised by `Error n` or the interpreter).
pub fn message(n: u16) -> &'static str {
    find(messages::RUNTIME, n).unwrap_or("Unknown error")
}

/// Message of test-time (verifier) error `n`.
pub fn test_message(n: u16) -> &'static str {
    find(messages::TEST, n).unwrap_or("Unknown error")
}

/// Editor message `n`.
pub fn editor_message(n: u16) -> &'static str {
    find(messages::EDITOR, n).unwrap_or("")
}

fn find(list: &'static [(u16, &'static str)], n: u16) -> Option<&'static str> {
    list.iter().find(|(k, _)| *k == n).map(|(_, t)| *t).filter(|t| !t.is_empty())
}

/// Errors 0..=19 cannot be trapped with On Error / Trap.
pub fn is_fatal(n: u16) -> bool {
    n < 20
}

// Frequently used runtime error numbers.
pub const RETURN_WITHOUT_GOSUB: u16 = 1;
pub const POP_WITHOUT_GOSUB: u16 = 2;
pub const ERROR_NOT_RESUMED: u16 = 3;
pub const RESUME_WITHOUT_ERROR: u16 = 7;
pub const PROGRAM_INTERRUPTED: u16 = 9;
pub const END_OF_PROGRAM: u16 = 10;
pub const OUT_OF_STACK: u16 = 13;
pub const DIVISION_BY_ZERO: u16 = 20;
pub const STRING_TOO_LONG: u16 = 21;
pub const SYNTAX_ERROR: u16 = 22;
pub const ILLEGAL_FUNCTION_CALL: u16 = 23;
pub const OUT_OF_MEMORY: u16 = 24;
pub const NON_DIMENSIONED_ARRAY: u16 = 27;
pub const ARRAY_ALREADY_DIMENSIONED: u16 = 28;
pub const OVERFLOW: u16 = 29;
pub const OUT_OF_DATA: u16 = 33;
pub const TYPE_MISMATCH: u16 = 34;
pub const BANK_ALREADY_RESERVED: u16 = 35;
pub const BANK_NOT_RESERVED: u16 = 36;
pub const LABEL_NOT_DEFINED: u16 = 40;
pub const SCREEN_NOT_OPENED: u16 = 47;
pub const ILLEGAL_SCREEN_PARAMETER: u16 = 48;
pub const ILLEGAL_NUMBER_OF_COLOURS: u16 = 49;
pub const VALID_SCREEN_NUMBERS: u16 = 50;
pub const WINDOW_NOT_OPENED: u16 = 54;
pub const WINDOW_ALREADY_OPENED: u16 = 55;
pub const WINDOW_TOO_SMALL: u16 = 56;
pub const WINDOW_TOO_LARGE: u16 = 57;
pub const ILLEGAL_WINDOW_PARAMETER: u16 = 60;
pub const BOB_NOT_DEFINED: u16 = 68;
pub const FILE_NOT_FOUND: u16 = 81;
pub const FILE_NOT_OPENED: u16 = 97;
pub const END_OF_FILE: u16 = 100;
