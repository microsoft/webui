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
        fn write_escaped(&mut self, content: &str) -> $crate::Result<()> {
            $crate::append_escaped_to_string(&mut self.$field, content);
            Ok(())
        }

        fn write_escaped_attribute(&mut self, name: &str, value: &str) -> $crate::Result<()> {
            self.$field.push(' ');
            self.$field.push_str(name);
            self.$field.push_str("=\"");
            $crate::append_escaped_to_string(&mut self.$field, value);
            self.$field.push('"');
            Ok(())
        }

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
        fn write_escaped(&mut self, content: &str) -> $crate::Result<()> {
            $crate::append_escaped_to_bytes(&mut self.$field, content);
            Ok(())
        }

        fn write_escaped_attribute(&mut self, name: &str, value: &str) -> $crate::Result<()> {
            self.$field.push(b' ');
            self.$field.extend_from_slice(name.as_bytes());
            self.$field.extend_from_slice(b"=\"");
            $crate::append_escaped_to_bytes(&mut self.$field, value);
            self.$field.push(b'"');
            Ok(())
        }

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

#[cfg(test)]
mod tests {
    use crate::{HandlerError, ResponseWriter, Result};

    crate::define_string_response_writer!(StringWriter, output);
    crate::define_bytes_response_writer!(BytesWriter, output);

    struct DefaultWriter(String);

    impl ResponseWriter for DefaultWriter {
        fn write(&mut self, content: &str) -> Result<()> {
            self.0.push_str(content);
            Ok(())
        }

        fn end(&mut self) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn buffered_escaped_writes_match_default_methods() -> Result<()> {
        let ascii: String = (0..=127).map(char::from).collect();
        let mut default = DefaultWriter(String::new());
        let mut text = StringWriter::with_capacity(1024);
        let mut bytes = BytesWriter::with_capacity(1024);
        for value in ["", "plain", "&<>\"'/", "&amp;", "é水🙂/", &ascii] {
            for writer in [
                &mut default as &mut dyn ResponseWriter,
                &mut text,
                &mut bytes,
            ] {
                writer.write_escaped(value)?;
                writer.write_escaped_attribute("data-value", value)?;
            }
        }
        assert_eq!(text.output, default.0);
        assert_eq!(bytes.output, default.0.as_bytes());
        Ok(())
    }

    #[test]
    fn default_escaped_methods_preserve_writer_errors() {
        struct Disconnected;
        impl ResponseWriter for Disconnected {
            fn write(&mut self, _content: &str) -> Result<()> {
                Err(HandlerError::ClientDisconnected)
            }

            fn end(&mut self) -> Result<()> {
                Ok(())
            }
        }

        assert!(matches!(
            Disconnected.write_escaped("untrusted <text>"),
            Err(HandlerError::ClientDisconnected)
        ));
        assert!(matches!(
            Disconnected.write_escaped_attribute("title", "untrusted <text>"),
            Err(HandlerError::ClientDisconnected)
        ));
    }
}
