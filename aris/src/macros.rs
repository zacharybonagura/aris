/// Table of ASCII characters, macros, and their corresponding logic symbols.
/// The format of each row is `(symbol, macros)`.
pub static TABLE: [(&str, &[&str]); 10] = [
    ("⊥", &[".con", "_|_"]),
    ("⊤", &[".taut", "^|^"]),
    ("¬", &[".not", "~"]),
    ("∀", &["forall"]),
    ("∃", &["exists"]),
    ("∧", &[".and", "&", r#"/\"#]),
    ("∨", &[".or", "|", r#"\/"#]),
    ("↔", &[".bicon", "<->"]),
    ("→", &[".impl", "->"]),
    ("≡", &[".equiv", "==="]),
];

/// Convert ASCII characters and macros to logic symbols.
///
/// ```rust
/// assert_eq!(aris::macros::expand("_|_ -> (^|^ .bicon ~P)"), "⊥ → (⊤ ↔ ¬P)");
/// ```
pub fn expand(s: &str) -> String {
    TABLE.iter().fold(s.to_string(), |s, (symbol, macros)| {
        macros.iter().fold(s, |s, macro_| s.replace(macro_, symbol))
    })
}
