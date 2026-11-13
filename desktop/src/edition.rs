//! A company edition's card in the Character list (ADR-0026): asked once per edition box, shown only for that box.

use base64::Engine;

use crate::client::EditionCard;
use crate::status::Device;

/// The edition whose card to ask the daemon for now: the connected box's, when no card of it is kept and none is being
/// asked for.
pub fn to_ask(device: &Device, kept: Option<&EditionCard>, asking: Option<&str>) -> Option<String> {
    let id = device.edition.as_ref().filter(|_| device.connected)?.id.as_str();
    (kept.is_none_or(|card| card.id != id) && asking != Some(id)).then(|| id.to_owned())
}

/// Whether the kept card is the connected box's; an ordinary box shows none.
pub fn shown(device: Option<&Device>, kept: &EditionCard) -> bool {
    device.is_some_and(|device| device.connected && device.edition.as_ref().is_some_and(|edition| edition.id == kept.id))
}

/// The look's bytes; None when there is none or it isn't base64.
pub fn look(card: &EditionCard) -> Option<Vec<u8>> {
    base64::engine::general_purpose::STANDARD.decode(card.look.as_deref()?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::status::Edition;

    fn device(edition: Option<&str>) -> Device {
        Device { connected: true, edition: edition.map(|id| Edition { id: id.to_owned() }), ..Device::default() }
    }

    fn card(look: Option<&str>) -> EditionCard {
        EditionCard { id: "luna".to_owned(), name: "Test Co".to_owned(), summary: "Our buddy".to_owned(), look: look.map(str::to_owned) }
    }

    #[test]
    fn an_edition_box_has_its_card_asked_once() {
        assert_eq!(to_ask(&device(None), None, None), None);
        assert_eq!(to_ask(&device(Some("luna")), None, None).as_deref(), Some("luna"));
        assert_eq!(to_ask(&device(Some("luna")), None, Some("luna")), None);
        assert_eq!(to_ask(&device(Some("luna")), Some(&card(None)), None), None);
        let gone = Device { connected: false, ..device(Some("luna")) };
        assert_eq!(to_ask(&gone, None, None), None);
    }

    #[test]
    fn the_card_shows_only_for_its_box() {
        assert!(shown(Some(&device(Some("luna"))), &card(None)));
        assert!(!shown(Some(&device(None)), &card(None)));
        assert!(!shown(Some(&device(Some("acme"))), &card(None)));
        assert!(!shown(None, &card(None)));
    }

    #[test]
    fn the_cards_look_draws_as_a_characters() {
        let card = serde_json::from_str::<EditionCard>(r#"{"id":"luna","name":"Test Co","summary":"Our buddy","look":null}"#).unwrap();
        assert_eq!(look(&card), None);
        let pack = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../characters/luna/pack.bin")).unwrap();
        let shipped = crate::character::Pack::parse(&pack).unwrap().look.unwrap();
        let encoded = base64::engine::general_purpose::STANDARD.encode(&shipped);
        let frames = look(&self::card(Some(&encoded))).and_then(|look| crate::character::look_frames(&look)).unwrap();
        assert!(!frames.is_empty());
    }
}
