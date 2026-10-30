use crate::log_filter::Field;
use ikaros_domain::LogEntry;

struct Term {
    field: Field,
    value: String,
    excluded: bool,
}

pub(crate) struct LogQuery {
    terms: Vec<Term>,
    text: String,
}

impl LogQuery {
    pub(crate) fn parse(query: &str) -> Self {
        let words = shlex::split(query)
            .unwrap_or_else(|| query.split_whitespace().map(str::to_owned).collect());
        let mut words = words.iter().peekable();
        let mut terms = Vec::new();
        let mut text = Vec::new();
        while let Some(word) = words.next() {
            let Some((field, excluded, value)) = field_value(word) else {
                if word != "&" {
                    text.push(word.as_str());
                }
                continue;
            };
            let value = if value.is_empty() {
                words
                    .next_if(|next| field_value(next).is_none() && next.as_str() != "&")
                    .map(|word| word.as_str())
                    .unwrap_or("")
            } else {
                value
            };
            if !value.is_empty() {
                terms.push(Term {
                    field,
                    value: value.to_lowercase(),
                    excluded,
                });
            }
        }
        Self {
            terms,
            text: text.join(" ").to_lowercase(),
        }
    }

    pub(crate) fn has_positive_filter(&self, field: Field) -> bool {
        self.terms
            .iter()
            .any(|term| term.field == field && !term.excluded)
    }

    pub(crate) fn append_filter(query: &str, field: Field, value: &str) -> Option<String> {
        if value.trim().is_empty() {
            return None;
        }
        if Self::parse(query).terms.iter().any(|term| {
            term.field == field && !term.excluded && term.value == value.to_lowercase()
        }) {
            return Some(query.to_string());
        }
        let prefix = if shlex::split(query).is_some() {
            query.trim_end().to_string()
        } else {
            // Preserve the parser's fallback tokens when an existing quote is incomplete.
            shlex::try_join(query.split_whitespace()).ok()?
        };
        let term = format!("{}:{}", field.key(), shlex::try_quote(value).ok()?);
        Some(if prefix.trim().is_empty() {
            term
        } else {
            format!("{prefix} {term}")
        })
    }

    pub(super) fn matches(&self, log: &LogEntry) -> bool {
        if self.terms.is_empty() && self.text.is_empty() {
            return true;
        }
        if !self.text.is_empty()
            && ![
                Some(log.message.as_str()),
                Some(log.tag.as_str()),
                log.package_name.as_deref(),
                log.process_name.as_deref(),
            ]
            .into_iter()
            .flatten()
            .any(|value| contains(value, &self.text))
        {
            return false;
        }
        for field in Field::ALL {
            let value = match field {
                Field::Package => log.package_name.as_deref().unwrap_or(""),
                Field::Tag => &log.tag,
                Field::Message => &log.message,
            };
            let mut positives = self
                .terms
                .iter()
                .filter(|term| term.field == field && !term.excluded)
                .peekable();
            if positives.peek().is_some() && !positives.any(|term| contains(value, &term.value)) {
                return false;
            }
            if self
                .terms
                .iter()
                .any(|term| term.field == field && term.excluded && contains(value, &term.value))
            {
                return false;
            }
        }
        true
    }
}

fn field_value(word: &str) -> Option<(Field, bool, &str)> {
    let (key, value) = word.split_once(':')?;
    let (key, excluded) = key
        .strip_prefix('-')
        .map(|key| (key, true))
        .unwrap_or((key, false));
    let field = Field::from_key(key)?;
    Some((field, excluded, value))
}

fn contains(value: &str, needle: &str) -> bool {
    value.to_lowercase().contains(needle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ikaros_domain::LogLevel;

    fn log() -> LogEntry {
        LogEntry {
            timestamp: String::new(),
            pid: 123,
            tid: 456,
            package_name: Some("com.example.app".into()),
            process_name: Some("com.example.app:worker".into()),
            level: LogLevel::Info,
            tag: "Network".into(),
            message: "Network timeout: 请求失败".into(),
        }
    }

    #[test]
    fn combines_package_and_keywords_and_preserves_plain_phrase_search() {
        assert!(LogQuery::parse("package:com.example TIMEOUT").matches(&log()));
        assert!(!LogQuery::parse("package:com.other timeout").matches(&log()));
        assert!(!LogQuery::parse("package:com.example success").matches(&log()));
        assert!(LogQuery::parse("network timeout").matches(&log()));
        assert!(LogQuery::parse("请求失败").matches(&log()));
        assert!(LogQuery::parse("").matches(&log()));
    }

    #[test]
    fn supports_quoted_values_key_spacing_and_combined_fields() {
        assert!(
            LogQuery::parse("package: com.example & message:\"network timeout:\" tag:Network")
                .matches(&log())
        );
        assert!(
            !LogQuery::parse("package:com.example message:timeout tag:Database").matches(&log())
        );
    }

    #[test]
    fn repeated_positive_keys_are_or_and_exclusions_are_and() {
        assert!(
            LogQuery::parse("package:com.other package:com.example message:timeout")
                .matches(&log())
        );
        assert!(
            LogQuery::parse("package:com.example -tag:Database -message:success").matches(&log())
        );
        assert!(!LogQuery::parse("package:com.example -message:timeout").matches(&log()));
        assert!(!LogQuery::parse("-package:com.example").matches(&log()));
    }

    #[test]
    fn missing_ownership_does_not_match_a_package_based_on_process_name_or_message() {
        let mut entry = log();
        entry.package_name = None;
        entry.message = "com.example.app timeout".into();
        assert!(!LogQuery::parse("package:com.example timeout").matches(&entry));
        assert!(LogQuery::parse("timeout").matches(&entry));
        assert!(LogQuery::parse("com.example.app:worker").matches(&entry));
        assert!(LogQuery::parse("package:").matches(&entry));
    }

    #[test]
    fn column_filters_recognize_nonempty_positive_fields() {
        let query =
            LogQuery::parse("PACKAGE: com.example tag:\"Activity Manager\" message:timeout");
        assert!(query.has_positive_filter(Field::Package));
        assert!(query.has_positive_filter(Field::Tag));
        let query = LogQuery::parse("package:com.first package:com.second -tag:Network");
        assert!(query.has_positive_filter(Field::Package));
        assert!(!query.has_positive_filter(Field::Tag));
    }

    #[test]
    fn empty_excluded_and_quoted_message_fields_do_not_hide_columns() {
        for text in [
            "",
            "timeout",
            "package: tag:",
            "package:\"\" tag:''",
            "-package:com.example -tag:Network",
            "message:\"package:com.example tag:Network\"",
        ] {
            let query = LogQuery::parse(text);
            assert!(!query.has_positive_filter(Field::Package), "{text}");
            assert!(!query.has_positive_filter(Field::Tag), "{text}");
        }
    }

    #[test]
    fn context_filters_preserve_existing_terms_and_quote_values() {
        let original = "package:com.example -message:success timeout";
        let mut entry = log();
        entry.tag = "Network \"worker\" 'quoted'\nline".into();
        let query = LogQuery::append_filter(original, Field::Tag, &entry.tag).unwrap();
        assert!(query.starts_with(original));
        assert!(LogQuery::parse(&query).matches(&entry));
        assert_eq!(LogQuery::append_filter(&query, Field::Tag, &entry.tag).unwrap(), query);
        entry.tag = "Other".into();
        assert!(!LogQuery::parse(&query).matches(&entry));

        entry.message = "first \"quoted\" line\nsecond: line".into();
        let query = LogQuery::append_filter("", Field::Message, &entry.message).unwrap();
        assert!(LogQuery::parse(&query).matches(&entry));
        entry.message = "first line".into();
        assert!(!LogQuery::parse(&query).matches(&entry));
    }

    #[test]
    fn context_filters_handle_empty_duplicate_and_incomplete_queries() {
        assert!(LogQuery::append_filter("timeout", Field::Tag, " ").is_none());
        let query = "TAG:\"Network Worker\" timeout";
        assert_eq!(LogQuery::append_filter(query, Field::Tag, "network worker").unwrap(), query);
        let query = LogQuery::append_filter("message:\"unfinished", Field::Package, "com.example").unwrap();
        assert!(shlex::split(&query).is_some());
        assert!(LogQuery::parse(&query).has_positive_filter(Field::Package));
    }
}
