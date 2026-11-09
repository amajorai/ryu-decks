//! The MCU Pro / XT Pro deck: its lifecycle, handshake and output queue.

use super::*;

impl MackieMcu {
    #[must_use]
    pub fn new(model: &DeviceModel, transport: Box<dyn Transport>) -> Self {
        let kind = Kind::of(model).unwrap_or(Kind::Xt);
        let keys = usize::from(kind.keys());
        Self {
            kind,
            transport,
            phase: Phase::Waiting,
            device: kind.devices()[0],
            query_at: None,
            query_every: QUERY_EVERY,
            outbox: VecDeque::new(),
            ready_at: Instant::now(),
            short_gap: SHORT_GAP,
            sysex_gap: SYSEX_GAP,
            leds: vec![false; keys],
            queued: vec![false; keys],
            held: vec![false; keys],
            rings: [false; RINGS as usize],
            queued_rings: [false; RINGS as usize],
            faders: [Fader::default(); MAX_FADERS],
            warned: false,
            unconfirmed_warned: false,
            named_ports: NAMED_PORTS,
            unit: kind,
            parser: Parser::default(),
        }
    }

    pub(super) fn push(&mut self, slot: Slot, bytes: Vec<u8>) {
        if slot != Slot::Other {
            self.outbox.retain(|out| out.slot != slot);
        }
        self.outbox.push_back(Out { slot, bytes });
    }

    /// The SysEx ids this deck queries and accepts answers from: on an
    /// unnamed (macOS) port an MCU Pro deck takes an XT Pro's too.
    pub(super) fn devices(&self) -> Vec<u8> {
        let mut devices = self.kind.devices().to_vec();
        if self.kind == Kind::Mcu && !self.named_ports {
            devices.extend(Kind::Xt.devices());
        }
        devices
    }

    /// Whether a key has a light on both the model and the unit behind it.
    pub(super) fn lit(&self, key: u8) -> bool {
        self.kind.has_light(key) && self.unit.has_light(key)
    }

    /// Queues the device query when it is due, while the deck has not
    /// answered or has not confirmed a Logic Control challenge.
    pub(super) fn query(&mut self) {
        if !matches!(self.phase, Phase::Waiting | Phase::Challenged) {
            return;
        }
        let now = Instant::now();
        if self.query_at.is_some_and(|at| at > now) {
            return;
        }
        if self.phase == Phase::Challenged {
            self.phase = Phase::Waiting;
            if !self.unconfirmed_warned {
                self.unconfirmed_warned = true;
                tracing::warn!(
                    "the Mackie MCU Pro / XT Pro never confirmed Logic Control's handshake; \
                     asking it again"
                );
            }
        }
        self.query_at = Some(now + self.query_every);
        for device in self.devices() {
            self.push(Slot::Other, sysex(device, &[QUERY]));
        }
    }

    pub(super) fn led_message(&self, key: u8, on: bool) -> Option<Vec<u8>> {
        let note = self.kind.note(key)?;
        Some(vec![NOTE_ON, note, if on { LED_ON } else { LED_OFF }])
    }

    pub(super) fn lcd_message(&self, text: &[u8]) -> Vec<u8> {
        let mut body = vec![LCD, 0];
        let mut chars = [BLANK; LCD_CHARS];
        for (slot, byte) in chars.iter_mut().zip(text) {
            *slot = *byte;
        }
        body.extend_from_slice(&chars);
        sysex(self.device, &body)
    }

    /// Every light off and the 7-segment digits blank.
    pub(super) fn dark_messages(&self) -> Vec<(Slot, Vec<u8>)> {
        let mut messages = Vec::new();
        for key in self.kind.lit_keys() {
            if !self.unit.has_light(key) {
                continue;
            }
            if let Some(bytes) = self.led_message(key, false) {
                messages.push((Slot::Led(key), bytes));
            }
        }
        if self.unit == Kind::Mcu {
            for lamp in LAMPS {
                messages.push((Slot::Other, vec![NOTE_ON, lamp, LED_OFF]));
            }
        }
        for strip in 0..RINGS {
            messages.push((
                Slot::Ring(strip),
                vec![CONTROL_CHANGE, RING_CC + strip, RING_OFF],
            ));
        }
        if self.unit == Kind::Mcu {
            for digit in SEGMENT_CCS {
                messages.push((Slot::Other, vec![CONTROL_CHANGE, digit, BLANK]));
            }
        }
        messages
    }

    /// The deck answered: darken it, greet on the LCD, then whatever is
    /// staged.
    pub(super) fn go_live(&mut self) {
        if self.phase == Phase::Live {
            return;
        }
        self.phase = Phase::Live;
        self.outbox.clear();
        for (slot, bytes) in self.dark_messages() {
            self.push(slot, bytes);
        }
        let greeting = self.lcd_message(GREETING);
        self.push(Slot::Other, greeting);
        self.queued.fill(false);
        self.queued_rings = [false; RINGS as usize];
        self.queue_changes();
    }

    pub(super) fn queue_changes(&mut self) {
        if self.phase != Phase::Live {
            return;
        }
        for key in 0..self.kind.keys() {
            let index = usize::from(key);
            let on = self.leds.get(index).copied().unwrap_or(false);
            if !self.lit(key) || self.queued.get(index) == Some(&on) {
                continue;
            }
            if let Some(queued) = self.queued.get_mut(index) {
                *queued = on;
            }
            if let Some(bytes) = self.led_message(key, on) {
                self.push(Slot::Led(key), bytes);
            }
        }
        for strip in 0..RINGS {
            let index = usize::from(strip);
            let on = self.rings[index];
            if self.queued_rings[index] == on {
                continue;
            }
            self.queued_rings[index] = on;
            let value = if on { RING_ALL } else { RING_OFF };
            self.push(
                Slot::Ring(strip),
                vec![CONTROL_CHANGE, RING_CC + strip, value],
            );
        }
    }

    pub(super) fn pump(&mut self) -> DriverResult<()> {
        let now = Instant::now();
        if let Some(floor) = now.checked_sub(MAX_CREDIT) {
            self.ready_at = self.ready_at.max(floor);
        }
        while let Some(out) = self.outbox.front() {
            if self.ready_at > now {
                break;
            }
            self.transport.write(&out.bytes)?;
            let gap = if out.bytes.first() == Some(&SYSEX_START) {
                self.sysex_gap
            } else {
                self.short_gap
            };
            self.outbox.pop_front();
            self.ready_at += gap;
        }
        Ok(())
    }

    pub(super) fn hint(&mut self) {
        if !self.warned {
            self.warned = true;
            tracing::warn!(
                "the Mackie MCU Pro / XT Pro sent a message Ryu does not expect; boot it in its \
                 Mackie Control (or Logic Control) mode for Ryu to read it"
            );
        }
    }

    pub(super) fn sysex(&mut self, bytes: &[u8]) {
        if bytes.get(1..4) != Some(&MACKIE[..]) {
            return;
        }
        let Some(&device) = bytes.get(4) else {
            return;
        };
        if !self.devices().contains(&device) {
            return;
        }
        match bytes.get(5).copied() {
            Some(READY) => self.ready(device, bytes),
            Some(CONFIRMED) if self.phase == Phase::Challenged && device == self.device => {
                self.go_live();
                self.outbox.push_front(Out {
                    slot: Slot::Other,
                    bytes: sysex(device, &[VERSION_REQUEST, 0]),
                });
            }
            Some(DENIED) if self.phase != Phase::Live => {
                self.phase = Phase::Denied;
                self.outbox.clear();
                tracing::warn!(
                    "the Mackie MCU Pro / XT Pro refused Logic Control's handshake; boot it in \
                     its Mackie Control mode for Ryu to drive it"
                );
            }
            _ => {}
        }
    }

    /// A device-ready answer: live at once in Mackie mode, a challenge to
    /// answer in Logic Control mode. A live deck that announces itself again
    /// was rebooted: it is darkened, greeted and relit (or challenged) anew.
    pub(super) fn ready(&mut self, device: u8, bytes: &[u8]) {
        if self.phase == Phase::Denied {
            return;
        }
        let unit = if Kind::Xt.devices().contains(&device) {
            Kind::Xt
        } else {
            Kind::Mcu
        };
        if device == unit.devices()[0] {
            self.device = device;
            self.unit = unit;
            if bytes.len() == CHALLENGE_LEN {
                tracing::debug!(serial = ?bytes.get(SERIAL), "Mackie surface answered");
            }
            self.phase = Phase::Waiting;
            self.go_live();
            return;
        }
        if bytes.len() != CHALLENGE_LEN {
            return;
        }
        let (Some(serial), Some(&[l0, l1, l2, l3])) = (bytes.get(SERIAL), bytes.get(CHALLENGE))
        else {
            return;
        };
        let mut body = vec![CHALLENGE_REPLY];
        body.extend_from_slice(serial);
        body.extend_from_slice(&challenge_response([l0, l1, l2, l3]));
        self.device = device;
        self.unit = unit;
        self.outbox.clear();
        self.push(Slot::Other, sysex(device, &body));
        self.phase = Phase::Challenged;
        self.query_at = Some(Instant::now() + self.query_every);
    }

    pub(super) fn note(&mut self, note: u8, pressed: bool, events: &mut Vec<Input>) {
        let Some(key) = self.kind.key(note) else {
            self.hint();
            return;
        };
        let Some(held) = self.held.get_mut(usize::from(key)) else {
            return;
        };
        if *held != pressed {
            *held = pressed;
            events.push(Input::Key {
                index: key,
                pressed,
            });
        }
    }

    pub(super) fn control(&mut self, number: u8, value: u8, events: &mut Vec<Input>) {
        let index = match number {
            JOG_CC if self.kind == Kind::Mcu => STRIPS,
            _ if (VPOT_CC..VPOT_CC + STRIPS).contains(&number) => number - VPOT_CC,
            EXTERNAL_CC => return,
            _ => {
                self.hint();
                return;
            }
        };
        events.push(Input::DialTurn {
            index,
            delta: relative(value),
        });
    }

    pub(super) fn fader(&mut self, channel: u8, lsb: u8, msb: u8, events: &mut Vec<Input>) {
        if channel >= self.kind.faders() {
            return;
        }
        let Some(fader) = self.faders.get_mut(usize::from(channel)) else {
            return;
        };
        let value = u16::from(lsb & 0x7f) | u16::from(msb & 0x7f) << 7;
        let delta = fader.moved(value, FADER_STEP);
        if delta != 0 {
            events.push(Input::DialTurn {
                index: self.kind.first_fader_dial() + channel,
                delta,
            });
        }
    }

    pub(super) fn receive(&mut self, message: &Message, events: &mut Vec<Input>) {
        let [status, data1, data2] = match message {
            Message::SysEx(bytes) => return self.sysex(bytes),
            Message::Channel(bytes) => *bytes,
        };
        if status == CONTROL_CHANGE && matches!(data1, HUI_ZONE_CC | HUI_PORT_CC) {
            self.hint();
            return;
        }
        if self.phase != Phase::Live {
            // Only the MCU Pro's own, plainly named port may go live on
            // input; an extender port may carry any MIDI gear.
            let shaped = matches!(status, NOTE_ON | NOTE_OFF | CONTROL_CHANGE)
                || status & 0xf0 == PITCH_BEND;
            if self.kind == Kind::Mcu && self.named_ports && self.phase == Phase::Waiting && shaped
            {
                self.go_live();
            } else {
                return;
            }
        }
        match status {
            NOTE_ON => self.note(data1, data2 > PRESS_OVER, events),
            NOTE_OFF => self.note(data1, false, events),
            CONTROL_CHANGE => self.control(data1, data2, events),
            _ if status & 0xf0 == PITCH_BEND => self.fader(status & 0x0f, data1, data2, events),
            _ if status & 0xf0 == CHANNEL_PRESSURE => {}
            _ => self.hint(),
        }
    }
}

impl Driver for MackieMcu {
    fn poll(&mut self, timeout_ms: i32) -> DriverResult<Vec<Input>> {
        self.query();
        self.pump()?;
        let mut events = Vec::new();
        let mut messages = Vec::new();
        let mut buf = [0u8; READ_LEN];
        for attempt in 0..MAX_MESSAGES_PER_POLL {
            let wait = if attempt == 0 { timeout_ms } else { 0 };
            let read = self.transport.read_timeout(&mut buf, wait)?;
            if read == 0 {
                break;
            }
            self.parser
                .feed(buf.get(..read).unwrap_or_default(), &mut messages);
        }
        for message in &messages {
            self.receive(message, &mut events);
        }
        self.pump()?;
        Ok(events)
    }

    fn set_face(&mut self, key: u8, face: Option<&Face>) -> DriverResult<()> {
        if self.lit(key) {
            if let Some(led) = self.leds.get_mut(usize::from(key)) {
                *led = is_lit(face.map(|face| face.color));
            }
        }
        Ok(())
    }

    fn flush(&mut self) -> DriverResult<()> {
        self.query();
        self.queue_changes();
        self.pump()
    }

    /// A V-Pot's LED ring, all on or off; the jog wheel and faders have no
    /// light.
    fn set_dial_light(&mut self, dial: u8, color: Option<[u8; 3]>) -> DriverResult<()> {
        if let Some(ring) = self.rings.get_mut(usize::from(dial)) {
            *ring = is_lit(color);
        }
        Ok(())
    }

    /// The MCU Pro has no brightness control.
    fn set_brightness(&mut self, _percent: u8) -> DriverResult<()> {
        Ok(())
    }
}

impl Drop for MackieMcu {
    /// Turns the lights off and blanks the displays at once, without
    /// pacing; anything still queued is dropped. Nothing is sent to a deck
    /// that never answered.
    fn drop(&mut self) {
        if self.phase != Phase::Live {
            return;
        }
        self.outbox.clear();
        for (_, bytes) in self.dark_messages() {
            let _ = self.transport.write(&bytes);
        }
        let blank = self.lcd_message(&[]);
        let _ = self.transport.write(&blank);
    }
}
