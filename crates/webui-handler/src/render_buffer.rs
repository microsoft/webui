// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Lazy growth for render collections that commonly hold one large value.

/// Start at one element and double on demand, without the four-element minimum.
#[inline]
pub(crate) fn push<T>(values: &mut Vec<T>, value: T) {
    if values.len() == values.capacity() {
        values.reserve_exact(values.capacity().max(1));
    }
    values.push(value);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shallow_storage_is_lazy_and_reused_before_growing() {
        let mut values = Vec::new();
        assert_eq!(values.capacity(), 0);
        push(&mut values, [1usize; 16]);
        assert_eq!(values.capacity(), 1);
        let allocation = values.as_ptr();
        assert_eq!(values.pop(), Some([1; 16]));
        push(&mut values, [2; 16]);
        assert_eq!(values.as_ptr(), allocation);
        assert_eq!(values.capacity(), 1);
        push(&mut values, [3; 16]);
        assert_eq!(values.capacity(), 2);
        for value in 4..40 {
            push(&mut values, [value; 16]);
        }
        assert_eq!(values.len(), 38);
        for value in (2..40).rev() {
            assert_eq!(values.pop(), Some([value; 16]));
        }
        let allocation = values.as_ptr();
        let capacity = values.capacity();
        push(&mut values, [40; 16]);
        assert_eq!(values.as_ptr(), allocation);
        assert_eq!(values.capacity(), capacity);
    }
}
