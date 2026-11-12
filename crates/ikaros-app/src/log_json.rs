// JSON 提取与拍平解析模块

use serde_json::Value;

/// 拍平后的 JSON 字段项
#[derive(Clone, Debug, PartialEq)]
pub struct JsonFieldItem {
    pub path: String,
    pub value: String,
    pub formatted_value: String,
    pub value_type: String,
}

/// 从任意日志文本中提取最外层的合法 JSON 子串
pub fn extract_json_str(text: &str) -> Option<&str> {
    let bytes = text.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        // 寻找起始符号 '{' 或 '['
        if bytes[i] == b'{' || bytes[i] == b'[' {
            let start = i;
            let open_char = bytes[i];
            let close_char = if open_char == b'{' { b'}' } else { b']' };

            let mut depth = 0;
            let mut in_str = false;
            let mut escape = false;
            let mut end = None;

            for (idx, &b) in bytes[start..].iter().enumerate() {
                let curr_idx = start + idx;

                if escape {
                    escape = false;
                    continue;
                }

                if b == b'\\' && in_str {
                    escape = true;
                    continue;
                }

                if b == b'"' {
                    in_str = !in_str;
                    continue;
                }

                if !in_str {
                    if b == open_char {
                        depth += 1;
                    } else if b == close_char {
                        depth -= 1;
                        if depth == 0 {
                            end = Some(curr_idx);
                            break;
                        }
                    }
                }
            }

            if let Some(end_idx) = end {
                let candidate = &text[start..=end_idx];
                // 验证是否是合法 JSON
                if serde_json::from_str::<Value>(candidate).is_ok() {
                    return Some(candidate);
                }
            }
        }
        i += 1;
    }

    None
}

/// 递归拍平 JSON Value，保留中间对象/数组以及叶子节点
pub fn flatten_json(val: &Value) -> Vec<JsonFieldItem> {
    let mut results = Vec::new();
    flatten_internal(val, "", &mut results);
    results
}

fn flatten_internal(val: &Value, path: &str, results: &mut Vec<JsonFieldItem>) {
    match val {
        Value::Object(map) => {
            if !path.is_empty() {
                results.push(JsonFieldItem {
                    path: path.to_string(),
                    value: serde_json::to_string(val).unwrap_or_default(),
                    formatted_value: serde_json::to_string_pretty(val).unwrap_or_default(),
                    value_type: "object".to_string(),
                });
            }
            for (key, sub_val) in map {
                let sub_path = if path.is_empty() {
                    key.clone()
                } else {
                    format!("{}.{}", path, key)
                };
                flatten_internal(sub_val, &sub_path, results);
            }
        }
        Value::Array(arr) => {
            if !path.is_empty() {
                results.push(JsonFieldItem {
                    path: path.to_string(),
                    value: serde_json::to_string(val).unwrap_or_default(),
                    formatted_value: serde_json::to_string_pretty(val).unwrap_or_default(),
                    value_type: "array".to_string(),
                });
            }
            for (index, sub_val) in arr.iter().enumerate() {
                let sub_path = format!("{}[{}]", path, index);
                flatten_internal(sub_val, &sub_path, results);
            }
        }
        Value::String(s) => {
            results.push(JsonFieldItem {
                path: path.to_string(),
                value: s.clone(),
                formatted_value: s.clone(),
                value_type: "string".to_string(),
            });
        }
        Value::Number(n) => {
            let str_val = n.to_string();
            results.push(JsonFieldItem {
                path: path.to_string(),
                value: str_val.clone(),
                formatted_value: str_val,
                value_type: "number".to_string(),
            });
        }
        Value::Bool(b) => {
            let str_val = b.to_string();
            results.push(JsonFieldItem {
                path: path.to_string(),
                value: str_val.clone(),
                formatted_value: str_val,
                value_type: "boolean".to_string(),
            });
        }
        Value::Null => {
            results.push(JsonFieldItem {
                path: path.to_string(),
                value: "null".to_string(),
                formatted_value: "null".to_string(),
                value_type: "null".to_string(),
            });
        }
    }
}

/// 解析文本中的 JSON 并返回格式化美化（pretty）字符串及拍平字段列表
pub fn parse_log_json(text: &str) -> Option<(String, Vec<JsonFieldItem>)> {
    let json_str = extract_json_str(text)?;
    let val: Value = serde_json::from_str(json_str).ok()?;
    let pretty = serde_json::to_string_pretty(&val).unwrap_or_else(|_| json_str.to_string());
    let items = flatten_json(&val);
    Some((pretty, items))
}

/// 过滤字段列表（大小写不敏感匹配 path 或 value）
pub fn filter_fields<'a>(items: &'a [JsonFieldItem], query: &str) -> Vec<&'a JsonFieldItem> {
    let trimmed = query.trim().to_lowercase();
    if trimmed.is_empty() {
        return items.iter().collect();
    }
    items
        .iter()
        .filter(|item| {
            item.path.to_lowercase().contains(&trimmed)
                || item.value.to_lowercase().contains(&trimmed)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_and_flatten_user_example() {
        let log = r#"[DeviceAuthHttp] RESPONSE_BODY id=11 json={"requestId":"KY4jGsOj","code":200,"msg":"success","developerMsg":"success","timestamp":1794214163723,"data":{"qrCode":"XXX","expiresAt":"XXX"}}"#;
        let (pretty, items) = parse_log_json(log).expect("应该成功解析出 JSON");

        assert!(pretty.contains("\"requestId\": \"KY4jGsOj\""));

        // 验证字段列表
        let req_id = items.iter().find(|i| i.path == "requestId").unwrap();
        assert_eq!(req_id.value, "KY4jGsOj");
        assert_eq!(req_id.formatted_value, "KY4jGsOj");
        assert_eq!(req_id.value_type, "string");

        let code = items.iter().find(|i| i.path == "code").unwrap();
        assert_eq!(code.value, "200");
        assert_eq!(code.value_type, "number");

        let data_qrcode = items.iter().find(|i| i.path == "data.qrCode").unwrap();
        assert_eq!(data_qrcode.value, "XXX");
        assert_eq!(data_qrcode.value_type, "string");

        let data_expires = items.iter().find(|i| i.path == "data.expiresAt").unwrap();
        assert_eq!(data_expires.value, "XXX");
        assert_eq!(data_expires.value_type, "string");

        // 中间对象 data 也应保留并具备格式化后的多行缩进
        let data_obj = items.iter().find(|i| i.path == "data").unwrap();
        assert_eq!(data_obj.value_type, "object");
        assert!(data_obj.value.contains("\"qrCode\":\"XXX\""));
        assert!(data_obj.formatted_value.contains("\"qrCode\": \"XXX\""));
    }

    #[test]
    fn test_filter_fields() {
        let items = vec![
            JsonFieldItem {
                path: "requestId".into(),
                value: "KY4jGsOj".into(),
                formatted_value: "KY4jGsOj".into(),
                value_type: "string".into(),
            },
            JsonFieldItem {
                path: "data.qrCode".into(),
                value: "http://example.com/qr".into(),
                formatted_value: "http://example.com/qr".into(),
                value_type: "string".into(),
            },
            JsonFieldItem {
                path: "code".into(),
                value: "200".into(),
                formatted_value: "200".into(),
                value_type: "number".into(),
            },
        ];

        let filtered = filter_fields(&items, "qrcode");
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].path, "data.qrCode");

        let filtered_by_val = filter_fields(&items, "example.com");
        assert_eq!(filtered_by_val.len(), 1);
        assert_eq!(filtered_by_val[0].path, "data.qrCode");

        let filtered_all = filter_fields(&items, "");
        assert_eq!(filtered_all.len(), 3);
    }

    #[test]
    fn test_plain_log_without_json() {
        let log = "10-29 12:00:00.000 com.test I TestTag: Just a normal log message";
        assert_eq!(extract_json_str(log), None);
        assert_eq!(parse_log_json(log), None);
    }

    #[test]
    fn test_json_with_nested_braces_in_string() {
        let log = r#"prefix {"msg": "bracket {here} in string", "count": 1} suffix"#;
        let (pretty, items) = parse_log_json(log).expect("应该正确处理字符串内括号");
        assert!(pretty.contains("bracket {here} in string"));
        let msg = items.iter().find(|i| i.path == "msg").unwrap();
        assert_eq!(msg.value, "bracket {here} in string");
    }
}
