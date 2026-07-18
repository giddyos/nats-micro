/// Returns one dot-delimited subject segment without allocating.
#[inline]
#[must_use]
pub fn segment(subject: &str, wanted: usize) -> Option<&str> {
    let bytes = subject.as_bytes();
    let mut segment_start = 0usize;
    let mut current = 0usize;
    let mut index = 0usize;

    while index <= bytes.len() {
        if index == bytes.len() || bytes[index] == b'.' {
            if current == wanted {
                return subject.get(segment_start..index);
            }
            current += 1;
            segment_start = index + 1;
        }
        index += 1;
    }

    None
}

/// Converts one statically located subject segment into a handler argument.
pub trait FromSubject<'a>: Sized {
    type Error: std::fmt::Display;

    fn from_subject(value: &'a str) -> Result<Self, Self::Error>;
}

impl<'a> FromSubject<'a> for &'a str {
    type Error = std::convert::Infallible;

    #[inline]
    fn from_subject(value: &'a str) -> Result<Self, Self::Error> {
        Ok(value)
    }
}

impl FromSubject<'_> for String {
    type Error = std::convert::Infallible;

    #[inline]
    fn from_subject(value: &str) -> Result<Self, Self::Error> {
        Ok(value.to_owned())
    }
}

macro_rules! impl_from_subject {
    ($($ty:ty),* $(,)?) => {
        $(
            impl FromSubject<'_> for $ty {
                type Error = <$ty as std::str::FromStr>::Err;

                #[inline]
                fn from_subject(value: &str) -> Result<Self, Self::Error> {
                    value.parse()
                }
            }
        )*
    };
}

impl_from_subject!(
    u8, u16, u32, u64, u128, usize, i8, i16, i32, i64, i128, isize, f32, f64,
);

#[cfg(test)]
mod tests {
    use super::{FromSubject, segment};

    #[test]
    fn extracts_segments_without_normalizing_the_subject() {
        assert_eq!(segment("users.v1.users.42.orders.7", 0), Some("users"));
        assert_eq!(segment("users.v1.users.42.orders.7", 3), Some("42"));
        assert_eq!(segment("users.v1.users.42.orders.7", 5), Some("7"));
        assert_eq!(segment("users.v1.users.42.orders.7", 6), None);
        assert_eq!(segment("a..b", 1), Some(""));
    }

    #[test]
    fn parses_borrowed_and_numeric_parameters() {
        assert_eq!(<&str>::from_subject("user-7").unwrap(), "user-7");
        assert_eq!(u64::from_subject("42").unwrap(), 42);
        assert!(u64::from_subject("nope").is_err());
    }
}
