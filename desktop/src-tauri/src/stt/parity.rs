pub fn word_error_rate(reference: &str, hypothesis: &str) -> f64 {
    let reference: Vec<&str> = reference.split_whitespace().collect();
    let hypothesis: Vec<&str> = hypothesis.split_whitespace().collect();
    if reference.is_empty() {
        return if hypothesis.is_empty() { 0.0 } else { 1.0 };
    }
    edit_distance(&reference, &hypothesis) as f64 / reference.len() as f64
}

fn edit_distance(a: &[&str], b: &[&str]) -> usize {
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut curr = vec![0usize; b.len() + 1];
    for (i, a_word) in a.iter().enumerate() {
        curr[0] = i + 1;
        for (j, b_word) in b.iter().enumerate() {
            let cost = if a_word.eq_ignore_ascii_case(b_word) { 0 } else { 1 };
            curr[j + 1] = (prev[j + 1] + 1).min(curr[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[b.len()]
}

pub fn parse_verbose_json_has_timestamps(body: &str) -> bool {
    let value: serde_json::Value = match serde_json::from_str(body) {
        Ok(value) => value,
        Err(_) => return false,
    };
    let segment_timing = value
        .get("segments")
        .and_then(serde_json::Value::as_array)
        .map(|items| items.iter().any(has_start_end))
        .unwrap_or(false);
    let word_timing = value
        .get("words")
        .and_then(serde_json::Value::as_array)
        .map(|items| items.iter().any(has_start_end))
        .unwrap_or(false);
    segment_timing || word_timing
}

fn has_start_end(item: &serde_json::Value) -> bool {
    item.get("start").and_then(serde_json::Value::as_f64).is_some()
        && item.get("end").and_then(serde_json::Value::as_f64).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wer_zero_for_identical() {
        assert_eq!(word_error_rate("the quick brown fox", "the quick brown fox"), 0.0);
    }

    #[test]
    fn wer_counts_one_substitution() {
        assert!((word_error_rate("the quick brown fox", "the quick green fox") - 0.25).abs() < 1e-9);
    }

    #[test]
    fn wer_is_case_insensitive() {
        assert_eq!(word_error_rate("Hello World", "hello world"), 0.0);
    }

    #[test]
    fn wer_handles_empty_reference() {
        assert_eq!(word_error_rate("", ""), 0.0);
        assert_eq!(word_error_rate("", "extra"), 1.0);
    }

    #[test]
    fn verbose_json_detects_segment_and_word_timing() {
        assert!(parse_verbose_json_has_timestamps(
            r#"{"text":"hi","segments":[{"start":0.0,"end":1.2,"text":"hi"}]}"#
        ));
        assert!(parse_verbose_json_has_timestamps(
            r#"{"text":"hi","words":[{"word":"hi","start":0.0,"end":0.4}]}"#
        ));
    }

    #[test]
    fn verbose_json_false_without_timing() {
        assert!(!parse_verbose_json_has_timestamps(r#"{"text":"hi"}"#));
        assert!(!parse_verbose_json_has_timestamps("not json"));
    }
}
