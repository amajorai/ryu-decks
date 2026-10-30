//! Device drivers. Each family speaks its own dialect behind one [`Driver`]
//! interface, over a [`Transport`] (a HID device, or a serial port for
//! Loupedeck, TourBox, the Yamaha CC121MK2, DJI remotes and monome grids and arcs,
//! a Linux input event node for the Griffin PowerMate, or a pair of USB MIDI
//! ports for the Novation Launchpads, Launch Controls (MK1/MK2 and 3), Launchkeys (MK3 and MK4), first Launchkey Mini and SL MkIII and the Akai APC minis, APC40 MKII, APC Key 25s, MIDImix, LPD8, Fire, MPK mini IV, MPK mini Plus II and first MPK mini Plus, the Ableton Push 2 and the Behringer X-Touch Mini, X-Touch One, BCR2000 and BCF2000 and the KORG nanoKONTROL2, nanoKONTROL Studio and padKONTROL and the Arturia BeatStep, MiniLab 3 and KeyLab Essential mk3, the DJ TechTools Midi Fighter Twister, Midi Fighter Spectra and Midi Fighter 64, Intech Studio's Grid modules and the PreSonus ATOM, ATOM SQ and FaderPorts, the Faderfox EC4, the Livid Instruments Base and Base II, the Allen & Heath Xone:K2, the Intuitive Instruments Exquis, the M-Audio Oxygen Pro 25, 49, 61 and Mini and Hammer 88 Pro and the iCON Platform M+ and M and QCon Pro X, XS1, Pro G2 and EX G2 and the Zoom R16 and R24 recorders, [`midi::MidiLink`]) so the framing can be tested without hardware. The Touch Bar is the
//! exception: AppKit drives it from the main thread over a [`touchbar::Link`].

pub mod akai_apc;
pub mod akai_apc40;
pub mod akai_apc40_mk1;
pub mod akai_apc_key25;
pub mod akai_apc_mini;
pub mod akai_fire;
pub mod akai_lpd8;
pub mod akai_midimix;
pub mod akai_mpk_mini_plus;
pub mod akai_mpk_mk4;
pub mod arturia_beatstep;
pub mod arturia_keylab_essential3;
pub mod arturia_minilab3;
pub mod behringer_bcontrol;
pub mod blackmagic;
pub mod buzz;
pub mod contour;
pub mod dji;
pub mod elgato;
pub mod evdev;
pub mod exquis;
pub mod face;
pub mod faderfox_ec4;
pub mod faderport;
pub mod faderport8;
pub mod framework;
pub mod g13;
pub mod g15;
pub mod gamesir;
pub mod griffin;
pub mod icon_p1;
pub mod icon_platform;
pub mod icon_platform_x;
pub mod icon_qcon;
pub mod infinitton;
pub mod intech_grid;
pub mod komplete;
pub mod korg;
pub mod korg_padkontrol;
pub mod lamparray;
pub mod launch_control;
pub mod launch_control3;
pub mod launchkey;
pub mod launchkey_mini;
pub mod launchkey_mk2;
pub mod launchkey_mk4;
pub mod launchpad;
pub mod launchpad_legacy;
pub mod launchpad_mk2;
pub mod launchpad_pro;
pub mod livid_base;
pub mod logitech;
pub mod logitech_rgb;
pub mod loupedeck;
pub mod mackie_mcu;
pub mod macropad;
pub mod maschine;
pub mod masterhub;
pub mod maudio_oxygen_pro;
pub mod midi;
pub mod midi_fighter_64;
pub mod midi_fighter_spectra;
pub mod midi_fighter_twister;
pub mod mirabox;
pub mod monome;
pub mod mountain;
pub mod mystrix;
pub mod novation_dicer;
pub mod playstation;
pub mod prehkeytec;
pub mod presonus_atom;
pub mod presonus_atom_sq;
pub mod push1;
pub mod push2;
pub mod sl_mkiii;
pub mod spacemouse;
pub mod steelseries;
pub mod surfacecore;
pub mod touchbar;
pub mod tourbox;
pub mod traktor;
pub mod ulanzi;
pub mod vec;
pub mod vial;
pub mod wooting;
pub mod worklouder;
pub mod xbox;
pub mod xencelabs;
pub mod xkeys;
pub mod xone_k2;
pub mod xtouch_mini;
pub mod xtouch_one;
pub mod yamaha;
pub mod zoom;

use image::RgbaImage;

use crate::catalog::{DeviceModel, Protocol};

/// One thing that happened on the device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Input {
    Key {
        index: u8,
        pressed: bool,
    },
    DialPress {
        index: u8,
        pressed: bool,
    },
    DialTurn {
        index: u8,
        delta: i8,
    },
    /// A swipe along a touch strip, from the strip key under the finger at the
    /// start to the one under it at the end (equal for a short swipe).
    StripSwipe {
        from: u8,
        to: u8,
        rightward: bool,
    },
}

/// What one key should show. `None` in [`Driver::set_face`] means the key is
/// unbound and goes dark.
#[derive(Debug, Clone)]
pub struct Face {
    /// The key's label: drawn into `image` by the Companion, and read aloud on
    /// decks whose keys are native buttons (the Touch Bar).
    pub label: String,
    /// Background, and the light colour on decks that have lights instead of screens.
    pub color: [u8; 3],
    /// A rendered key face (already composed by the Companion); drawn over `color`.
    pub image: Option<RgbaImage>,
}

/// A deck whose keys must be unlocked on the device before the host may read
/// them (a Vial keyboard's matrix).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct LockState {
    pub unlocked: bool,
    /// The user is holding the unlock keys; it cannot be cancelled.
    pub unlocking: bool,
    /// How far through the hold, 0 to 100.
    pub progress: u8,
    /// The keys to hold down together.
    pub keys: Vec<u8>,
    /// Some unlock keys are in the switch matrix but not on the drawn layout,
    /// so Ryu cannot show them: such a board is unlocked in the Vial app.
    pub keys_off_layout: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum DriverError {
    #[error("device I/O failed: {0}")]
    Io(String),
    #[error("could not encode the key image: {0}")]
    Image(String),
    /// The device opened but will not work with Ryu (for example it refused
    /// a handshake). The manager treats it like a deck that would not open and
    /// waits before trying again.
    #[error("{0}")]
    Refused(String),
}

impl From<hidapi::HidError> for DriverError {
    fn from(error: hidapi::HidError) -> Self {
        Self::Io(error.to_string())
    }
}

impl From<std::io::Error> for DriverError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error.to_string())
    }
}

pub type DriverResult<T> = Result<T, DriverError>;

/// The HID operations a driver needs. Implemented for a real `HidDevice` and,
/// in tests, for a recorder.
pub trait Transport: Send {
    fn write(&mut self, data: &[u8]) -> DriverResult<()>;
    /// Reads one input report. `Ok(0)` means nothing arrived before the timeout.
    fn read_timeout(&mut self, buf: &mut [u8], timeout_ms: i32) -> DriverResult<usize>;
    fn send_feature_report(&mut self, data: &[u8]) -> DriverResult<()>;
    /// Reads one feature report. `buf[0]` names the report on the way in and
    /// carries its id on the way out; returns how many bytes arrived.
    fn get_feature_report(&mut self, buf: &mut [u8]) -> DriverResult<usize>;
    /// Reads the HID report descriptor of the opened collection's interface;
    /// returns how many bytes arrived. Fails where there is none to read.
    fn report_descriptor(&mut self, _buf: &mut [u8]) -> DriverResult<usize> {
        Err(DriverError::Io("no report descriptor to read".to_owned()))
    }
    /// The usage page of the HID collection this transport was opened on,
    /// where the opener recorded it ([`HidCollection`]).
    fn usage_page(&self) -> Option<u16> {
        None
    }
    /// A Linux input event node (`evdev::EvdevLink`, Linux only) rather than a HID
    /// device: reads and writes are [`evdev::Event`] records.
    fn is_evdev(&self) -> bool {
        false
    }
}

impl Transport for hidapi::HidDevice {
    fn write(&mut self, data: &[u8]) -> DriverResult<()> {
        hidapi::HidDevice::write(self, data)?;
        Ok(())
    }

    fn read_timeout(&mut self, buf: &mut [u8], timeout_ms: i32) -> DriverResult<usize> {
        Ok(hidapi::HidDevice::read_timeout(self, buf, timeout_ms)?)
    }

    fn send_feature_report(&mut self, data: &[u8]) -> DriverResult<()> {
        hidapi::HidDevice::send_feature_report(self, data)?;
        Ok(())
    }

    fn get_feature_report(&mut self, buf: &mut [u8]) -> DriverResult<usize> {
        Ok(hidapi::HidDevice::get_feature_report(self, buf)?)
    }

    fn report_descriptor(&mut self, buf: &mut [u8]) -> DriverResult<usize> {
        Ok(hidapi::HidDevice::get_report_descriptor(self, buf)?)
    }
}

/// A HID collection opened by path, with the usage page the scan found it on:
/// a deck driven through two collections told apart by page (a Wooting
/// keyboard) learns from it which dialect each one speaks.
pub struct HidCollection {
    pub device: hidapi::HidDevice,
    pub usage_page: u16,
}

impl Transport for HidCollection {
    fn write(&mut self, data: &[u8]) -> DriverResult<()> {
        Transport::write(&mut self.device, data)
    }

    fn read_timeout(&mut self, buf: &mut [u8], timeout_ms: i32) -> DriverResult<usize> {
        Transport::read_timeout(&mut self.device, buf, timeout_ms)
    }

    fn send_feature_report(&mut self, data: &[u8]) -> DriverResult<()> {
        Transport::send_feature_report(&mut self.device, data)
    }

    fn get_feature_report(&mut self, buf: &mut [u8]) -> DriverResult<usize> {
        Transport::get_feature_report(&mut self.device, buf)
    }

    fn report_descriptor(&mut self, buf: &mut [u8]) -> DriverResult<usize> {
        Transport::report_descriptor(&mut self.device, buf)
    }

    fn usage_page(&self) -> Option<u16> {
        Some(self.usage_page)
    }
}

/// A USB serial port as a [`Transport`]: writes go out whole, and a read returns
/// whatever bytes arrived within the timeout.
pub struct SerialLink(pub Box<dyn serialport::SerialPort>);

impl Transport for SerialLink {
    fn write(&mut self, data: &[u8]) -> DriverResult<()> {
        std::io::Write::write_all(&mut self.0, data)?;
        Ok(())
    }

    fn read_timeout(&mut self, buf: &mut [u8], timeout_ms: i32) -> DriverResult<usize> {
        let timeout = std::time::Duration::from_millis(u64::try_from(timeout_ms).unwrap_or(0));
        self.0
            .set_timeout(timeout)
            .map_err(|error| DriverError::Io(error.to_string()))?;
        match std::io::Read::read(&mut self.0, buf) {
            Ok(len) => Ok(len),
            Err(error) if error.kind() == std::io::ErrorKind::TimedOut => Ok(0),
            Err(error) => Err(error.into()),
        }
    }

    fn send_feature_report(&mut self, _data: &[u8]) -> DriverResult<()> {
        Err(DriverError::Io(
            "a serial deck has no feature reports".to_owned(),
        ))
    }

    fn get_feature_report(&mut self, _buf: &mut [u8]) -> DriverResult<usize> {
        Err(DriverError::Io(
            "a serial deck has no feature reports".to_owned(),
        ))
    }
}

pub trait Driver: Send {
    /// Drains whatever input arrived, waiting at most `timeout_ms` for the first report.
    fn poll(&mut self, timeout_ms: i32) -> DriverResult<Vec<Input>>;
    /// Stages one key's face. Changes reach the device on [`Driver::flush`].
    fn set_face(&mut self, key: u8, face: Option<&Face>) -> DriverResult<()>;
    /// Sends every staged face.
    fn flush(&mut self) -> DriverResult<()>;
    fn set_brightness(&mut self, percent: u8) -> DriverResult<()>;
    /// Stages one dial's light (`None` turns it off). Changes reach the device on
    /// [`Driver::flush`]. Decks without dial lights ignore it.
    fn set_dial_light(&mut self, _dial: u8, _color: Option<[u8; 3]>) -> DriverResult<()> {
        Ok(())
    }
    /// Stages the colour of the deck's own lights (`None` turns them off), sent
    /// on [`Driver::flush`]. Decks without them ignore it.
    fn set_deck_light(&mut self, _color: Option<[u8; 3]>) -> DriverResult<()> {
        Ok(())
    }
    /// Called every couple of seconds. Decks with a host watchdog use it to stay awake.
    fn heartbeat(&mut self) -> DriverResult<()> {
        Ok(())
    }
    /// Whether the deck's keys are locked, on decks that have such a lock.
    fn lock_state(&self) -> Option<LockState> {
        None
    }
    /// Starts unlocking the deck's keys: the user then holds the keys
    /// [`Driver::lock_state`] names.
    fn start_unlock(&mut self) -> DriverResult<()> {
        Err(DriverError::Refused("This deck has no lock.".to_owned()))
    }
}

/// The driver for an opened device of `model`.
pub fn for_model(model: &'static DeviceModel, transport: Box<dyn Transport>) -> Box<dyn Driver> {
    for_model_with_display(model, transport, None)
}

/// The driver for an opened device of `model`, with the second HID interface
/// that decks with [`Protocol::paired_interfaces`] draw their faces through, or
/// the lighting collection of decks with [`Protocol::paired_collections`] (and
/// of the Adafruit MacroPad, where Windows lists it apart).
pub fn for_model_with_display(
    model: &'static DeviceModel,
    transport: Box<dyn Transport>,
    display: Option<Box<dyn Transport>>,
) -> Box<dyn Driver> {
    match model.protocol {
        Protocol::ElgatoOriginal
        | Protocol::ElgatoMini
        | Protocol::ElgatoV2
        | Protocol::ElgatoPlus
        | Protocol::ElgatoGalleon => Box::new(elgato::Elgato::new(model, transport)),
        Protocol::UlanziD200 => Box::new(ulanzi::UlanziD200::new(model, transport)),
        Protocol::WorkLouderRpc => Box::new(worklouder::WorkLouder::new(model, transport)),
        Protocol::Loupedeck => Box::new(loupedeck::Loupedeck::new(model, transport)),
        Protocol::MiraboxV1 | Protocol::MiraboxV2 | Protocol::MiraboxV3 => {
            Box::new(mirabox::Mirabox::new(model, transport))
        }
        Protocol::Infinitton => Box::new(infinitton::Infinitton::new(model, transport)),
        Protocol::ContourShuttle => Box::new(contour::Contour::new(model, transport)),
        Protocol::BlackmagicPanel => Box::new(blackmagic::Blackmagic::new(model, transport)),
        Protocol::XKeysLegacy | Protocol::XKeysRgb24 | Protocol::XKeysRgbBanks => {
            Box::new(xkeys::XKeys::new(model, transport))
        }
        Protocol::LogitechHidpp => Box::new(logitech::Logitech::new(model, transport)),
        Protocol::LogitechPerKeyLighting2 | Protocol::LogitechPerKeyLighting => {
            Box::new(logitech_rgb::RgbKeyboard::new(model, transport))
        }
        Protocol::MountainDisplayPad => {
            Box::new(mountain::DisplayPad::new(model, transport, display))
        }
        Protocol::XencelabsQuickKeys => Box::new(xencelabs::QuickKeys::new(model, transport)),
        Protocol::MystrixRawHid => Box::new(mystrix::Mystrix::new(model, transport)),
        Protocol::FrameworkRawHid => Box::new(framework::Macropad::new(model, transport)),
        Protocol::AdafruitMacroPadHid => {
            Box::new(macropad::MacroPad::new(model, transport, display))
        }
        Protocol::VecPedal => Box::new(vec::Footpedal::new(model, transport)),
        Protocol::TourBoxSerial => Box::new(tourbox::TourBox::new(model, transport)),
        Protocol::MasterHub => Box::new(masterhub::MasterHub::new(model, transport)),
        Protocol::SpaceMouseHid => Box::new(spacemouse::SpaceMouse::new(model, transport)),
        Protocol::LogitechG13 => Box::new(g13::G13::new(model, transport)),
        Protocol::LogitechG15 => Box::new(g15::G15::new(model, transport)),
        Protocol::MaschineHid => Box::new(maschine::Maschine::new(model, transport)),
        Protocol::TraktorF1Hid => Box::new(traktor::TraktorF1::new(model, transport)),
        Protocol::KompleteKontrolHid => Box::new(komplete::KompleteKontrol::new(model, transport)),
        Protocol::VialRawHid => Box::new(vial::Vial::new(model, transport)),
        Protocol::LampArrayHid => Box::new(lamparray::LampArray::new(model, transport)),
        Protocol::WootingAnalog | Protocol::WootingLegacy => {
            Box::new(wooting::Wooting::new(model, transport, display))
        }
        Protocol::YamahaCc1Serial => Box::new(yamaha::Cc1::new(model, transport)),
        Protocol::XboxGip => Box::new(xbox::Xbox::new(model, transport)),
        Protocol::PlayStationDualSense
        | Protocol::PlayStationDualShock4
        | Protocol::PlayStationAccess => Box::new(playstation::PlayStation::new(model, transport)),
        Protocol::SonyBuzz => Box::new(buzz::Buzz::new(model, transport)),
        Protocol::SteelSeriesOled | Protocol::SteelSeriesOledChunked => {
            Box::new(steelseries::ApexOled::new(model, transport))
        }
        Protocol::GameSirConfig => Box::new(
            gamesir::GameSir::new(model, transport)
                .with_backup_dir(crate::paths::data_dir().join("gamesir")),
        ),
        Protocol::GriffinPowerMate => Box::new(griffin::PowerMate::new(transport)),
        Protocol::DjiDuml => Box::new(dji::Remote::new(model, transport)),
        Protocol::PrehPosKey => Box::new(prehkeytec::PrehKeyTec::new(model, transport)),
        Protocol::SurfaceCoreHid => Box::new(surfacecore::SurfaceCore::new(model, transport)),
        Protocol::MonomeMext | Protocol::MonomeSeries | Protocol::Monome40h => {
            Box::new(monome::Monome::new(model, transport))
        }
        Protocol::NovationLaunchpadMidi => Box::new(launchpad::Launchpad::new(model, transport)),
        Protocol::LaunchpadLegacyMidi => {
            Box::new(launchpad_legacy::LaunchpadLegacy::new(transport))
        }
        Protocol::LaunchpadMk2Midi => Box::new(launchpad_mk2::LaunchpadMk2::new(transport)),
        Protocol::LaunchpadProMidi => Box::new(launchpad_pro::LaunchpadPro::new(transport)),
        Protocol::NovationDicerMidi => Box::new(novation_dicer::NovationDicer::new(transport)),
        Protocol::NovationLaunchControlXl | Protocol::NovationLaunchControl => {
            Box::new(launch_control::LaunchControl::new(model, transport))
        }
        Protocol::NovationLaunchControlXl3 | Protocol::NovationLaunchControl3 => {
            Box::new(launch_control3::LaunchControl3::new(model, transport))
        }
        Protocol::LaunchkeyMk3Midi => Box::new(launchkey::Launchkey::new(model, transport)),
        Protocol::LaunchkeyMk4Midi => Box::new(launchkey_mk4::LaunchkeyMk4::new(model, transport)),
        Protocol::LaunchkeyMk2Midi => Box::new(launchkey_mk2::LaunchkeyMk2::new(model, transport)),
        Protocol::LaunchkeyMiniMidi => Box::new(launchkey_mini::LaunchkeyMini::new(transport)),
        Protocol::AkaiApcMiniMk2Midi => Box::new(akai_apc::ApcMiniMk2::new(model, transport)),
        Protocol::AkaiApcMiniMidi => Box::new(akai_apc_mini::ApcMini::new(model, transport)),
        Protocol::AkaiMidimixMidi => Box::new(akai_midimix::Midimix::new(model, transport)),
        Protocol::AkaiLpd8Midi => Box::new(akai_lpd8::Lpd8::new(model, transport)),
        Protocol::AkaiFireMidi => Box::new(akai_fire::Fire::new(model, transport)),
        Protocol::AkaiMpkMk4Midi => Box::new(akai_mpk_mk4::MpkMk4::new(model, transport)),
        Protocol::AkaiMpkMiniPlusMidi => {
            Box::new(akai_mpk_mini_plus::MpkMiniPlus::new(model, transport))
        }
        Protocol::AkaiApc40Mk2Midi => Box::new(akai_apc40::Apc40Mk2::new(model, transport)),
        Protocol::AkaiApc40Mk1Midi | Protocol::AkaiApc20Midi => {
            Box::new(akai_apc40_mk1::Apc40Mk1::new(model, transport))
        }
        Protocol::AkaiApcKey25Midi | Protocol::AkaiApcKey25Mk2Midi => {
            Box::new(akai_apc_key25::ApcKey25::new(model, transport))
        }
        Protocol::NovationSlMk3Midi => Box::new(sl_mkiii::SlMk3::new(model, transport)),
        Protocol::AbletonPush1Midi => Box::new(push1::Push1::new(model, transport)),
        Protocol::AbletonPush2Midi => Box::new(push2::Push2::new(model, transport)),
        Protocol::BehringerXTouchMiniMidi => {
            Box::new(xtouch_mini::XTouchMini::new(model, transport))
        }
        Protocol::BehringerXTouchOneMidi => Box::new(xtouch_one::XTouchOne::new(model, transport)),
        Protocol::KorgNanoKontrol2 | Protocol::KorgNanoKontrolStudio => {
            Box::new(korg::Korg::new(model, transport))
        }
        Protocol::KorgPadKontrol => Box::new(korg_padkontrol::PadKontrol::new(model, transport)),
        Protocol::MidiFighterTwisterMidi => {
            Box::new(midi_fighter_twister::Twister::new(model, transport))
        }
        Protocol::MidiFighterSpectraMidi => {
            Box::new(midi_fighter_spectra::Spectra::new(model, transport))
        }
        Protocol::MidiFighter64Midi => {
            Box::new(midi_fighter_64::MidiFighter64::new(model, transport))
        }
        Protocol::LividBaseMidi => Box::new(livid_base::Base::new(model, transport)),
        Protocol::AllenHeathXoneK2Midi => Box::new(xone_k2::XoneK2::new(model, transport)),
        Protocol::ExquisMidi => Box::new(exquis::Exquis::new(model, transport)),
        Protocol::MAudioOxygenProMidi => {
            Box::new(maudio_oxygen_pro::OxygenPro::new(model, transport))
        }
        Protocol::IntechGridMidi => Box::new(intech_grid::Grid::new(model, transport)),
        Protocol::PresonusAtomMidi => Box::new(presonus_atom::Atom::new(model, transport)),
        Protocol::PresonusAtomSqMidi => Box::new(presonus_atom_sq::AtomSq::new(model, transport)),
        Protocol::PresonusFaderPortMidi => Box::new(faderport::FaderPort::new(model, transport)),
        Protocol::PresonusFaderPort8Midi => Box::new(faderport8::FaderPort8::new(model, transport)),
        Protocol::BehringerBControlMidi => {
            Box::new(behringer_bcontrol::BControl::new(model, transport))
        }
        Protocol::FaderfoxEc4Midi => Box::new(faderfox_ec4::FaderfoxEc4::new(model, transport)),
        Protocol::IconPlatformMMidi => Box::new(icon_platform::IconPlatform::new(model, transport)),
        Protocol::IconPlatformXPlusMidi => {
            Box::new(icon_platform_x::IconPlatformX::new(model, transport))
        }
        Protocol::IconQconProMidi => Box::new(icon_qcon::IconQcon::new(model, transport)),
        Protocol::IconQconG2Midi => Box::new(icon_qcon::IconQcon::new(model, transport)),
        Protocol::IconP1Midi => Box::new(icon_p1::IconP1::new(model, transport)),
        Protocol::MackieMcuProMidi | Protocol::MackieXtProMidi => {
            Box::new(mackie_mcu::MackieMcu::new(model, transport))
        }
        Protocol::ZoomR16Midi => Box::new(zoom::ZoomR16::new(model, transport)),
        Protocol::ArturiaBeatStepMidi => {
            Box::new(arturia_beatstep::BeatStep::new(model, transport))
        }
        Protocol::ArturiaMiniLab3Midi => {
            Box::new(arturia_minilab3::MiniLab3::new(model, transport))
        }
        Protocol::ArturiaKeyLabEssential3Midi => Box::new(
            arturia_keylab_essential3::KeyLabEssential3::new(model, transport),
        ),
        Protocol::TouchBar => Box::new(touchbar::TouchBar::unlinked(model)),
    }
}

#[cfg(test)]
pub(crate) mod testing {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    use super::{DriverResult, Transport};

    /// Everything a driver sent, shared with the test that inspects it.
    #[derive(Default)]
    pub struct Log {
        pub writes: Vec<Vec<u8>>,
        pub features: Vec<Vec<u8>>,
        pub reads: VecDeque<Vec<u8>>,
        /// Answers to feature-report reads, in order.
        pub feature_replies: VecDeque<Vec<u8>>,
        /// The report descriptor, when the device has one to read.
        pub descriptor: Option<Vec<u8>>,
        /// The usage page the transport says it was opened on.
        pub usage_page: Option<u16>,
        /// The transport plays a Linux input event node.
        pub evdev: bool,
    }

    #[derive(Clone, Default)]
    pub struct Recorder(pub Arc<Mutex<Log>>);

    impl Recorder {
        pub fn queue_read(&self, report: Vec<u8>) {
            self.0.lock().unwrap().reads.push_back(report);
        }

        pub fn queue_feature_reply(&self, report: Vec<u8>) {
            self.0.lock().unwrap().feature_replies.push_back(report);
        }

        pub fn writes(&self) -> Vec<Vec<u8>> {
            self.0.lock().unwrap().writes.clone()
        }

        pub fn features(&self) -> Vec<Vec<u8>> {
            self.0.lock().unwrap().features.clone()
        }
    }

    impl Transport for Recorder {
        fn write(&mut self, data: &[u8]) -> DriverResult<()> {
            self.0.lock().unwrap().writes.push(data.to_vec());
            Ok(())
        }

        fn read_timeout(&mut self, buf: &mut [u8], _timeout_ms: i32) -> DriverResult<usize> {
            let Some(report) = self.0.lock().unwrap().reads.pop_front() else {
                return Ok(0);
            };
            let len = report.len().min(buf.len());
            buf[..len].copy_from_slice(&report[..len]);
            Ok(len)
        }

        fn send_feature_report(&mut self, data: &[u8]) -> DriverResult<()> {
            self.0.lock().unwrap().features.push(data.to_vec());
            Ok(())
        }

        /// A device that was asked for a report it never got answers with an error.
        fn get_feature_report(&mut self, buf: &mut [u8]) -> DriverResult<usize> {
            let Some(report) = self.0.lock().unwrap().feature_replies.pop_front() else {
                return Err(super::DriverError::Io(
                    "no feature report queued".to_owned(),
                ));
            };
            let len = report.len().min(buf.len());
            buf[..len].copy_from_slice(&report[..len]);
            Ok(len)
        }

        fn report_descriptor(&mut self, buf: &mut [u8]) -> DriverResult<usize> {
            let log = self.0.lock().unwrap();
            let Some(descriptor) = &log.descriptor else {
                return Err(super::DriverError::Io("no descriptor".to_owned()));
            };
            let len = descriptor.len().min(buf.len());
            buf[..len].copy_from_slice(&descriptor[..len]);
            Ok(len)
        }

        fn usage_page(&self) -> Option<u16> {
            self.0.lock().unwrap().usage_page
        }

        fn is_evdev(&self) -> bool {
            self.0.lock().unwrap().evdev
        }
    }
}
