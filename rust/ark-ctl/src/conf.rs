//! `read_conf_value` and `write_conf_value` of the manager, on text instead of files.

/// Last `KEY=` line's value (everything after the first `=`); `default` when missing or empty.
pub fn read_conf_value(text: &str, key: &str, default: &str) -> String {
    let prefix = format!("{key}=");
    let val = text
        .split('\n')
        .rfind(|l| l.starts_with(&prefix))
        .map_or("", |l| &l[prefix.len()..]);
    if val.is_empty() {
        default.to_string()
    } else {
        val.to_string()
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum WriteError {
    Newline,
}

/// Replace `KEY=...` on every line that has it, otherwise append `KEY=VALUE`. A value with a newline is refused.
/// Like `echo >>`, an append goes straight after a last line that has no newline.
pub fn write_conf_value(text: &str, key: &str, value: &str) -> Result<String, WriteError> {
    if value.contains('\n') {
        return Err(WriteError::Newline);
    }
    let prefix = format!("{key}=");
    if text.split('\n').any(|l| l.starts_with(&prefix)) {
        let replaced: Vec<String> = text
            .split('\n')
            .map(|l| {
                if l.starts_with(&prefix) {
                    format!("{key}={value}")
                } else {
                    l.to_string()
                }
            })
            .collect();
        Ok(replaced.join("\n"))
    } else {
        Ok(format!("{text}{key}={value}\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_takes_the_last_match_and_the_default() {
        let t = "A=1\nB=x=y\nA=2\nC=\n";
        assert_eq!(read_conf_value(t, "A", "d"), "2");
        assert_eq!(read_conf_value(t, "B", "d"), "x=y");
        assert_eq!(read_conf_value(t, "C", "d"), "d");
        assert_eq!(read_conf_value(t, "Z", "d"), "d");
        assert_eq!(read_conf_value("AB=1\n", "B", "d"), "d");
    }

    #[test]
    fn write_replaces_every_match_and_appends_otherwise() {
        assert_eq!(
            write_conf_value("A=1\nB=2\nA=3\n", "A", "9").unwrap(),
            "A=9\nB=2\nA=9\n"
        );
        assert_eq!(write_conf_value("A=1\n", "B", "2").unwrap(), "A=1\nB=2\n");
        assert_eq!(write_conf_value("A=1", "B", "2").unwrap(), "A=1B=2\n");
        assert_eq!(write_conf_value("", "B", "2").unwrap(), "B=2\n");
    }

    #[test]
    fn special_characters_stay_literal() {
        assert_eq!(
            write_conf_value("P=old\n", "P", "a|b&c\\d").unwrap(),
            "P=a|b&c\\d\n"
        );
    }

    #[test]
    fn newline_values_are_refused() {
        assert_eq!(
            write_conf_value("A=1\n", "A", "x\ny"),
            Err(WriteError::Newline)
        );
    }
}
