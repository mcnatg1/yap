use crate::audio::evidence::{ClientSpeakerAttribution, SpeakerTurn};

mod revision;
mod wire;

pub(super) fn hash(value: char) -> String {
    value.to_string().repeat(64)
}

pub(super) fn turn() -> SpeakerTurn {
    SpeakerTurn::new(
        "turn-1",
        0,
        20,
        ClientSpeakerAttribution::unknown(),
        Some(0.8),
    )
    .unwrap()
}
