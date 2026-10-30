#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Field {
    Package,
    Tag,
    Message,
}

impl Field {
    pub(crate) const ALL: [Self; 3] = [Self::Package, Self::Tag, Self::Message];

    pub(crate) fn from_key(key: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|field| key.eq_ignore_ascii_case(field.key()))
    }

    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Package => "package",
            Self::Tag => "tag",
            Self::Message => "message",
        }
    }

    fn description(self, excluded: bool) -> &'static str {
        match (self, excluded) {
            (Self::Package, false) => "按应用包名筛选",
            (Self::Tag, false) => "按 Tag 筛选",
            (Self::Message, false) => "按日志正文筛选",
            (Self::Package, true) => "排除指定包名的日志",
            (Self::Tag, true) => "排除指定 Tag 的日志",
            (Self::Message, true) => "排除包含指定正文的日志",
        }
    }
}

pub(crate) struct Completion {
    pub prefix: String,
    pub description: &'static str,
    pub query: String,
    pub cursor: usize,
}

pub(crate) fn completions(query: &str, cursor: usize) -> Vec<Completion> {
    let Some((start, end)) = field_range(query, cursor) else {
        return Vec::new();
    };
    let typed = query[start..cursor].to_ascii_lowercase();
    [false, true]
        .into_iter()
        .flat_map(|excluded| Field::ALL.map(|field| (field, excluded)))
        .filter_map(|(field, excluded)| {
            let prefix = format!("{}{}:", if excluded { "-" } else { "" }, field.key());
            if !prefix.starts_with(&typed) || typed == prefix {
                return None;
            }
            Some(Completion {
                cursor: start + prefix.len(),
                query: format!("{}{}{}", &query[..start], prefix, &query[end..]),
                prefix,
                description: field.description(excluded),
            })
        })
        .collect()
}

// Only replace a field key at the caret; quoted values and other terms stay intact.
fn field_range(query: &str, cursor: usize) -> Option<(usize, usize)> {
    let before = query.get(..cursor)?;
    let mut start = 0;
    let mut quote = None;
    let mut escaped = false;
    for (offset, ch) in before.char_indices() {
        if escaped {
            escaped = false;
        } else if ch == '\\' && quote != Some('\'') {
            escaped = true;
        } else if quote == Some(ch) {
            quote = None;
        } else if quote.is_none() {
            if ch == '\'' || ch == '"' {
                quote = Some(ch);
            } else if ch.is_whitespace() {
                start = offset + ch.len_utf8();
            }
        }
    }
    if quote.is_some() || escaped {
        return None;
    }
    let end = query[cursor..]
        .find(char::is_whitespace)
        .map(|length| cursor + length)
        .unwrap_or(query.len());
    let key = query[start..end]
        .strip_suffix(':')
        .unwrap_or(&query[start..end]);
    let key = key.strip_prefix('-').unwrap_or(key);
    key.chars()
        .all(|ch| ch.is_ascii_alphabetic())
        .then_some((start, end))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggests_supported_fields_and_exclusions() {
        let choices = completions("", 0);
        assert_eq!(choices.len(), 6);
        for choice in choices {
            assert!(Field::ALL.iter().any(|field| {
                choice.prefix.trim_start_matches('-') == format!("{}:", field.key())
            }));
        }
        assert_eq!(completions("pa", 2)[0].query, "package:");
        assert_eq!(completions("-ta", 3)[0].query, "-tag:");
        assert_eq!(completions("-", 1).len(), 3);
        assert!(completions("unknown", 7).is_empty());
        assert!(completions("package:", 8).is_empty());
    }

    #[test]
    fn completes_at_caret_and_preserves_surrounding_terms() {
        let query = "package:com.example -me timeout";
        let cursor = query.find("-me").unwrap() + 3;
        let choice = &completions(query, cursor)[0];
        assert_eq!(choice.query, "package:com.example -message: timeout");
        assert_eq!(
            &choice.query[..choice.cursor],
            "package:com.example -message:"
        );
        assert_eq!(
            completions("pack tag:Network", 2)[0].query,
            "package: tag:Network"
        );
        assert_eq!(completions("timeout ", 8).len(), 6);
    }

    #[test]
    fn leaves_values_quotes_and_escaped_spaces_untouched() {
        for query in [
            "package:com.example",
            "message:\"-pa",
            "message:\"network timeout\"",
            "\"network pa",
            "network\\ pa",
        ] {
            assert!(completions(query, query.len()).is_empty(), "{query}");
        }
        let query = "message:\"请求失败\" -p";
        let choice = &completions(query, query.len())[0];
        assert_eq!(choice.query, "message:\"请求失败\" -package:");
        assert_eq!(choice.cursor, choice.query.len());
        assert!(completions("请求", 1).is_empty());
        assert!(completions("pa", 10).is_empty());
    }
}
