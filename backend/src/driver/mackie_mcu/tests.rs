use super::super::midi::normalise;
use super::super::testing::Recorder;
use super::*;
use crate::catalog::{by_id, identify_midi, ModelView};

fn open(id: &str) -> (MackieMcu, Recorder) {
    let recorder = Recorder::default();
    let mut deck = MackieMcu::new(by_id(id).unwrap(), Box::new(recorder.clone()));
    // Pin the Windows/Linux port naming so results do not depend on the
    // host; the macOS (unnamed) path has its own tests below.
    deck.named_ports = true;
    (deck, recorder)
}

fn face(color: [u8; 3]) -> Face {
    Face {
        label: String::new(),
        color,
        image: None,
    }
}

fn key(index: u8, pressed: bool) -> Input {
    Input::Key { index, pressed }
}

fn turn(index: u8, delta: i8) -> Input {
    Input::DialTurn { index, delta }
}

fn ready(device: u8) -> Vec<u8> {
    vec![0xf0, 0x00, 0x00, 0x66, device, 0x01, 0xf7]
}

fn no_pitch_bend(writes: &[Vec<u8>]) {
    assert!(writes.iter().all(|write| write[0] & 0xf0 != 0xe0));
}

/// Opens a deck and makes it live with its own ready answer.
fn live(id: &str, device: u8) -> (MackieMcu, Recorder) {
    let (mut deck, recorder) = open(id);
    recorder.queue_read(ready(device));
    deck.poll(0).unwrap();
    (deck, recorder)
}

#[test]
fn port_names_pick_the_mcu_pro_and_its_extender_ports() {
    let mcu = by_id(MCU_PRO_ID).unwrap();
    let xt = by_id(XT_PRO_ID).unwrap();
    let found = |raw: &str| identify_midi(&normalise(raw), true).map(|model| model.id);
    assert_eq!(found("MCU Pro USB v3.1"), Some(MCU_PRO_ID));
    assert_eq!(found("mcu pro usb v3.2"), Some(MCU_PRO_ID));
    assert_eq!(found("MIDIIN2 (MCU Pro USB v3.1)"), Some(XT_PRO_ID));
    assert_eq!(found("MIDIOUT4 (MCU Pro USB v3.1)"), Some(XT_PRO_ID));
    // Bitwig numbers the first extender port 1: still an extender.
    assert_eq!(found("MIDIIN1 (MCU Pro USB v3.1)"), Some(XT_PRO_ID));
    let alsa = |port: u16| PortName {
        base: "MCU Pro USB v3.1 MIDI 1".to_owned(),
        interface: None,
        alsa_port: Some(port),
    };
    assert!(port_matches(mcu, &alsa(0)) && !port_matches(xt, &alsa(0)));
    assert!(port_matches(xt, &alsa(2)) && !port_matches(mcu, &alsa(2)));
    assert!(!port_matches(xt, &alsa(4)) && !port_matches(mcu, &alsa(4)));
    assert_eq!(found("MCU Classic"), None);
    assert_eq!(ModelView::from(mcu).vendor_id, "0a73");
}

#[test]
fn opening_sends_only_the_device_query_until_the_deck_answers() {
    assert!(SHORT_GAP.is_zero() && SYSEX_GAP.is_zero() && QUERY_EVERY.is_zero());
    let (mut deck, recorder) = open(MCU_PRO_ID);
    deck.set_face(0x5e, Some(&face([255, 0, 0]))).unwrap();
    deck.flush().unwrap();
    assert_eq!(
        recorder.writes(),
        vec![
            vec![0xf0, 0x00, 0x00, 0x66, 0x14, 0x00, 0xf7],
            vec![0xf0, 0x00, 0x00, 0x66, 0x10, 0x00, 0xf7],
        ]
    );
    // Asked again on the next poll (every two seconds outside tests).
    deck.poll(0).unwrap();
    assert_eq!(recorder.writes().len(), 4);
}

#[test]
fn a_ready_answer_darkens_the_mcu_pro_and_greets_on_its_lcd() {
    let (mut deck, recorder) = live(MCU_PRO_ID, 0x14);
    let writes = recorder.writes();
    let after: Vec<Vec<u8>> = writes[2..].to_vec();
    let lit: Vec<u8> = (0x00..=0x1f).chain(0x28..=0x65).collect();
    assert_eq!(after.len(), lit.len() + 3 + 8 + 12 + 1);
    for (write, note) in after.iter().zip(&lit) {
        assert_eq!(write, &vec![0x90, *note, 0x00]);
    }
    let rest = &after[lit.len()..];
    assert_eq!(
        rest[..3],
        [
            vec![0x90, 0x71, 0],
            vec![0x90, 0x72, 0],
            vec![0x90, 0x73, 0]
        ]
    );
    for strip in 0..8u8 {
        assert_eq!(rest[3 + usize::from(strip)], vec![0xb0, 0x30 + strip, 0]);
    }
    for (index, cc) in (0x40..=0x4bu8).enumerate() {
        assert_eq!(rest[11 + index], vec![0xb0, cc, 0x20]);
    }
    let mut lcd = vec![0xf0, 0x00, 0x00, 0x66, 0x14, 0x12, 0x00, b'R', b'y', b'u'];
    lcd.extend([0x20; 109]);
    lcd.push(0xf7);
    assert_eq!(rest[23], lcd);
    no_pitch_bend(&writes);
    // No more queries once live.
    deck.poll(0).unwrap();
    assert_eq!(recorder.writes().len(), writes.len());
}

#[test]
fn buttons_touches_vpots_and_jog_turn_into_keys_and_dials() {
    let (mut deck, recorder) = live(MCU_PRO_ID, 0x14);
    for message in [
        [0x90, 0x5e, 0x7f],
        [0x90, 0x5e, 0x00],
        [0x90, 0x00, 0x7f],
        [0x80, 0x00, 0x40],
        [0x90, 0x68, 0x7f],
        [0x90, 0x70, 0x7f],
        [0x90, 0x71, 0x7f],
        [0xb0, 0x10, 0x01],
        [0xb0, 0x17, 0x41],
        [0xb0, 0x10, 0x00],
        [0xb0, 0x3c, 0x45],
        [0xb0, 0x2e, 0x40],
    ] {
        recorder.queue_read(message.to_vec());
    }
    assert_eq!(
        deck.poll(0).unwrap(),
        vec![
            key(0x5e, true),
            key(0x5e, false),
            key(0, true),
            key(0, false),
            key(0x68, true),
            key(112, true),
            turn(0, 1),
            turn(7, -1),
            turn(0, 1),
            turn(8, -5),
        ]
    );
}

#[test]
fn faders_seed_then_step_and_are_never_written() {
    let (mut deck, recorder) = live(MCU_PRO_ID, 0x14);
    for message in [
        [0xe0, 0x00, 0x00],
        [0xe0, 0x7f, 0x00],
        [0xe0, 0x00, 0x01],
        [0xe0, 0x00, 0x03],
        [0xe8, 0x00, 0x40],
        [0xe8, 0x00, 0x3f],
        [0xe9, 0x00, 0x00],
    ] {
        recorder.queue_read(message.to_vec());
    }
    assert_eq!(
        deck.poll(0).unwrap(),
        vec![turn(9, 1), turn(9, 2), turn(17, -1)]
    );
    no_pitch_bend(&recorder.writes());
}

#[test]
fn lights_go_only_to_lit_keys_and_rings() {
    let (mut deck, recorder) = live(MCU_PRO_ID, 0x14);
    let before = recorder.writes().len();
    for key in [0x00, 0x20, 0x5e, 0x66, 0x68] {
        deck.set_face(key, Some(&face([0, 200, 0]))).unwrap();
    }
    deck.set_dial_light(2, Some([255, 255, 255])).unwrap();
    deck.set_dial_light(9, Some([255, 255, 255])).unwrap();
    deck.flush().unwrap();
    assert_eq!(
        recorder.writes()[before..],
        [
            vec![0x90, 0x00, 0x7f],
            vec![0x90, 0x5e, 0x7f],
            vec![0xb0, 0x32, 0x2b],
        ]
    );
    deck.set_dial_light(2, None).unwrap();
    deck.set_face(0, None).unwrap();
    deck.flush().unwrap();
    assert_eq!(
        recorder.writes()[before + 3..],
        [vec![0x90, 0x00, 0x00], vec![0xb0, 0x32, 0x00]]
    );
}

#[test]
fn the_logic_control_challenge_is_answered_then_confirmed() {
    assert_eq!(challenge_response([0, 0, 0, 0]), [0x0a, 0x00, 0x00, 0x70]);
    // l3 = 127 shifts past the width; nothing panics and it reads zero.
    assert_eq!(challenge_response([1, 2, 3, 127]), [0x0a, 0x00, 0x70, 0x7f]);
    let (mut deck, recorder) = open(MCU_PRO_ID);
    // The confirmation is not overdue during this test.
    deck.query_every = Duration::from_secs(60);
    let mut challenge = vec![0xf0, 0x00, 0x00, 0x66, 0x10, 0x01];
    challenge.extend([1, 2, 3, 4, 5, 6, 7]);
    challenge.extend([0x10, 0x20, 0x30, 0x02]);
    challenge.push(0xf7);
    recorder.queue_read(challenge);
    deck.poll(0).unwrap();
    let writes = recorder.writes();
    let mut reply = vec![0xf0, 0x00, 0x00, 0x66, 0x10, 0x02, 1, 2, 3, 4, 5, 6, 7];
    reply.extend(challenge_response([0x10, 0x20, 0x30, 0x02]));
    reply.push(0xf7);
    assert_eq!(writes.last(), Some(&reply));
    let count = writes.len();
    // Input before the confirmation neither presses nor lights.
    recorder.queue_read(vec![0x90, 0x00, 0x7f]);
    assert!(deck.poll(0).unwrap().is_empty());
    assert_eq!(recorder.writes().len(), count);
    let mut confirm = vec![0xf0, 0x00, 0x00, 0x66, 0x10, 0x03];
    confirm.extend([1, 2, 3, 4, 5, 6, 7]);
    confirm.push(0xf7);
    recorder.queue_read(confirm);
    deck.poll(0).unwrap();
    let writes = recorder.writes();
    assert_eq!(
        writes[count],
        vec![0xf0, 0x00, 0x00, 0x66, 0x10, 0x13, 0x00, 0xf7]
    );
    // The LCD goes out under the Logic Control id.
    assert_eq!(
        writes.last().unwrap()[..7],
        [0xf0, 0x00, 0x00, 0x66, 0x10, 0x12, 0x00]
    );
    // It wrote 0x7f to the first LED only after the confirmation, never
    // from the early press.
    assert!(writes.iter().all(|write| write != &vec![0x90, 0x00, 0x7f]));
}

#[test]
fn an_unconfirmed_challenge_is_queried_again() {
    let (mut deck, recorder) = open(XT_PRO_ID);
    let mut challenge = vec![0xf0, 0x00, 0x00, 0x66, 0x11, 0x01];
    challenge.extend([0; 11]);
    challenge.push(0xf7);
    recorder.queue_read(challenge.clone());
    deck.poll(0).unwrap();
    assert_eq!(deck.phase, Phase::Challenged);
    let count = recorder.writes().len();
    // No confirmation: the query goes out again (two seconds later
    // outside tests) and a fresh challenge is answered.
    deck.poll(0).unwrap();
    assert_eq!(deck.phase, Phase::Waiting);
    assert!(deck.unconfirmed_warned);
    assert_eq!(
        recorder.writes()[count..],
        [
            vec![0xf0, 0x00, 0x00, 0x66, 0x15, 0x00, 0xf7],
            vec![0xf0, 0x00, 0x00, 0x66, 0x11, 0x00, 0xf7],
        ]
    );
    recorder.queue_read(challenge);
    deck.poll(0).unwrap();
    assert_eq!(deck.phase, Phase::Challenged);
    assert_eq!(recorder.writes().last().unwrap()[4..6], [0x11, 0x02]);
}

#[test]
fn a_rebooted_deck_that_announces_itself_again_is_relit() {
    let (mut deck, recorder) = live(XT_PRO_ID, 0x15);
    deck.set_face(3, Some(&face([255, 255, 255]))).unwrap();
    deck.set_dial_light(1, Some([255, 255, 255])).unwrap();
    deck.flush().unwrap();
    let before = recorder.writes().len();
    recorder.queue_read(ready(0x15));
    deck.poll(0).unwrap();
    let again = recorder.writes()[before..].to_vec();
    // Darkened and greeted again, its light and ring resent in place of
    // their off messages.
    assert_eq!(again.len(), 32 + 8 + 1);
    assert!(again.contains(&vec![0x90, 0x03, 0x7f]) && again.contains(&vec![0xb0, 0x31, 0x2b]));
    assert!(
        !again.contains(&vec![0x90, 0x03, 0x00]) && !again.contains(&vec![0xb0, 0x31, 0x00])
    );
    assert_eq!(again[0], vec![0x90, 0x00, 0x00]);
    assert!(again
        .iter()
        .any(|write| write.get(4..6) == Some(&[0x15, 0x12][..])));
    // A fresh Logic Control challenge from a live deck is answered.
    let mut challenge = vec![0xf0, 0x00, 0x00, 0x66, 0x11, 0x01];
    challenge.extend([0; 11]);
    challenge.push(0xf7);
    recorder.queue_read(challenge);
    deck.poll(0).unwrap();
    assert_eq!(recorder.writes().last().unwrap()[4..6], [0x11, 0x02]);
}

#[test]
fn an_unnamed_mcu_port_takes_an_xt_pro_that_answers() {
    let (mut deck, recorder) = open(MCU_PRO_ID);
    deck.named_ports = false;
    deck.poll(0).unwrap();
    let queried: Vec<u8> = recorder.writes().iter().map(|write| write[4]).collect();
    assert_eq!(queried, vec![0x14, 0x10, 0x15, 0x11]);
    recorder.queue_read(ready(0x15));
    deck.poll(0).unwrap();
    assert_eq!(deck.phase, Phase::Live);
    // Past the second round of queries sent before the answer was read.
    let writes = recorder.writes()[8..].to_vec();
    // The XT Pro's strip lights and rings, its LCD; no globals, lamps or
    // digits.
    assert_eq!(writes.len(), 32 + 8 + 1);
    assert_eq!(writes.last().unwrap()[4..6], [0x15, 0x12]);
    let before = recorder.writes().len();
    deck.set_face(0x5e, Some(&face([255, 255, 255]))).unwrap();
    deck.set_face(0x02, Some(&face([255, 255, 255]))).unwrap();
    deck.flush().unwrap();
    assert_eq!(recorder.writes()[before..], [vec![0x90, 0x02, 0x7f]]);
    // Its fader touch and first fader read with the MCU Pro's layout.
    for message in [[0x90, 0x68, 0x7f], [0xe0, 0x00, 0x00], [0xe0, 0x00, 0x01]] {
        recorder.queue_read(message.to_vec());
    }
    assert_eq!(deck.poll(0).unwrap(), vec![key(0x68, true), turn(9, 1)]);
    // A named MCU Pro port ignores an XT Pro's answer.
    let (mut deck, recorder) = open(MCU_PRO_ID);
    deck.named_ports = true;
    recorder.queue_read(ready(0x15));
    deck.poll(0).unwrap();
    assert_eq!(deck.phase, Phase::Waiting);
}

#[test]
fn a_denied_handshake_stays_silent() {
    let (mut deck, recorder) = open(MCU_PRO_ID);
    let mut challenge = vec![0xf0, 0x00, 0x00, 0x66, 0x10, 0x01];
    challenge.extend([0; 11]);
    challenge.push(0xf7);
    recorder.queue_read(challenge);
    recorder.queue_read(vec![0xf0, 0x00, 0x00, 0x66, 0x10, 0x04, 0xf7]);
    deck.poll(0).unwrap();
    let count = recorder.writes().len();
    deck.set_face(0, Some(&face([255, 255, 255]))).unwrap();
    deck.flush().unwrap();
    deck.poll(0).unwrap();
    drop(deck);
    assert_eq!(recorder.writes().len(), count);
}

#[test]
fn the_xt_pro_stays_silent_on_pitch_bend_and_notes_until_it_answers() {
    let (mut deck, recorder) = open(XT_PRO_ID);
    deck.set_face(0, Some(&face([255, 255, 255]))).unwrap();
    for message in [[0xe0, 0x00, 0x40], [0xe0, 0x00, 0x50], [0x90, 0x68, 0x7f]] {
        recorder.queue_read(message.to_vec());
    }
    assert!(deck.poll(0).unwrap().is_empty());
    deck.flush().unwrap();
    let writes = recorder.writes();
    assert!(writes
        .iter()
        .all(|write| write[..5] == [0xf0, 0x00, 0x00, 0x66, write[4]]
            && write[5] == 0x00
            && matches!(write[4], 0x15 | 0x11)));
    drop(deck);
    assert_eq!(recorder.writes(), writes);

    let (mut deck, recorder) = live(XT_PRO_ID, 0x15);
    let writes = recorder.writes();
    // Rec to Select off, eight rings off, the LCD; no lamps or digits.
    assert_eq!(writes.len(), 2 + 32 + 8 + 1);
    assert!(writes
        .iter()
        .all(|write| !(write[0] == 0x90 && write[1] > 0x1f)
            && !(write[0] == 0xb0 && write[1] >= 0x40)));
    assert_eq!(writes.last().unwrap()[4..6], [0x15, 0x12]);
    deck.set_face(0, Some(&face([255, 255, 255]))).unwrap();
    deck.flush().unwrap();
    assert_eq!(recorder.writes().last(), Some(&vec![0x90, 0x00, 0x7f]));
    for message in [
        [0x90, 0x6f, 0x7f],
        [0xe7, 0x00, 0x00],
        [0xe7, 0x00, 0x02],
        [0xe8, 0, 0],
    ] {
        recorder.queue_read(message.to_vec());
    }
    assert_eq!(deck.poll(0).unwrap(), vec![key(47, true), turn(15, 2)]);
}

#[test]
fn the_mcu_pro_port_goes_live_on_input_where_ports_are_named() {
    for named in [true, false] {
        let (mut deck, recorder) = open(MCU_PRO_ID);
        deck.named_ports = named;
        recorder.queue_read(vec![0x90, 0x5e, 0x7f]);
        let events = deck.poll(0).unwrap();
        if named {
            assert_eq!(events, vec![key(0x5e, true)]);
            assert!(recorder.writes().len() > 2);
        } else {
            assert!(events.is_empty());
        }
    }
}

#[test]
fn letting_go_darkens_and_blanks_at_once() {
    let (deck, recorder) = live(MCU_PRO_ID, 0x14);
    let before = recorder.writes().len();
    drop(deck);
    let writes = recorder.writes();
    let closing = &writes[before..];
    assert_eq!(closing.len(), 94 + 3 + 8 + 12 + 1);
    let mut blank = vec![0xf0, 0x00, 0x00, 0x66, 0x14, 0x12, 0x00];
    blank.extend([0x20; 112]);
    blank.push(0xf7);
    assert_eq!(closing.last(), Some(&blank));
    no_pitch_bend(&writes);
}

#[test]
fn short_and_foreign_sysex_is_ignored() {
    let (mut deck, recorder) = open(XT_PRO_ID);
    for message in [
        vec![0xf0, 0xf7],
        vec![0xf0, 0x00, 0x00, 0x66, 0xf7],
        vec![0xf0, 0x00, 0x00, 0x66, 0x14, 0x01, 0xf7],
        vec![0xf0, 0x00, 0x20, 0x29, 0x15, 0x01, 0xf7],
        vec![0xf0, 0x00, 0x00, 0x66, 0x11, 0x01, 0xf7],
        vec![0xf0, 0x00, 0x00, 0x66, 0x15, 0x03, 0xf7],
    ] {
        recorder.queue_read(message);
    }
    deck.poll(0).unwrap();
    assert_eq!(deck.phase, Phase::Waiting);
    assert!(recorder
        .writes()
        .iter()
        .all(|write| write.get(5) == Some(&0x00)));
}

#[test]
fn key_names_and_lights_follow_the_note_map() {
    let mcu = by_id(MCU_PRO_ID).unwrap();
    let xt = by_id(XT_PRO_ID).unwrap();
    let names = key_names(mcu).unwrap();
    assert_eq!(
        (names.len(), names[0x5e], names[112]),
        (113, "Play", "Master Touch")
    );
    let names = key_names(xt).unwrap();
    assert_eq!(
        (names.len(), names[39], names[40]),
        (48, "V-Pot 8", "Fader Touch 1")
    );
    assert_eq!(lit_keys(mcu).unwrap().len(), 94);
    assert_eq!(lit_keys(xt).unwrap(), (0..32).collect::<Vec<u8>>());
    assert_eq!((mcu.dial_light_colors(), xt.dial_light_colors()), (8, 8));
}
