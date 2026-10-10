#![doc = include_str!("../README.md")]

/// Uppercases the first character of `input`, leaving the rest untouched.
///
/// Unicode-aware: the first character is mapped with [`char::to_uppercase`],
/// which may yield multiple characters (e.g. `ß` → `SS`). Empty input returns
/// an empty `String`.
///
/// # Examples
///
/// ```
/// use core_strings::capitalize_first_letter;
///
/// assert_eq!(capitalize_first_letter("users"), "Users");
/// assert_eq!(capitalize_first_letter(""), "");
/// ```
pub fn capitalize_first_letter(input: &str) -> String {
    let mut chars = input.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_capitalize_first_letter() {
        assert_eq!(capitalize_first_letter(""), "");
        assert_eq!(capitalize_first_letter("a"), "A");
        assert_eq!(capitalize_first_letter("hello"), "Hello");
        assert_eq!(capitalize_first_letter("users"), "Users");
        assert_eq!(capitalize_first_letter("API"), "API");
        assert_eq!(capitalize_first_letter("ßeta"), "SSeta");
    }
}
