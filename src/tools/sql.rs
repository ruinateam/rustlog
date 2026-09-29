//! Pieces of SQL that cannot be bound as parameters because they are
//! spliced into several statements.

/// A comma-separated list of quoted string literals, for `IN (...)`.
pub fn string_list(values: &[String]) -> String {
    values
        .iter()
        .map(|value| format!("'{}'", value.replace('\\', "\\\\").replace('\'', "\\'")))
        .collect::<Vec<_>>()
        .join(",")
}

#[cfg(test)]
mod tests {
    use super::string_list;

    #[test]
    fn escapes_quotes_and_backslashes() {
        let values = ["plain".to_owned(), r"it's\".to_owned()];
        assert_eq!(string_list(&values), r"'plain','it\'s\\'");
    }
}
