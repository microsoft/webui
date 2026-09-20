// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/// Append one quoted HTML attribute to an existing string allocation.
#[doc(hidden)]
#[inline]
pub fn append_attribute_to_string(output: &mut String, name: &str, value: &str) {
    output.push(' ');
    output.push_str(name);
    output.push_str("=\"");
    output.push_str(value);
    output.push('"');
}

/// Append one boolean HTML attribute to an existing string allocation.
#[doc(hidden)]
#[inline]
pub fn append_boolean_attribute_to_string(output: &mut String, name: &str) {
    output.push(' ');
    output.push_str(name);
}

/// Append one quoted HTML attribute to an existing byte allocation.
#[doc(hidden)]
#[inline]
pub fn append_attribute_to_bytes(output: &mut Vec<u8>, name: &str, value: &str) {
    output.push(b' ');
    output.extend_from_slice(name.as_bytes());
    output.extend_from_slice(b"=\"");
    output.extend_from_slice(value.as_bytes());
    output.push(b'"');
}

/// Append one boolean HTML attribute to an existing byte allocation.
#[doc(hidden)]
#[inline]
pub fn append_boolean_attribute_to_bytes(output: &mut Vec<u8>, name: &str) {
    output.push(b' ');
    output.extend_from_slice(name.as_bytes());
}

/// Generate optimized attribute methods for a string-backed `ResponseWriter`.
#[doc(hidden)]
#[macro_export]
macro_rules! string_response_writer_methods {
    ($field:ident) => {
        fn write_attribute(&mut self, name: &str, value: &str) -> $crate::Result<()> {
            self.$field.push(' ');
            self.$field.push_str(name);
            self.$field.push_str("=\"");
            self.$field.push_str(value);
            self.$field.push('"');
            Ok(())
        }

        fn write_boolean_attribute(&mut self, name: &str) -> $crate::Result<()> {
            self.$field.push(' ');
            self.$field.push_str(name);
            Ok(())
        }
    };
}

/// Generate optimized attribute methods for a byte-backed `ResponseWriter`.
#[doc(hidden)]
#[macro_export]
macro_rules! bytes_response_writer_methods {
    ($field:ident) => {
        fn write_attribute(&mut self, name: &str, value: &str) -> $crate::Result<()> {
            self.$field.push(b' ');
            self.$field.extend_from_slice(name.as_bytes());
            self.$field.extend_from_slice(b"=\"");
            self.$field.extend_from_slice(value.as_bytes());
            self.$field.push(b'"');
            Ok(())
        }

        fn write_boolean_attribute(&mut self, name: &str) -> $crate::Result<()> {
            self.$field.push(b' ');
            self.$field.extend_from_slice(name.as_bytes());
            Ok(())
        }
    };
}

/// Define a private string-backed writer with local, inlinable methods.
#[doc(hidden)]
#[macro_export]
macro_rules! define_string_response_writer {
    ($name:ident, $field:ident) => {
        struct $name {
            $field: String,
        }

        impl $name {
            fn with_capacity(capacity: usize) -> Self {
                Self {
                    $field: String::with_capacity(capacity),
                }
            }
        }

        impl $crate::ResponseWriter for $name {
            fn write(&mut self, content: &str) -> $crate::Result<()> {
                self.$field.push_str(content);
                Ok(())
            }

            $crate::string_response_writer_methods!($field);

            fn end(&mut self) -> $crate::Result<()> {
                Ok(())
            }
        }
    };
}

/// Define a private byte-backed writer with local, inlinable methods.
#[doc(hidden)]
#[macro_export]
macro_rules! define_bytes_response_writer {
    ($name:ident, $field:ident) => {
        struct $name {
            $field: Vec<u8>,
        }

        impl $name {
            fn with_capacity(capacity: usize) -> Self {
                Self {
                    $field: Vec::with_capacity(capacity),
                }
            }
        }

        impl $crate::ResponseWriter for $name {
            fn write(&mut self, content: &str) -> $crate::Result<()> {
                self.$field.extend_from_slice(content.as_bytes());
                Ok(())
            }

            $crate::bytes_response_writer_methods!($field);

            fn end(&mut self) -> $crate::Result<()> {
                Ok(())
            }
        }
    };
}
