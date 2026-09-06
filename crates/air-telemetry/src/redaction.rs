use url::Url;

const SENSITIVE_KEYS: &[&str] = &[
    "secret",
    "token",
    "password",
    "authorization",
    "authentication",
];

pub fn redact_log_value(input: &str) -> String {
    let mut output = input.to_owned();
    for key in SENSITIVE_KEYS {
        output = redact_key_value(&output, key);
    }
    output = redact_url_queries(&output);
    output = redact_local_paths(&output);
    output
}

fn redact_key_value(input: &str, key: &str) -> String {
    let lower = input.to_ascii_lowercase();
    let mut result = String::with_capacity(input.len());
    let mut cursor = 0;
    while let Some(relative) = lower[cursor..].find(key) {
        let key_start = cursor + relative;
        let key_end = key_start + key.len();
        result.push_str(&input[cursor..key_end]);
        let mut value_start = key_end;
        while let Some(ch) = input[value_start..].chars().next() {
            if ch == ':' || ch == '=' || ch.is_whitespace() || ch == '"' || ch == '\'' {
                result.push(ch);
                value_start += ch.len_utf8();
            } else {
                break;
            }
        }
        let mut value_end = value_start;
        while let Some(ch) = input[value_end..].chars().next() {
            if ch == '&' || ch == ',' || ch == '}' || ch == ']' || ch.is_whitespace() {
                break;
            }
            value_end += ch.len_utf8();
        }
        if value_end > value_start {
            result.push_str("***");
        }
        cursor = value_end;
    }
    result.push_str(&input[cursor..]);
    result
}

fn redact_url_queries(input: &str) -> String {
    input
        .split_whitespace()
        .map(|part| {
            let trimmed = part.trim_matches(|ch| ch == ',' || ch == ';');
            match Url::parse(trimmed) {
                Ok(mut url) if url.query().is_some() => {
                    let keys: Vec<String> =
                        url.query_pairs().map(|(key, _)| key.into_owned()).collect();
                    url.query_pairs_mut().clear();
                    for key in keys {
                        url.query_pairs_mut().append_pair(&key, "***");
                    }
                    part.replace(trimmed, url.as_str())
                }
                _ => part.to_owned(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// 脱敏本地文件路径中的用户目录部分。
///
/// 策略：匹配 Windows 用户路径 `<盘符>:\Users\<name>\` 和 Unix 家目录
/// `/home/<name>/`、`/Users/<name>/`，将用户名替换为 `***`，保留其余路径
/// 结构以便排查问题。
///
/// 例如：`C:\Users\Administrator\data\config` → `C:\Users\***\data\config`
/// 例如：`/home/myuser/.config` → `/home/***/.config`
/// 非用户目录路径（如 `D:\code\air`）不做脱敏。
fn redact_local_paths(input: &str) -> String {
    let mut result = input.to_owned();

    // Windows: <盘符>:\Users\<用户名>\
    // 多次扫描替换，因为一条消息中可能包含多个用户路径
    result = redact_path_segment(&result, r"\Users\", '\\');
    // Unix: /home/<用户名>/ 或 /Users/<用户名>/
    result = redact_path_segment(&result, "/home/", '/');
    result = redact_path_segment(&result, "/Users/", '/');

    result
}

/// 在文本中查找 `prefix` 后的用户名段并替换为 `***`。
///
/// `prefix` 是路径中用户目录的前缀（如 `\Users\`），`sep` 是路径分隔符
/// （Windows 为 `\`，Unix 为 `/`）。用户名是从 prefix 结尾到下一个分隔符之间的内容。
fn redact_path_segment(input: &str, prefix: &str, sep: char) -> String {
    let mut result = String::with_capacity(input.len());
    let mut cursor = 0;
    let lower_input = input.to_ascii_lowercase();
    let lower_prefix = prefix.to_ascii_lowercase();

    while cursor < input.len() {
        // 在剩余文本中查找前缀（大小写不敏感）
        if let Some(relative) = lower_input[cursor..].find(&lower_prefix) {
            let prefix_start = cursor + relative;
            let prefix_end = prefix_start + prefix.len();

            // 复制前缀之前的文本和前缀本身
            result.push_str(&input[cursor..prefix_end]);

            // 找到用户名结束位置（下一个分隔符或非路径字符）
            let mut name_end = prefix_end;
            while name_end < input.len() {
                let ch = input.as_bytes()[name_end] as char;
                if ch == sep || ch.is_whitespace() || ch == ']' || ch == '}' || ch == ',' {
                    break;
                }
                name_end += ch.len_utf8();
            }

            // 将用户名替换为 ***
            if name_end > prefix_end {
                result.push_str("***");
            }

            cursor = name_end;
        } else {
            result.push_str(&input[cursor..]);
            break;
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_named_secret_fields() {
        let redacted =
            redact_log_value("secret=abc token: def password=\"p1\" authorization Bearer");
        assert!(!redacted.contains("abc"));
        assert!(!redacted.contains("def"));
        assert!(!redacted.contains("p1"));
        assert!(redacted.contains("secret=***"));
    }

    #[test]
    fn redacts_mihomo_authentication_fields() {
        let redacted = redact_log_value(r#"{"authentication":["user:password"]}"#);

        assert!(redacted.contains("authentication"));
        assert!(!redacted.contains("user:password"));
    }

    #[test]
    fn redacts_subscription_url_query_values() {
        let redacted = redact_log_value("fetch https://example.test/sub?token=abc&user=bob");
        assert!(redacted.contains("token=***"));
        assert!(redacted.contains("user=***"));
        assert!(!redacted.contains("abc"));
        assert!(!redacted.contains("bob"));
    }

    #[test]
    fn redacts_windows_user_path() {
        let redacted = redact_log_value(
            r#"path is not subpath of home directory or SAFE_PATHS:  C:\Users\Administrator\data\config\core.runtime.config.yaml"#,
        );
        // 用户名被遮蔽
        assert!(!redacted.contains("Administrator"));
        // 路径结构和文件名保留
        assert!(redacted.contains("C:\\Users\\***\\data\\config\\core.runtime.config.yaml"));
    }

    #[test]
    fn redacts_unix_home_path() {
        let redacted =
            redact_log_value("config loaded from /home/myuser/.config/mihomo/config.yaml");
        assert!(!redacted.contains("myuser"));
        assert!(redacted.contains("/home/***/.config/mihomo/config.yaml"));
    }

    #[test]
    fn redacts_macos_users_path() {
        let redacted =
            redact_log_value("error in /Users/john/Library/Application Support/air/config.yaml");
        assert!(!redacted.contains("john"));
        assert!(redacted.contains("/Users/***/Library/Application Support/air/config.yaml"));
    }

    #[test]
    fn non_user_path_not_redacted() {
        // 非用户目录路径不做脱敏（如 D:\code\air 这类开发路径）
        let redacted = redact_log_value("D:\\code\\air\\data\\config\\core.runtime.config.yaml");
        assert!(redacted.contains("D:\\code\\air\\data\\config\\core.runtime.config.yaml"));
    }

    #[test]
    fn redacts_multiple_user_paths_in_one_message() {
        let redacted =
            redact_log_value("allowed paths: [C:\\Users\\Admin\\data C:\\Users\\Admin\\cache]");
        assert!(!redacted.contains("Admin"));
        assert!(redacted.contains("C:\\Users\\***\\data"));
        assert!(redacted.contains("C:\\Users\\***\\cache"));
    }

    #[test]
    fn redacts_mihomo_safe_paths_error_body() {
        // 模拟用户报告的实际 mihomo 400 错误
        // 注意：JSON 字符串中反斜杠已转义为 \\，所以原始文本中的路径
        // 形如 C:\Users\Admin\，redact_local_paths 匹配 \Users\ 前缀
        let input = r#"{"message":"path is not subpath of home directory or SAFE_PATHS:  C:\Users\Admin\data\config\core.runtime.config.yaml \n allowed paths:  [C:\Users\Admin\data\cache\core C:\Users\Admin\data\config  C:\Users\Admin\data\data C:\Users\Admin\data\cache]"}"#;
        let redacted = redact_log_value(input);
        assert!(!redacted.contains("Admin"));
        assert!(redacted.contains("core.runtime.config.yaml"));
    }

    #[test]
    fn path_segment_redaction_case_insensitive() {
        // Windows 路径前缀大小写不敏感
        let redacted = redact_log_value("c:\\users\\TestUser\\data\\file.txt");
        assert!(!redacted.contains("TestUser"));
        assert!(redacted.contains("c:\\users\\***\\data\\file.txt"));
    }
}
