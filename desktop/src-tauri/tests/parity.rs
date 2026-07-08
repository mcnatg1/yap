use yap_desktop_lib::stt::parity::parse_verbose_json_has_timestamps;

const MOCK_VERBOSE_JSON: &str = include_str!("fixtures/parity-contract.verbose.json");

#[test]
fn mock_verbose_json_contract_carries_timestamps() {
    assert!(
        parse_verbose_json_has_timestamps(MOCK_VERBOSE_JSON),
        "mock verbose_json contract must include segment or word timestamps"
    );
}
