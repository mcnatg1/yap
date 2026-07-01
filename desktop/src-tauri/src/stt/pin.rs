#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrispasrPin {
    pub crispasr_version: String,
    pub binary_sha256: String,
    pub gguf_repo: String,
    pub gguf_revision: String,
    pub gguf_file: String,
    pub gguf_sha256: String,
}

pub const PIN_TEXT: &str = include_str!("../../../crispasr-version.txt");

pub fn load_pin() -> Result<CrispasrPin, String> {
    parse_pin(PIN_TEXT)
}

pub fn parse_pin(text: &str) -> Result<CrispasrPin, String> {
    let mut version = None;
    let mut binary_sha = None;
    let mut repo = None;
    let mut revision = None;
    let mut file = None;
    let mut gguf_sha = None;

    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = line
            .split_once('=')
            .ok_or_else(|| format!("crispasr-version.txt: missing '=' in line: {line}"))?;
        let value = value.trim().to_string();
        match key.trim() {
            "crispasr_version" => version = Some(value),
            "binary_sha256" => binary_sha = Some(value),
            "gguf_repo" => repo = Some(value),
            "gguf_revision" => revision = Some(value),
            "gguf_file" => file = Some(value),
            "gguf_sha256" => gguf_sha = Some(value),
            other => return Err(format!("crispasr-version.txt: unknown key: {other}")),
        }
    }

    let require = |field: Option<String>, name: &str| {
        field.ok_or_else(|| format!("crispasr-version.txt: missing key: {name}"))
    };
    let binary_sha256 = require(binary_sha, "binary_sha256")?;
    let gguf_sha256 = require(gguf_sha, "gguf_sha256")?;
    if !is_sha256(&binary_sha256) {
        return Err("crispasr-version.txt: binary_sha256 must be 64 hex chars".into());
    }
    if !is_sha256(&gguf_sha256) {
        return Err("crispasr-version.txt: gguf_sha256 must be 64 hex chars".into());
    }

    Ok(CrispasrPin {
        crispasr_version: require(version, "crispasr_version")?,
        binary_sha256,
        gguf_repo: require(repo, "gguf_repo")?,
        gguf_revision: require(revision, "gguf_revision")?,
        gguf_file: require(file, "gguf_file")?,
        gguf_sha256,
    })
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
# comment line
crispasr_version=0.4.6
binary_sha256=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
gguf_repo=cstr/cohere-transcribe-03-2026-GGUF
gguf_revision=1111111111111111111111111111111111111111
gguf_file=cohere-transcribe-q4_k.gguf
gguf_sha256=bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb
";

    #[test]
    fn parses_valid_pin() {
        let pin = parse_pin(SAMPLE).unwrap();
        assert_eq!(pin.crispasr_version, "0.4.6");
        assert_eq!(pin.gguf_repo, "cstr/cohere-transcribe-03-2026-GGUF");
        assert_eq!(pin.gguf_file, "cohere-transcribe-q4_k.gguf");
    }

    #[test]
    fn rejects_missing_key() {
        assert!(parse_pin("crispasr_version=0.4.6\n").is_err());
    }

    #[test]
    fn rejects_non_hex_sha() {
        let bad = SAMPLE.replace(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "zzzz",
        );
        assert!(parse_pin(&bad).is_err());
    }
}
