//! The two-pass const writer and the string helpers the renderers need.
//!
//! The renderers run twice: once as `Output<0>`, which only counts, and once as
//! `Output<LENGTH>`, which writes.

/// Const-evaluable output buffer.
///
/// With `CAPACITY == 0` nothing is stored and only [`Output::length`] advances (the measuring
/// pass). With `CAPACITY > 0` the bytes are written into `bytes`, which must be exactly the
/// length the measuring pass reported; anything else is an out-of-bounds const-eval error.
pub struct Output<const CAPACITY: usize> {
    /// The rendered bytes. Meaningful only when `CAPACITY` is the measured length.
    pub bytes: [u8; CAPACITY],
    /// The number of bytes rendered.
    pub length: usize,
}

impl<const CAPACITY: usize> Default for Output<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const CAPACITY: usize> Output<CAPACITY> {
    /// A fresh, empty writer.
    pub const fn new() -> Self {
        Self {
            bytes: [0u8; CAPACITY],
            length: 0,
        }
    }

    const fn put_byte(&mut self, byte: u8) {
        if CAPACITY > 0 {
            self.bytes[self.length] = byte;
        }
        self.length += 1;
    }

    /// Append a string.
    pub const fn put(&mut self, text: &str) {
        let bytes = text.as_bytes();
        let mut index = 0;
        while index < bytes.len() {
            self.put_byte(bytes[index]);
            index += 1;
        }
    }

    /// Append a decimal integer.
    pub const fn put_usize(&mut self, value: usize) {
        if value >= 10 {
            self.put_usize(value / 10);
        }
        self.put_byte(b'0' + (value % 10) as u8);
    }

    /// Append a string as a double-quoted Javascript literal, escaping backslashes and
    /// quotes.
    pub const fn put_quoted(&mut self, text: &str) {
        self.put_byte(b'"');
        let bytes = text.as_bytes();
        let mut index = 0;
        while index < bytes.len() {
            let byte = bytes[index];
            if byte == b'"' || byte == b'\\' {
                self.put_byte(b'\\');
            }
            self.put_byte(byte);
            index += 1;
        }
        self.put_byte(b'"');
    }

    /// Append an identifier, adding a trailing underscore if it is a reserved word.
    pub const fn put_ident(&mut self, name: &str) {
        self.put(name);
        if is_reserved(name) {
            self.put("_");
        }
    }

    /// Append an identifier as a quoted string, with the same underscore rule.
    pub const fn put_quoted_ident(&mut self, name: &str) {
        self.put_byte(b'"');
        self.put_ident(name);
        self.put_byte(b'"');
    }

    /// Append `count` levels of two-space indentation.
    pub const fn indent(&mut self, count: usize) {
        let mut level = 0;
        while level < count {
            self.put("  ");
            level += 1;
        }
    }
}

/// Compare two strings for equality in a const context.
pub const fn str_eq(left: &str, right: &str) -> bool {
    let left = left.as_bytes();
    let right = right.as_bytes();
    if left.len() != right.len() {
        return false;
    }
    let mut index = 0;
    while index < left.len() {
        if left[index] != right[index] {
            return false;
        }
        index += 1;
    }
    true
}

/// True if `text` is a bare Javascript identifier, which is what decides whether a schema
/// container is declared under its own name or rendered inline. Postcard-schema names std
/// generics `"Vec<T>"`, `"Result<T, E>"` and so on, which this rejects.
pub const fn is_ident(text: &str) -> bool {
    let bytes = text.as_bytes();
    if bytes.is_empty() {
        return false;
    }
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        let alpha = byte.is_ascii_alphabetic() || byte == b'_' || byte == b'$';
        let ok = if index == 0 {
            alpha
        } else {
            alpha || byte.is_ascii_digit()
        };
        if !ok {
            return false;
        }
        index += 1;
    }
    true
}

/// The Javascript reserved words, plus the two identifiers that are not reserved but cannot
/// be bound in strict mode.
const RESERVED: &[&str] = &[
    "arguments",
    "await",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "debugger",
    "default",
    "delete",
    "do",
    "else",
    "enum",
    "eval",
    "export",
    "extends",
    "false",
    "finally",
    "for",
    "function",
    "if",
    "implements",
    "import",
    "in",
    "instanceof",
    "interface",
    "let",
    "new",
    "null",
    "package",
    "private",
    "protected",
    "public",
    "return",
    "static",
    "super",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "typeof",
    "var",
    "void",
    "while",
    "with",
    "yield",
];

/// True if `name` may not be used as an identifier and needs a trailing underscore.
pub const fn is_reserved(name: &str) -> bool {
    let mut index = 0;
    while index < RESERVED.len() {
        if str_eq(RESERVED[index], name) {
            return true;
        }
        index += 1;
    }
    false
}
