// 文字列操作関数の実装
pub fn len(s: &str) -> usize {
    s.len()
}

pub fn sub(s: &str, start: usize, end: Option<usize>) -> String {
    let end = end.unwrap_or(s.len());
    if start >= s.len() || start > end {
        return String::new();
    }
    
    s[start.min(s.len())..end.min(s.len())].to_string()
}

pub fn upper(s: &str) -> String {
    s.to_uppercase()
}

pub fn lower(s: &str) -> String {
    s.to_lowercase()
}
