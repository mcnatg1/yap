#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VadKind {
    Speech,
    Silence,
}

#[cfg(test)]
mod tests {
    use super::VadKind;

    #[test]
    fn vad_kind_serializes_with_snake_case_names() {
        assert_eq!(
            serde_json::to_string(&VadKind::Speech).expect("speech should serialize"),
            "\"speech\""
        );
        assert_eq!(
            serde_json::to_string(&VadKind::Silence).expect("silence should serialize"),
            "\"silence\""
        );
    }
}
