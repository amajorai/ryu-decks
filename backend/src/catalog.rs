//! Every control deck this app can drive, keyed by USB vendor/product id.
//!
//! A model row is data, not behavior: the [`Protocol`] says which driver speaks to
//! it and the [`KeyImage`] says how a key face must be encoded. The USB ids and
//! image geometry are interoperability facts, cross-checked against the public
//! open-source drivers for each family (python-elgato-streamdeck and
//! rust-elgato-streamdeck for Elgato, redphx/strmdck and ulanzi-linux for Ulanzi,
//! creator-micro-kit for Work Louder, foxxyz/loupedeck for Loupedeck and Razer,
//! mirajazz, ajazz-sdk, ajazz-rs and the OpenDeck AKP153/AKP03/AKP05 plugins for
//! Mirabox and Ajazz, node-logitech-mx-creative-console for the Logitech MX
//! Creative Console, mountain-displaypad for the Mountain DisplayPad,
//! node-xencelabs-quick-keys and companion-surface-xencelabs-quick-keys for the
//! Xencelabs Quick Keys, node-blackmagic-controller and blackmagic-misc for the
//! Blackmagic Design panels, companion-surface-203-systems-mystrix and the
//! MatrixOS firmware for the 203 Systems Mystrix,
//! companion-surface-framework-macropad and Julusian's framework_qmk_firmware
//! for the Framework Laptop 16 RGB Macropad, companion-surface-vec-footpedal
//! for the VEC footpedal, AndyCappDev's tuxbox for the TourBox, and
//! @elgato-stream-deck/core and companion-surface-elgato-stream-deck for the
//! Corsair Galleon 100 SD, companion-surface-coolermaster-masterhub for the
//! Cooler Master MasterHub modules, and pyspacemouse, spacemouse-node and
//! 3dmouse-hid for the 3Dconnexion 3D mice, g13-rs, ecraven/g13 and
//! g13pad for the Logitech G13, the Linux kernel's hid-lg-g15 and g15tools'
//! libg15 for the Logitech G15, G15 v2, G11, G510 and G110, and Phil Hadfield's
//! companion-surface-yamaha-cc1 for the Yamaha CC121MK2, and Daniel Jocher's
//! companion-surface-prehkeytec-keyboard for the PrehKeyTec POS keyboards, and
//! Joseph Adams's companion-surface-dji-controller for the DJI RC-N1, and
//! SoundMinded's SurfaceCore firmware and Companion module for the Haute42 B16,
//! and shaduzlabs/cabl, cross-checked against wrl/maschine.rs, for the Native
//! Instruments Maschine controllers, and against mixxxdj/mixxx's mapping for the
//! Traktor Kontrol F1; shaduzlabs/cabl alone for the Komplete Kontrol S-series MK1;
//! and WootingKb's wooting-rgb-sdk and wooting-analog-sdk for the Wooting keyboards;
//! dualsense-ts, @hcodes/webhid-ds4, ps-access and the Linux kernel's
//! hid-playstation for the Sony PlayStation controllers; @thalaguer/buzzer and
//! the Linux kernel's hid-sony for the Sony Buzz! buzzers; Maxim Kulkin's
//! nodejs-macropad and its CircuitPython firmware for the Adafruit MacroPad;
//! Pedro Segato's gamesir-reverse-engineering for the GameSir Cyclone 2;
//! not-jan's apex-tux for the SteelSeries Apex keyboards' OLED; the Linux
//! kernel's powermate and hid-quirks drivers, crash7's griffin_powermate,
//! blastur's pypowermate and sandeepmistry's node-powermate for the Griffin
//! PowerMate; Novation's Programmer's reference manuals, cross-checked against
//! launchpad.js, Mixxx's mapping and Ableton Live's remote scripts, for the
//! Novation Launchpads; Novation's MK2 Programmer's Reference Guide,
//! cross-checked against launchy, Mixxx's mapping and DrivenByMoss, for the
//! Launchpad MK2; the launchy crate, launchpad-mini, launchpad.py,
//! midi-launchpad, Mixxx's mappings and Ableton Live's remote script for the
//! bi-colour Launchpads; launch-control-xl, launchy, @mohayonao/launch-control,
//! DrivenByMoss and Ableton Live's remote scripts for the Launch Controls; and
//! Ableton Live's remote script, tstriker's akai-apc-mini-mk2 and DJj123dj's
//! apc-mini-mk2 for the Akai APC mini MK2; Ableton's APC_mini Remote Script as
//! published in gluon/AbletonLive12_MIDIRemoteScripts, cross-checked against
//! DrivenByMoss, for the first APC mini; Ableton's APC_Key_25 and
//! APC_Key_25_mk2 Remote Scripts, same source, for the APC Key 25s; the
//! akai-midimix npm package, cross-checked against Ableton's MIDI_Mix Remote
//! Script, same source, for the MIDImix; Ableton's BeatStep and _Arturia
//! Remote Scripts, same source, cross-checked against DrivenByMoss and the
//! beatstep npm package, for the Arturia BeatStep; Ableton's MiniLab_3
//! Remote Script, same source, for the Arturia MiniLab 3; Bitwig's MPK mini
//! IV / MPK mini Plus II extension (MIT, facts only) and Ableton's MPK_mini_IV
//! Remote Script, same source, for the Akai MPK minis; Bitwig's MPK mini
//! Plus extension (MIT, facts only) and Ableton's MPK_mini_Plus Remote
//! Script, same source, for the first MPK mini Plus; Bitwig's KeyLab
//! Essential mk3 extension (MIT, facts only) and Ableton's KeyLab_Essential_mk3
//! Remote Script, same source, for the Arturia KeyLab Essential mk3; DJ TechTools'
//! Midi_Fighter_Twister_Open_Source firmware, facts only, cross-checked
//! against Mixxx's mapping, for the Midi Fighter Twister; the akai-lpd8 npm package,
//! cross-checked against Ableton's LPD8 Remote Script, same source, and
//! Mixxx's Akai-LPD8-RK mapping, for the Akai LPD8; DrivenByMoss for the Akai
//! Fire; Intech Studio's grid-fw firmware and grid-editor, facts only, for
//! the Grid modules; Ableton's ATOM and ATOM SQ Remote Scripts, same source,
//! for the PreSonus ATOM and ATOM SQ; Ardour's FaderPort and FaderPort 8
//! surfaces (GPL-2.0-or-later, facts only) for the PreSonus FaderPorts, with
//! the newer FaderPorts' USB ids from Ableton's Remote Scripts).
//!
//! MIDI decks (the Launchpads, Launch Controls, Launchkeys, the Akai APCs, MIDImix, LPD8, Fire, MPK mini IV and MPK mini Plus, the Push 2, the Arturia BeatStep, MiniLab 3 and KeyLab Essential mk3, the Midi Fighter Twister, the Intech Studio Grid, the PreSonus ATOM and ATOM SQ and the PreSonus FaderPorts) list their USB ids for display only: midir sees
//! no USB ids, so they are found by their MIDI port names
//! ([`identify_midi`]) and never by [`identify`].
//!
//! QMK keyboards and macropads on Vial firmware are not listed: they share no
//! USB ids, so the device loop finds them by Vial's serial-number marker and
//! builds each one's model from what the board reports (see [`vial`]). HID
//! LampArray devices (Windows Dynamic Lighting) are not listed either: they are
//! found by their Lighting and Illumination collection, and each one's model is
//! built from the lamps it reports (see [`lamparray`]).

use serde::Serialize;

use crate::driver::{
    akai_apc, akai_apc40, akai_apc40_mk1, akai_apc_key25, akai_apc_mini, akai_fire, akai_lpd8,
    akai_midimix, akai_mpk_mini_plus, akai_mpk_mk4, arturia_beatstep, arturia_keylab_essential3,
    arturia_minilab3, behringer_bcontrol, blackmagic, buzz, dji, exquis, faderfox_ec4, faderport,
    faderport8, g13, g15, gamesir, icon_p1, icon_platform, icon_platform_x, icon_qcon, intech_grid,
    komplete, korg, korg_padkontrol, lamparray, launch_control, launch_control3, launchkey,
    launchkey_mini, launchkey_mk2, launchkey_mk4, launchpad, launchpad_legacy, launchpad_mk2,
    launchpad_pro, livid_base, logitech_rgb, mackie_mcu, macropad, maschine, masterhub,
    maudio_oxygen_pro, midi, midi_fighter_64, midi_fighter_spectra, midi_fighter_twister, monome,
    mystrix, novation_dicer, playstation, prehkeytec, presonus_atom, presonus_atom_sq, push1,
    push2, sl_mkiii, spacemouse, steelseries, surfacecore, tourbox, traktor, vial, wooting, xbox,
    xone_k2, xtouch_mini, xtouch_one, yamaha, zoom,
};

pub const ELGATO_VENDOR_ID: u16 = 0x0fd9;
/// Corsair's vendor id. The Galleon 100 SD is a keyboard, so the Stream Deck
/// panel is only one of the HID interfaces on its product id.
pub const CORSAIR_VENDOR_ID: u16 = 0x1b1c;
pub const ULANZI_VENDOR_ID: u16 = 0x2207;
/// Cooler Master's vendor id. Every MasterHub module is a HID device of its own
/// on it, each with its own product id.
pub const COOLER_MASTER_VENDOR_ID: u16 = 0x2516;
/// Work Louder's current boards ship on Espressif's vendor id.
pub const WORK_LOUDER_VENDOR_ID: u16 = 0x303a;
pub const LOUPEDECK_VENDOR_ID: u16 = 0x2ec2;
pub const RAZER_VENDOR_ID: u16 = 0x1532;
/// Ajazz's own vendor id. Mirabox builds the same decks for many brands, each on
/// its own vendor id, so most Mirabox rows list their rebrands in `more_usb_ids`.
pub const AJAZZ_VENDOR_ID: u16 = 0x0300;
pub const MIRABOX_VENDOR_ID: u16 = 0x5548;
pub const APPLE_VENDOR_ID: u16 = 0x05ac;
pub const XKEYS_VENDOR_ID: u16 = 0x05f3;
pub const CONTOUR_VENDOR_ID: u16 = 0x0b33;
pub const LOGITECH_VENDOR_ID: u16 = 0x046d;
/// 3Dconnexion's own vendor id. Its older 3D mice used Logitech's
/// (0x046d), on product ids no Logitech deck here shares.
pub const THREEDCONNEXION_VENDOR_ID: u16 = 0x256f;
/// The Generic Desktop page, where a 3D mouse's multi-axis collection sits.
pub const GENERIC_DESKTOP_USAGE_PAGE: u16 = 0x0001;
pub const MOUNTAIN_VENDOR_ID: u16 = 0x3282;
pub const XENCELABS_VENDOR_ID: u16 = 0x28bd;
pub const BLACKMAGIC_VENDOR_ID: u16 = 0x1edb;
/// 203 Systems' vendor id (Matrix OS devices).
pub const SYSTEMS203_VENDOR_ID: u16 = 0x0203;
/// The Mystrix's vendor collection, which carries Matrix OS's report 0xFF. Its
/// keyboard, mouse, consumer and gamepad collections sit on standard pages.
pub const MYSTRIX_USAGE_PAGE: u16 = 0xff00;
/// A Mystrix answers to 0x1040 plus the device id (0 to 63) set on it, so every
/// one of these 64 product ids is the same deck.
const MYSTRIX_PRODUCT_IDS: [u16; 64] = mystrix_product_ids();

const fn mystrix_product_ids() -> [u16; 64] {
    let mut ids = [0u16; 64];
    let mut at = 0;
    while at < ids.len() {
        ids[at] = 0x1040 + at as u16;
        at += 1;
    }
    ids
}
/// Framework Computer's vendor id. Its keyboard modules share it on other
/// product ids; only 0x0013 is the RGB Macropad.
pub const FRAMEWORK_VENDOR_ID: u16 = 0x32ac;
/// The raw-HID collection (usage 0x61) of the Companion macropad firmware.
/// Stock firmware puts VIA on 0xff60 instead, which Ryu does not drive, and the
/// keyboard and QMK console collections never answer the raw-HID commands.
pub const FRAMEWORK_MACROPAD_USAGE_PAGE: u16 = 0xffdd;
/// Adafruit's USB vendor id, which CircuitPython boards use.
pub const ADAFRUIT_VENDOR_ID: u16 = 0x239a;
/// The Consumer page, which every collection of the MacroPad's Companion
/// firmware is on (as is stock CircuitPython's consumer control, so the
/// firmware is told apart by its report descriptor).
pub const CONSUMER_USAGE_PAGE: u16 = 0x000c;
/// The vendor collection the Quick Keys takes commands on (interface 2).
pub const XENCELABS_USAGE_PAGE: u16 = 0xff0a;
/// TourBox Elite / Elite Plus, on Keil's vendor id.
pub const TOURBOX_VENDOR_ID: u16 = 0xc251;
/// The TourBox Neo's id: ArteryTek AT32's generic USB serial id, which other
/// boards share, so a port on it is only claimed once it answers the unlock.
pub const TOURBOX_NEO_USB_ID: (u16, u16) = (0x2e3c, 0x5740);
/// TinyUSB's generic serial id. One installer comment calls it a Neo; it is
/// claimed, like the Neo's, only once it answers the unlock.
pub const TOURBOX_TINYUSB_USB_ID: (u16, u16) = (0xcafe, 0x4001);
/// Yamaha Corporation's USB vendor id.
pub const YAMAHA_VENDOR_ID: u16 = 0x0499;
/// DJI's USB vendor id (its drone remote controllers).
pub const DJI_VENDOR_ID: u16 = 0x2ca3;
/// SoundMinded's SurfaceCore firmware: TinyUSB's default test vendor id, which
/// other hobby boards share (the TourBox TinyUSB id above is on it too), so the
/// driver confirms the board from its device-info answer.
pub const SURFACECORE_USB_ID: (u16, u16) = (0xcafe, 0x2043);
/// SurfaceCore's vendor HID collection (interface 4). Its CDC and MIDI
/// interfaces are never opened.
pub const SURFACECORE_USAGE_PAGE: u16 = 0xff00;
/// Native Instruments' USB vendor id (the Maschine, Traktor Kontrol and Komplete
/// Kontrol controllers).
pub const NATIVE_INSTRUMENTS_VENDOR_ID: u16 = 0x17cc;
/// Focusrite-Novation's USB vendor id (the Launchpads, Launch Controls and
/// Launchkeys). Shown only: a MIDI
/// deck is found by its port name, never by USB id.
pub const NOVATION_VENDOR_ID: u16 = 0x1235;
/// Akai Professional's USB vendor id (the APCs). Shown only: a MIDI
/// deck is found by its port name, never by USB id.
pub const AKAI_VENDOR_ID: u16 = 0x09e8;
/// Ableton's USB vendor id (the Push 2). Shown only: a MIDI deck is found by
/// its port name, never by USB id.
pub const ABLETON_VENDOR_ID: u16 = 0x2982;
/// Behringer's USB vendor id (the X-Touch Mini and One). Shown only: a MIDI deck is
/// found by its port name, never by USB id.
pub const BEHRINGER_VENDOR_ID: u16 = 0x1397;
/// iCON Pro Audio's USB vendor id (the P1-Nano). Shown only: a MIDI deck is
/// found by its port name, never by USB id.
pub const ICON_VENDOR_ID: u16 = 0x1d03;
/// Zoom's vendor id. The R16 and R24 recorders share product id 0x00dd in
/// their audio interface mode.
pub const ZOOM_VENDOR_ID: u16 = 0x1686;
/// Mackie Designs' USB vendor id (the MCU Pro). Shown only: a MIDI deck is
/// found by its port name, never by USB id.
pub const MACKIE_VENDOR_ID: u16 = 0x0a73;
/// KORG's USB vendor id (the nanoKONTROL2 and nanoKONTROL Studio). Shown
/// only: a MIDI deck is found by its port name, never by USB id.
pub const KORG_VENDOR_ID: u16 = 0x0944;
/// Arturia's USB vendor id (the BeatStep). Shown only: a MIDI deck is found by
/// its port name, never by USB id.
pub const ARTURIA_VENDOR_ID: u16 = 0x1c75;
/// DJ TechTools' USB vendor id (the Midi Fighter Twister). Shown only: a MIDI
/// deck is found by its port name, never by USB id.
pub const DJTT_VENDOR_ID: u16 = 0x2580;
/// PreSonus' USB vendor id (the ATOM, ATOM SQ and FaderPorts). Shown only: a MIDI deck is found by its
/// port name, never by USB id.
pub const PRESONUS_VENDOR_ID: u16 = 0x194f;
/// M-Audio's USB vendor id (the Oxygen Pros and Hammer 88 Pro). Shown only: a
/// MIDI deck is found by its port name, never by USB id.
pub const MAUDIO_VENDOR_ID: u16 = 0x0763;
/// The USB ids of Intech Studio's Grid modules on Espressif (ESP32-S3) and
/// Raspberry Pi (RP2350) silicon; the Atmel (D51) ones are on
/// [`ATMEL_VENDOR_ID`]. Shown only: a MIDI deck is found by its port name,
/// never by USB id.
pub const INTECH_GRID_ESP32_USB_ID: (u16, u16) = (0x303a, 0x8123);
pub const INTECH_GRID_RP2350_USB_ID: (u16, u16) = (0x2e8a, 0x1145);
/// FTDI, whose stock USB serial bridge older monome grids and arcs use. Shown
/// only: monome devices are found by their USB serial number, never by USB id.
pub const FTDI_VENDOR_ID: u16 = 0x0403;
/// Wooting's own USB vendor id (every keyboard on firmware from 2020 on).
pub const WOOTING_VENDOR_ID: u16 = 0x31e3;
/// Atmel's USB vendor id, which the first Wooting One and Two firmware used.
/// Other Atmel boards share it, so only Wooting's two product ids on it are
/// listed, and only their analog collection (a Wooting usage page) is claimed.
pub const ATMEL_VENDOR_ID: u16 = 0x03eb;
/// Microsoft's USB vendor id (the Xbox controllers).
pub const MICROSOFT_VENDOR_ID: u16 = 0x045e;
/// Sony's USB vendor id (the PlayStation controllers and the Buzz! buzzers).
pub const SONY_VENDOR_ID: u16 = 0x054c;
/// GameSir's USB vendor id (the Cyclone 2, wired and through its dongle).
pub const GAMESIR_VENDOR_ID: u16 = 0x3537;
/// GameSir's vendor configuration collection (usage 0x40), which GameSir
/// Connect drives. The XInput gamepad, keyboard, mouse and consumer
/// collections on the same ids are never opened.
pub const GAMESIR_USAGE_PAGE: u16 = 0xfff0;
/// SteelSeries' USB vendor id (the Apex keyboards with an OLED).
pub const STEELSERIES_VENDOR_ID: u16 = 0x1038;
/// Griffin Technology's USB vendor id (the PowerMate knob).
pub const GRIFFIN_VENDOR_ID: u16 = 0x077d;
/// PrehKeyTec's USB vendor id (its programmable POS keyboards).
pub const PREHKEYTEC_VENDOR_ID: u16 = 0x053a;
/// Allen & Heath (Mixxx's udev rules). The Xone:K2's product id is not
/// known; it is found by MIDI port name and its vendor id is shown only.
pub const ALLEN_HEATH_VENDOR_ID: u16 = 0x22f0;
/// The POSKey collection of a PrehKeyTec keyboard (interface 1 on the units
/// seen). Its ordinary keyboard interface is never opened.
pub const PREHKEYTEC_USAGE_PAGE: u16 = 0xfffa;
/// Usage pages from here up are vendor-defined. Logitech's HID++ collection is one
/// of them; the keyboard and consumer collections of the same device never are.
pub const VENDOR_USAGE_PAGES: u16 = 0xff00;
/// The vendor collection Mirabox decks take commands on (usage page 0xffa0).
pub const MIRABOX_USAGE_PAGE: u16 = 0xffa0;
/// The vendor HID collection that carries Work Louder's JSON-RPC channel. The
/// keyboard and consumer-control collections of the same device never answer it.
pub const WORK_LOUDER_RPC_USAGE_PAGE: u16 = 0xff00;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Family {
    Elgato,
    Ulanzi,
    WorkLouder,
    Loupedeck,
    Mirabox,
    Apple,
    Infinitton,
    // `snake_case` alone would send "x_keys"; the Companion expects "xkeys".
    #[serde(rename = "xkeys")]
    XKeys,
    Contour,
    Logitech,
    Mountain,
    Xencelabs,
    Blackmagic,
    #[serde(rename = "203systems")]
    Systems203,
    Framework,
    Vec,
    // `snake_case` alone would send "tour_box".
    #[serde(rename = "tourbox")]
    TourBox,
    /// The Stream Deck panel Corsair builds into the Galleon 100 SD keyboard.
    Corsair,
    /// The Cooler Master MasterHub's modules, each a deck of its own.
    CoolerMaster,
    // `snake_case` alone would send "three_dconnexion".
    #[serde(rename = "3dconnexion")]
    ThreeDconnexion,
    /// QMK keyboards and macropads on Vial firmware, whatever their maker.
    Qmk,
    Yamaha,
    /// Xbox controllers.
    Microsoft,
    /// PrehKeyTec's programmable POS keyboards (MCI and MSI).
    // `snake_case` alone would send "preh_key_tec".
    #[serde(rename = "prehkeytec")]
    PrehKeyTec,
    /// DJI's drone remote controllers.
    Dji,
    /// Haute42's leverless controllers, on SoundMinded's SurfaceCore firmware.
    Haute42,
    /// Native Instruments' Maschine, Traktor Kontrol and Komplete Kontrol
    /// controllers, in HID mode.
    NativeInstruments,
    /// monome grids and arcs over USB serial.
    Monome,
    /// HID LampArray devices (Windows Dynamic Lighting), whatever their maker.
    LampArray,
    /// Wooting's analog keyboards.
    Wooting,
    /// Sony's PlayStation controllers and Buzz! quiz buzzers.
    Sony,
    /// The Adafruit MacroPad RP2040, on the Companion CircuitPython firmware.
    Adafruit,
    /// GameSir controllers (the Cyclone 2).
    // `snake_case` alone would send "game_sir".
    #[serde(rename = "gamesir")]
    GameSir,
    /// SteelSeries Apex keyboards, for their OLED alone.
    // `snake_case` alone would send "steel_series".
    #[serde(rename = "steelseries")]
    SteelSeries,
    /// Griffin Technology's PowerMate knob.
    Griffin,
    /// Novation's Launchpads and Launch Controls, over USB MIDI.
    Novation,
    /// Akai Professional's APC mini MK2, over USB MIDI.
    Akai,
    /// Ableton's Push 2, over USB MIDI.
    Ableton,
    /// Behringer's X-Touch Mini and X-Touch One, over USB MIDI.
    Behringer,
    /// KORG's nanoKONTROL2 and nanoKONTROL Studio, over USB MIDI.
    Korg,
    /// Arturia's BeatStep, over USB MIDI.
    Arturia,
    /// DJ TechTools' Midi Fighter Twister, over USB MIDI.
    #[serde(rename = "djtechtools")]
    DjTechTools,
    /// Intech Studio's Grid modules, over USB MIDI.
    Intech,
    /// PreSonus' ATOM, ATOM SQ and FaderPorts, over USB MIDI.
    Presonus,
    /// Faderfox's EC4, over USB MIDI.
    Faderfox,
    /// Livid Instruments' Base and Base II, over USB MIDI.
    Livid,
    /// Allen & Heath's Xone:K2, over USB MIDI.
    AllenHeath,
    /// Intuitive Instruments' (Dualo's) Exquis, over USB MIDI.
    IntuitiveInstruments,
    /// M-Audio's Oxygen Pros and Hammer 88 Pro, over USB MIDI.
    // `snake_case` alone would send "m_audio".
    #[serde(rename = "maudio")]
    MAudio,
    /// iCON Pro Audio's Platform M+ and Platform M, QCon Pro X, XS1, Pro G2
    /// and EX G2 and P1-Nano, over USB MIDI.
    Icon,
    /// Zoom's R16 and R24 recorders as DAW control surfaces, over USB MIDI.
    Zoom,
    /// Mackie's MCU Pro and XT Pro control surfaces, over USB MIDI.
    Mackie,
}

/// The wire dialect a driver must speak. Elgato alone has four.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    /// The 2017 Stream Deck: 8191-byte BMP pages and right-to-left key numbering.
    ElgatoOriginal,
    /// Stream Deck Mini: 1024-byte BMP pages with a 16-byte header.
    ElgatoMini,
    /// Every later Stream Deck: 1024-byte JPEG pages with an 8-byte header.
    ElgatoV2,
    /// Stream Deck + and + XL: the V2 key protocol plus dial and touch-strip reports.
    ElgatoPlus,
    /// The Corsair Galleon 100 SD's Stream Deck panel: the + reports and image
    /// pages, plus a keep-alive feature report every 500 ms and ring lights
    /// set one LED per feature report.
    ElgatoGalleon,
    /// Ulanzi D200 family: framed 1024-byte packets and a zipped button grid.
    UlanziD200,
    /// Work Louder Creator Micro 2 / Codex Micro: JSON-RPC over a vendor report.
    WorkLouderRpc,
    /// Loupedeck Live, Live S, CT and Razer Stream Controllers: WebSocket-framed
    /// messages over a USB serial port, keys drawn into one RGB565 framebuffer.
    Loupedeck,
    /// Mirabox / Ajazz first generation: 512-byte "CRT" packets, presses only.
    MiraboxV1,
    /// Mirabox / Ajazz second generation: 1024-byte packets, presses only.
    MiraboxV2,
    /// Mirabox / Ajazz third generation: 1024-byte packets, presses and releases.
    MiraboxV3,
    /// Infinitton iDisplay: raw BGR faces in two fixed-header pages, committed
    /// with a feature report; one input bit per key.
    Infinitton,
    /// X-keys with a blue and a red backlight bank, on or off per key.
    XKeysLegacy,
    /// X-keys XK-24M-RGB: one green-red-blue backlight per key.
    XKeysRgb24,
    /// X-keys X-blox modules: two full-RGB backlights per key.
    XKeysRgbBanks,
    /// Contour ShuttleXpress / ShuttlePro: buttons, a jog counter and a shuttle
    /// position in one report; nothing to draw.
    ContourShuttle,
    /// Logitech MX Creative Keypad and Dialpad: HID++ 2.0 requests, one at a
    /// time, each answered before the next; the keypad's faces are JPEG
    /// rectangles of one 480 x 480 panel.
    LogitechHidpp,
    /// Mountain DisplayPad: input and commands on HID interface 3, raw BGR key
    /// faces on interface 1, one key at a time, each start and finish answered.
    MountainDisplayPad,
    /// Xencelabs Quick Keys: 32-byte commands, key labels as UTF-16 text on one
    /// OLED, an RGB wheel ring; the wireless dongle addresses each deck by id.
    XencelabsQuickKeys,
    /// Blackmagic Design's DaVinci Resolve editors and ATEM Micro Panel: silent
    /// until a feature-report handshake, which must be repeated; held keycodes,
    /// jog and T-bar reports in; whole-state light bitfields out.
    BlackmagicPanel,
    /// 203 Systems Mystrix: 64-byte reports with id 0xFF to and from the Matrix
    /// OS Companion app; a pad colour per command, presses and releases in.
    MystrixRawHid,
    /// Framework Laptop 16 RGB Macropad on the Companion raw-HID firmware:
    /// 32-byte reports with no id; one key's colour or every key's per command,
    /// presses and releases in.
    FrameworkRawHid,
    /// Adafruit MacroPad RP2040 on the Companion CircuitPython HID firmware:
    /// report `01` (up to six held key numbers, 1 to 12) in; report `02`
    /// (one key's number and R, G, B) out. Every collection is on the Consumer
    /// page; the firmware is recognised by its report descriptor, and Windows
    /// lists the key and colour collections as separate paths. No handshake,
    /// keep-alive or brightness command.
    AdafruitMacroPadHid,
    /// VEC footpedal (made by P.I. Engineering): a 2-byte report with no id
    /// holding the three pedals' bits; nothing is ever written to it.
    VecPedal,
    /// TourBox Neo and Elite over USB serial: unlocked by an 8-byte command and
    /// a 94-byte config, then one byte per press, release or detent.
    TourBoxSerial,
    /// Cooler Master MasterHub modules: one envelope (`01 channel len`) for
    /// every module's input, 72 x 72 keypad and 240 x 240 dial JPEGs in
    /// 320-byte chunks, and 64-byte light reports for the knobs, sliders and base.
    MasterHub,
    /// 3Dconnexion 3D mice: input reports 1 (translation, and rotation too in
    /// a 13-byte report), 2 (rotation) and 3 (a button bitmap); the only
    /// output is the LED, report `04 01` on and `04 00` off.
    SpaceMouseHid,
    /// Logitech G915 (wired) and G815: HID++ 2.0 on the vendor collection
    /// (usage page 0xFF00, usage 2), one request at a time. G-key presses
    /// arrive as a GamingGKeys (0x8010) bitmask once diverted; the lights are
    /// PerKeyLighting2 (0x8081) zones, committed per frame, under RgbEffects
    /// (0x8071) software control, which is reasserted every heartbeat.
    LogitechPerKeyLighting2,
    /// Logitech G910, G513 and G Pro keyboard: the same G-key bitmask (the
    /// G910 only); lights as PerKeyLighting (0x8080) key groups in unanswered
    /// 64-byte `0x12` frames and a commit, after a fixed ColorLedEffects
    /// (0x8070) colour that stops the onboard effect.
    LogitechPerKeyLighting,
    /// Logitech G13: an 8-byte whole-state report (stick X and Y, a 40-bit key
    /// bitmap) in; the 160 x 43 LCD as output report 3 (960 bytes in 8-row
    /// column bands), and feature reports 7 (backlight RGB) and 5 (M-key LEDs).
    LogitechG13,
    /// Logitech G15, G15 v2, G11, G510 and G110, the G-key block's vendor
    /// collection: whole-state key reports (a scrambled 9-byte bitmap on the
    /// G15 and G11) in; the G13's LCD report 3 out, and feature reports for
    /// the backlight levels or colour and the M-key LEDs.
    LogitechG15,
    /// QMK boards on Vial firmware: 32-byte VIA / Vial raw-HID commands on usage
    /// page 0xFF60, each answered before the next; the layout read from the
    /// board, keys polled from its switch matrix once unlocked, VialRGB
    /// direct-mode lights. Never matched by USB id.
    VialRawHid,
    /// Yamaha CC121MK2 over USB serial: framed, checksummed messages ending in
    /// 0xC1, silent until a handshake and dropped without a keep-alive; key
    /// faces in RGB565 tiles of one 480 x 272 panel, ten-colour lights.
    YamahaCc1Serial,
    /// Xbox controllers as HID gamepads: GIP input frames (`20 ..`) and Guide
    /// frames (`07 ..`) over USB, or Bluetooth report 1. Input only; nothing is
    /// ever written to the pad.
    XboxGip,
    /// The DualSense and DualSense Edge over USB: input report 0x01 (64
    /// bytes), the light bar set by output report 0x02 (48 bytes).
    PlayStationDualSense,
    /// The DualShock 4 over USB: input report 0x01 (64 bytes), the light bar
    /// set by output report 0x05 (32 bytes).
    PlayStationDualShock4,
    /// The PlayStation Access controller over USB: the raw buttons and stick
    /// of input report 0x01 (64 bytes), its light set by output report 0x02
    /// (32 bytes).
    PlayStationAccess,
    /// Sony Buzz! buzzers (four handsets on one hub or wireless receiver): a
    /// report with no id whose bytes 2 to 4 hold the twenty buttons, and one
    /// output report with no id that turns the four dome LEDs on or off.
    SonyBuzz,
    /// GameSir's configuration protocol (the Cyclone 2), on the vendor
    /// collection at usage page 0xFFF0: 64-byte reports, `0F cmd ..` out and
    /// `10 cmd ..` answers in, one command at a time. The state report `12`
    /// streams only while `0F F2 00` arrives every second; the deck light is
    /// written into the stored light profile (profile 32).
    GameSirConfig,
    /// The Griffin PowerMate (USB): input reports with no id, the push in
    /// byte 0 bit 0 and the turn as a signed byte 1; the base light set by
    /// Griffin's vendor request sent as a 9-byte feature report. On Linux the
    /// kernel hides it from hidraw, so it is reached through its input event
    /// node instead (`BTN_0`, `REL_DIAL`, and `MSC_PULSELED` for the light).
    GriffinPowerMate,
    /// The 128 x 40 OLED of a SteelSeries Apex keyboard, on HID interface 1:
    /// one 642-byte feature report per frame, `61`, 640 bytes of one bit per
    /// pixel (row by row on the earlier boards, the panel's 8-row pages on
    /// the Apex Pro Gen 3), then `00`. Write only: no input, init,
    /// keep-alive or brightness command; the keys type as usual.
    SteelSeriesOled,
    /// The Apex Pro TKL Wireless Gen 3's OLED, on HID interface 3: the paged
    /// frame in eight 641-byte feature reports of 80 bytes each, headed by
    /// the frame command (`0C` by cable, `4C` through the dongle), `01`, the
    /// offset (16-bit, little-endian), `50` and `00`.
    SteelSeriesOledChunked,
    /// PrehKeyTec POS keyboards' POSKey collection: report 0x04 carrying one
    /// key's scan-code-shaped token, or 0xFE and the token for a release. The
    /// only output is the identity query, which names the model (several share
    /// a product id). Never matched by product id alone: only the vendor
    /// collection on usage page 0xFFFA.
    PrehPosKey,
    /// DJI drone remote controllers over USB serial: DUML frames (CRC-8 header,
    /// CRC-16 trailer). Silent until simulator mode is turned on; the stick and
    /// wheel channels are then asked for every 20 ms. One of the remote's two
    /// serial ports answers, so every port is probed. Input only.
    DjiDuml,
    /// SoundMinded's SurfaceCore firmware (on the Haute42 B16): 64-byte vendor
    /// HID reports with no id; `01 id state` per button edge in, `02 id R G B`
    /// per light out, `04` asking which board it is. No brightness command.
    SurfaceCoreHid,
    /// Native Instruments Maschine MK2, Mikro MK2 and Jam in HID mode: report
    /// `01` (a button bitfield and absolute encoder counters), `20` (pad
    /// pressure words) and the Jam's `02` (touch strips) in; whole light
    /// reports (`80` to `82`) and 256-byte display chunks (`E0`, `E1`) out.
    /// No handshake, keep-alive or brightness command.
    MaschineHid,
    /// Native Instruments Traktor Kontrol F1 in HID mode, on HID interface 1:
    /// report `01` (a button bitfield, a wrapping 8-bit encoder and eight
    /// 12-bit knob and fader readings) in; one whole light report `80` (pads as
    /// blue, red, green, 7 bits each) out. No handshake, keep-alive or
    /// brightness command.
    TraktorF1Hid,
    /// Native Instruments Komplete Kontrol S-series MK1, the HID control
    /// section: `A0 00 00` to start; report `01` (a button bitfield, a 4-bit
    /// browse encoder and eight 10-bit wrapping encoders) in; button LEDs
    /// (`80`) and the RGB light guide (`82`) out. No keep-alive or brightness
    /// command; the piano keys are MIDI and never read.
    KompleteKontrolHid,
    /// monome grids from 2011 and arcs over USB serial (mext): a header byte
    /// (`section << 4 | command`) and a payload its header fixes. Asked which
    /// sections it has (`00`) and its grid size (`05`) when found; keys `20`
    /// / `21 x y` and encoder deltas `50 n d` in; 16-level 8 x 8 block maps
    /// (`1A`) and whole rings (`91`) out. Never matched by USB id.
    MonomeMext,
    /// monome 64 and 128 grids from 2007 to 2010 (series): two-byte messages,
    /// keys `00` / `10 xy` in; on/off 8 x 8 quadrant frames (`8q`) out.
    MonomeSeries,
    /// monome 40h and the arduinome: two-byte messages, keys `01` / `00 xy`
    /// in; on/off rows (`7y`) out.
    Monome40h,
    /// HID LampArray (the Lighting and Illumination page, 0x59): feature
    /// reports only, laid out by the device's report descriptor. The array's
    /// and each lamp's attributes are read once; autonomous mode is turned off
    /// while Ryu holds the lamps, which take range and multi-lamp updates, the
    /// last one marked complete, at most one per the device's update interval.
    /// No input. Never matched by USB id.
    LampArrayHid,
    /// Wooting keyboards on firmware from 2020 on (vendor id 0x31E3), through
    /// two collections: analog key reports with no id on page 0xFF53 (four
    /// bytes a key: matrix position, code, actuated bit and a 10-bit depth) or
    /// 0xFF54 (three bytes a key: HID usage and an 8-bit depth) in, and on the
    /// lighting collection (page 0x1337, or 0xFF55 with numbered reports) a
    /// whole 6 x 21 RGB565 colour frame out, after an 8-byte colour-init
    /// feature command whose answer is read back. No keep-alive or brightness
    /// command.
    WootingAnalog,
    /// The first Wooting One and Two firmware (Atmel's vendor id): the same
    /// analog collection on page 0xFF54, and colours as raw, gamma-corrected
    /// LED driver memory in four or five CRC-checked 129-byte reports.
    WootingLegacy,
    /// Novation Launchpad Mini MK3, Launchpad X and Launchpad Pro MK3 in
    /// Programmer mode, over the device's `MIDI` port (never its `DAW` or `DIN`
    /// port): Novation SysEx (`F0 00 20 29 02 <dev>`, `dev` 0D, 0C or 0E) to
    /// enter Programmer mode (`0E 01`) and set RGB LEDs (`03`); Note On / Off
    /// from the pads and Control Change from the buttons in. No keep-alive;
    /// brightness is applied by scaling the colours. Never matched by USB id.
    NovationLaunchpadMidi,
    /// Novation's bi-colour Launchpads (the original, S and Mini MK1/MK2) in
    /// their default X-Y layout, over their one MIDI port: Note On (grid and
    /// scene keys) and Control Change (top row) on channel 1 both ways, LEDs at
    /// four red and four green levels, `92` rapid updates for a whole-deck
    /// repaint, `B0 00 00` to reset. No SysEx, no keep-alive; brightness scales
    /// the levels. Never matched by USB id.
    LaunchpadLegacyMidi,
    /// Novation Launchpad MK2 in its Session layout, over its one MIDI port:
    /// Novation SysEx (`F0 00 20 29 02 18`) to select the layout (`22 00`),
    /// clear (`0E 00`) and set RGB LEDs at six bits a channel (`0B`); Note On
    /// / Off from the pads and right-hand buttons and Control Change from the
    /// top row in. No logo, keep-alive or brightness command; brightness
    /// scales the colours. Never matched by USB id.
    LaunchpadMk2Midi,
    /// Novation Launchpad Pro (2015, before the MK3) in its Programmer
    /// layout, over its Standalone MIDI port: Novation SysEx
    /// (`F0 00 20 29 02 10`) to select Standalone mode (`21 01`) and the
    /// Programmer layout (`2C 03`), clear (`0E 00`) and set RGB LEDs at six
    /// bits a channel (`0B`, at most 48 a message), the logo as LED 99; Note On
    /// / Off from the pads and Control Change from every round button in. No
    /// keep-alive or brightness command; brightness scales the colours.
    /// Letting go selects the Session layout (`2C 00`). Never matched by USB
    /// id.
    LaunchpadProMidi,
    /// Novation Dicer (a chained pair, one port): Note On / Off on channels
    /// 11-13 (deck-A unit) and 14-16 (deck-B unit), notes `3C`-`40` and a
    /// second layer `41`-`45` for buttons 1-5, all six variants one key; a
    /// light is Note On with an attested colour byte (`7F` / `71` green, `4F`
    /// / `41` orange, `00` off) written to all three of its unit's channels and
    /// both layers; `BA 00 00` resets both units on open and when Ryu lets go.
    /// No brightness command: Ryu picks the dim or bright byte. Never matched
    /// by USB id.
    NovationDicerMidi,
    /// Novation Launch Control XL (MK1 and MK2) on factory template 1, over
    /// its one MIDI port: template select (`F0 00 20 29 02 11 77 tt F7`, both
    /// ways), Note On / Off from the track and side buttons, Control Change
    /// from the arrows and the absolute knobs and faders; bi-colour button and
    /// knob LEDs (red and green at four levels) by Note On, the single-colour
    /// ones on or off. `Bn 00 00` resets. No keep-alive; brightness scales the
    /// levels. Never matched by USB id.
    NovationLaunchControlXl,
    /// Novation Launch Control (MK1) on factory template 1: as the XL with
    /// device byte `0A`, eight pads, sixteen absolute knobs without LEDs and
    /// four arrows. Never matched by USB id.
    NovationLaunchControl,
    /// Novation Launch Control XL 3 in DAW mode, over its second (`DAW`)
    /// MIDI interface, never the first (custom modes) or the DIN output:
    /// `F0 00 20 29 02 15 02 7F F7` turns DAW mode on and `B6 45` / `48` /
    /// `49 7F` make the encoder rows relative. Buttons are Control Change on
    /// channel 1 (Shift `B6 3F`), encoders (relative) and faders (absolute)
    /// Control Change on channel 16; the deck reports its surface mode on
    /// `B6 1E`. Every button and encoder LED takes exact RGB by SysEx (`01 53
    /// <cc> r g b`), and the 128 x 64 OLED a 7-pixels-a-byte bitmap (`09
    /// 20`). No keep-alive or volatile brightness command; `02 00` on drop.
    /// Never matched by USB id.
    NovationLaunchControlXl3,
    /// Novation Launch Control 3: as the XL 3 with device byte `16`, eight
    /// buttons, Shift and two relative encoder rows (no faders). Never
    /// matched by USB id.
    NovationLaunchControl3,
    /// Novation Launchkey MK3 (Mini, 25, 37, 49, 61) in DAW mode, over its
    /// second (`DAW`) MIDI interface, never the first (the piano keys): Note
    /// On `9F 0C 7F` turns DAW mode on, `BF 03 02` / `BF 09 01` / `BF 0A 01`
    /// pick the session pad layout and the volume knob and fader modes; pads
    /// are notes 96-103 and 112-119 on channel 1, buttons Control Change on
    /// channel 1 or 16, knobs and faders absolute Control Change on channel
    /// 16. Pads and some buttons take an index into a 128-colour palette, the
    /// others off, half or full. No keep-alive or brightness command; `9F 0C
    /// 00` on drop. Never matched by USB id.
    LaunchkeyMk3Midi,
    /// Novation Launchkey MK4 (25, 37, 49, 61 and Mini 25 / 37) in DAW mode,
    /// over its second (DAW) MIDI interface, never the first (the piano
    /// keys): Note On `9F 0C 7F` turns DAW mode on, `B6 1D 02` / `B6 1E 02` /
    /// `B6 1F 01` pick the DAW pad layout, Plugin encoder mode and Volume
    /// fader mode, and `B6 45 7F` relative encoder output. Pads are notes
    /// 0x60-0x67 and 0x70-0x77 on channel 1, buttons Control Change on
    /// channel 1 (Shift `B6 3F`), knobs relative and faders absolute Control
    /// Change on channel 16. Pads take exact RGB by SysEx (`F0 00 20 29 02
    /// <14|13> 01 43 ...`), some buttons a palette index, the others a
    /// brightness on channel 4. No keep-alive or volatile brightness command;
    /// `9F 0C 00` on drop. Never matched by USB id.
    LaunchkeyMk4Midi,
    /// Novation Launchkey MK2 (25, 49, 61) in InControl ("extended") mode,
    /// over its second (InControl) MIDI interface, never the first (the piano
    /// keys): Note On `9F 0C 7F` turns InControl mode on and `9F 0D/0E/0F 7F`
    /// put the knobs, faders and pads in it, sent again when the keyboard
    /// reports leaving. Everything on channel 16: pads notes 96-103 and
    /// 112-119 and the round buttons notes 104 and 120 (a 128-colour palette
    /// index by Note On), the other buttons Control Change (the fader buttons
    /// on or off), knobs and faders absolute Control Change. No keep-alive or
    /// brightness command; `9F 0C 00` on drop. Never matched by USB id.
    LaunchkeyMk2Midi,
    /// Novation's first Launchkey Mini in InControl mode, over its second
    /// (InControl) MIDI interface, never the first (the piano keys): `90 0C
    /// 7F` (Extended mode) and `90 0F 7F` (InControl on), sent again when the
    /// deck reports leaving. Everything on channel 1: pads notes 96-103 and
    /// 112-119 and the round buttons notes 104 and 120 (a red/green colour
    /// byte by Note On, as the first Launchpads), the arrows Control Change
    /// 104-107 (no light), knobs absolute `CC 21-28`. No keep-alive or
    /// brightness command; `90 0C 00` on drop. Never matched by USB id.
    LaunchkeyMiniMidi,
    /// Novation SL MkIII (49 and 61) in InControl view, over its second
    /// (`InControl`) MIDI port, never the first (the piano keys): Note On /
    /// Off from the pads and Control Change from the buttons, the relative
    /// (two's-complement) knobs and the absolute faders, all on channel 16.
    /// Every light by SysEx (`F0 00 20 29 02 0A 01 03 <led> 01 <r> <g> <b>
    /// F7`, 7-bit RGB), the screens set to the Knob layout and given knob
    /// colours and soft button labels as properties. The user enters
    /// InControl view on the deck; no keep-alive or brightness command (Ryu
    /// scales the colours). Never matched by USB id.
    NovationSlMk3Midi,
    /// Akai APC mini MK2 in session pad mode, over its first (`Control`)
    /// MIDI port, never its `Notes` port: Akai SysEx (`F0 47 7F 4F <id> <len
    /// MSB> <len LSB> ... F7`) sets session pad mode (`62`), asks for the
    /// fader positions (`60`, answered by `61`) and sets the pads' 8-bit RGB
    /// (`24`); Note On / Off from the pads and buttons and absolute Control
    /// Change from the faders on channel 1; the scene and track LEDs on or
    /// off by Note On. No keep-alive; brightness scales the colours. Never
    /// matched by USB id.
    AkaiApcMiniMk2Midi,
    /// The first Akai APC mini (MK1), over its one MIDI port: Note On / Off
    /// from the pads and buttons and absolute Control Change from the faders
    /// on channel 1; every light is a Note On whose velocity is its state
    /// (pads off, green, red or yellow; scene and track buttons on or off).
    /// No start-up message, keep-alive or brightness command. Never matched
    /// by USB id.
    AkaiApcMiniMidi,
    /// Akai MIDImix, over its one MIDI port: Note On / Off from the Mute,
    /// Solo (Mute with SOLO held), Rec Arm, bank and SOLO buttons and
    /// absolute Control Change from the knobs and faders on channel 1; each
    /// light is a Note On, 127 on or 0 off. No start-up message, keep-alive
    /// or brightness command. Never matched by USB id.
    AkaiMidimixMidi,
    /// Akai LPD8 (MK1), over its one MIDI port, in program 1's factory PAD
    /// mode: Note On / Off from the pads (notes 36-43) and absolute Control
    /// Change from the knobs (1-8) on channel 1; each pad light is a Note On,
    /// 1 on or 0 off, re-sent every 300 ms because the deck drops them when
    /// its program or mode is switched. No start-up message or brightness
    /// command. Never matched by USB id.
    AkaiLpd8Midi,
    /// Akai Fire, over its one MIDI port: Note On / Off from the pads (notes
    /// 54-117) and buttons, relative two's-complement Control Change from the
    /// knobs and the Select encoder; pad colours in Akai SysEx `65`, button
    /// lights as Control Change from each button's fixed colours, and the
    /// 128 x 64 OLED in eight SysEx `0E` bands, sent again every couple of
    /// seconds so it does not sleep. No start-up message or brightness
    /// command (brightness is applied by Ryu). Never matched by USB id.
    AkaiFireMidi,
    /// Akai MPK mini IV and MPK mini Plus II, over their `DAW Port` (never
    /// the keybed's `MIDI Port`): the universal identity request, then (on
    /// the reply, or after a short wait) Akai SysEx `F0 47 00 <pid> ...`
    /// taking the screen (`1C`, sent again every 5.8 s), setting pad mode 1,
    /// asking for the DAW/plugin mode and moving pad notes off the keybed
    /// port, and three fixed text lines. Note On / Off from the pads (notes
    /// 0x24-0x33, channel 10); Control Change on channel 1 from the buttons,
    /// the two's-complement knobs and the one-click main encoder. Pad lights
    /// by palette index and button lights on or off, by Note On whose
    /// channel is the brightness state. Input is ignored in plugin mode.
    /// Never matched by USB id.
    AkaiMpkMk4Midi,
    /// Akai MPK mini Plus (the first generation), over its one MIDI port:
    /// only the five transport buttons, Control Change 0x73-0x77 on channel
    /// 1 (127 pressed, 0 released); each light is a Note On to the button's
    /// controller number, 127 on or 0 off, re-sent every 300 ms. No start-up
    /// message or brightness command; the pads, knobs and keybed are left to
    /// other MIDI software. Never matched by USB id.
    AkaiMpkMiniPlusMidi,
    /// Akai APC40 MKII in Ableton Live mode, over its one MIDI port: the
    /// universal identity request, whose Akai reply gives the device id, then
    /// Akai's introduction (`F0 47 <id> 29 60 00 04 41 ...`) selecting Ableton
    /// mode; Note On / Off from the buttons by channel and note, Control
    /// Change from the knobs (absolute, re-centred by writing 64 back),
    /// Tempo and Cue (two's complement) and faders (absolute); the 45 RGB
    /// pads by palette index, other lights on or off by Note On. No
    /// keep-alive or brightness command. Never matched by USB id.
    AkaiApc40Mk2Midi,
    /// The first Akai APC40 (MK1) in Ableton Live mode, over its one MIDI
    /// port: the universal identity request and Akai's introduction (`F0 47
    /// 7F 73 60 00 04 41 ...`) sent to every device id at once, again with
    /// the device id an identity reply gives; Note On / Off from the buttons
    /// by channel and note (the clip matrix is note `53 + scene` on channel
    /// `track`), Control Change from the knobs (absolute per channel,
    /// re-centred by writing 64 back), Cue (two's complement) and faders
    /// (absolute); the 40 clip pads off, green, red or amber and every other
    /// light on or off by Note On. No keep-alive or brightness command. Never
    /// matched by USB id.
    AkaiApc40Mk1Midi,
    /// The Akai APC20 in Ableton Live mode, the same way as the first APC40
    /// with model byte `7B`: no knobs or crossfader, note 81 is Shift, and the
    /// channel faders may send any of `CC 0-7` as the deck's slider mode picks.
    /// Never matched by USB id.
    AkaiApc20Midi,
    /// The first Akai APC Key 25 (MK1), over its one MIDI port: Note On / Off
    /// from the buttons and absolute Control Change from the knobs on channel
    /// 1 (the piano keys, on another channel, are dropped); the clip pads off,
    /// green, red or amber and the scene and track buttons on or off by Note
    /// On. No start-up message beyond an optional identity request, no
    /// keep-alive or brightness command. Never matched by USB id.
    AkaiApcKey25Midi,
    /// Akai APC Key 25 MK2, over its second (`Control`) MIDI port, never its
    /// `Keys` port: Note On / Off from the buttons and two's-complement
    /// Control Change from the knobs on channel 1; the clip pads by palette
    /// index, at full or half brightness by MIDI channel, the scene and track
    /// buttons on or off. No keep-alive. Never matched by USB id.
    AkaiApcKey25Mk2Midi,
    /// The first Ableton Push (Akai-built), over its User MIDI port (never
    /// the Live port, and never a mode switch): Note On / Off from the pads,
    /// Control Change from the buttons, two's-complement CCs from the
    /// encoders and pitch bend from the strip; the strip set to host-driven
    /// volume and the 4 x 68 character display written by SysEx; the pads and
    /// lower button row by palette index, the upper row and scene column as
    /// red/amber/yellow/green and the other buttons at off/dim/full, all on
    /// channel 1. No keep-alive or brightness command. Never matched by USB
    /// id.
    AbletonPush1Midi,
    /// Ableton Push 2, over its User MIDI port (never the Live port): User
    /// mode set by SysEx (`F0 00 21 1D 01 01 0A 01 F7`), the palette read
    /// back (never written), the touch strip set to pitch bend without
    /// return; Note On / Off from the pads, Control Change from the buttons,
    /// two's-complement CCs from the encoders and pitch bend from the strip;
    /// every light by palette index on channel 1, the global LED brightness
    /// by SysEx. No keep-alive. Never matched by USB id.
    AbletonPush2Midi,
    /// Behringer X-Touch Mini in Mackie Control (MC) mode, selected at open
    /// by `B0 7F 01` (and `B0 7F 00` back to Standard mode on letting go),
    /// over its one MIDI port: Note On from the buttons and encoder pushes
    /// (velocity 0 releases), sign-magnitude relative Control Change from
    /// the encoders and pitch bend from the fader; each button LED off or on
    /// and each encoder ring off or full by Note On / Control Change on
    /// channel 1. No keep-alive or brightness command. Never matched by USB
    /// id.
    BehringerXTouchMiniMidi,
    /// Behringer X-Touch One in its standalone MIDI CC (or MIDI note) mode,
    /// which the user selects on the deck, over its one MIDI port: Control
    /// Change (or Note On) from the 35 buttons and the fader touch on their
    /// own numbers, relative CC from the encoder and jog wheel and absolute CC
    /// from the motor fader; each button LED off or on by Control Change, and
    /// the LCD's backlight colour and text and the 7-segment display by
    /// Behringer SysEx, all on channel 1. No keep-alive or brightness command.
    /// Never matched by USB id.
    BehringerXTouchOneMidi,
    /// Behringer B-Control BCR2000 and BCF2000, over their first MIDI port:
    /// Ryu writes its own BCL text preset into the edit buffer at open (one
    /// Behringer SysEx per line, paced from the poll loop; never stored),
    /// then reads absolute Control Change on channel 1 from the encoders
    /// (turned into steps and put back to 64 when still) and faders, and
    /// 127 / 0 from the momentary buttons; each button LED on or off by the
    /// same Control Change. No brightness command or keep-alive. Never
    /// matched by USB id: both report the same product id.
    BehringerBControlMidi,
    /// KORG nanoKONTROL2 in KORG Native Mode, entered at open by
    /// `F0 42 4g 00 01 13 00 00 00 01 F7` (on the Global channel `g` its
    /// Search Device reply gives) and left on letting go: every control as
    /// Control Change on channel 16 (absolute knobs and sliders, buttons 7F
    /// and 00), and each button LED off or on by the same Control Change.
    /// No brightness command; every LED is resent every 30 seconds. Never
    /// matched by USB id.
    KorgNanoKontrol2,
    /// KORG nanoKONTROL Studio over USB in KORG Native Mode, entered by
    /// `F0 42 4g 00 01 37 02 00 00 00 01 F7`: like the nanoKONTROL2, plus
    /// the Select buttons, Return to Zero, a relative jog wheel (`CC 6E`) and
    /// the Scene button as a SysEx parameter change. Each LED by Control
    /// Change on channel 16 (inferred from the nanoKONTROL2: KORG documents
    /// the Studio's Native Mode outputs only). No brightness command. Never
    /// matched by USB id.
    KorgNanoKontrolStudio,
    /// KORG padKONTROL in KORG Native Mode, over its second MIDI port (PORT
    /// A in, CTRL out): a Universal Device Inquiry for the Global channel
    /// `g` (half a second's non-blocking wait, then `g = 0`), then on
    /// `F0 42 4g 6E 08` Native Mode in (`00 00 01`), packet 1 with every
    /// normal-MIDI transmit off and packet 2 with every LED off. Every
    /// control reports as a 9-byte SysEx event (pads, switches, encoder,
    /// knobs, X-Y pad, pedal); each LED on or off by `01 nn 20/00`. No
    /// brightness command; every LED is resent every 30 seconds; LEDs off
    /// and `00 00 00` on drop. Never matched by USB id.
    KorgPadKontrol,
    /// Arturia BeatStep, over its one MIDI port: about a second after open,
    /// Ryu's own mapping written to the deck's working memory as 144
    /// `F0 00 20 6B 7F 42 02 00 <property> <control> <value> F7` writes a
    /// millisecond apart (never stored to a preset); then Note On / Off from
    /// the pads and Shift on channel 10, Control Change from Stop, Play and
    /// the relative (two's-complement) encoders on channel 1. Each pad's red
    /// and blue LEDs by the same SysEx (property 0x10). No keep-alive or
    /// brightness command. Never matched by USB id.
    ArturiaBeatStepMidi,
    /// Arturia MiniLab 3 in its DAW program, over its main MIDI port: on open
    /// the DAW connection SysEx (`F0 00 20 6B 7F 42 02 00 40 6A 21 F7`), a
    /// program request, a fixed OLED message, the eight encoders realigned to
    /// 64 and every light, queued and paced from poll and flush. Note On / Off
    /// from the pads on channel 10; Control Change on channel 1 from Shift,
    /// the transport keys, the absolute encoders and faders and the relative
    /// main encoder. Each pad's and transport key's RGB light (7 bits a
    /// channel) by SysEx. Input is ignored while the deck reports another
    /// program. No keep-alive or brightness command (Ryu scales the colours).
    /// Never matched by USB id.
    ArturiaMiniLab3Midi,
    /// Arturia KeyLab Essential mk3 (49, 61 or 88 keys) in its DAW program,
    /// over its main MIDI port: on open the DAW connection SysEx (`F0 00 20 6B
    /// 7F 42 02 0F 40 5A 01 F7`), a program request, a fixed two-line text
    /// message, the nine knobs realigned to 64 and every light, queued and
    /// paced from poll and flush. Note On / Off from the pads on channel 11;
    /// Control Change on channel 1 from the buttons, the absolute knobs and
    /// faders and the relative display encoder. Each pad's and button's RGB
    /// light (7 bits a channel) by SysEx. Input is ignored while the deck
    /// reports another program. No keep-alive or brightness command (Ryu
    /// scales the colours). Never matched by USB id.
    ArturiaKeyLabEssential3Midi,
    /// DJ TechTools Midi Fighter Twister on its factory configuration, over
    /// its one MIDI port: knobs as absolute values (`B0`, channel 1) Ryu turns
    /// into steps and puts back to 64 once still, pushes (`B1`), and four side
    /// buttons and the bank changes on channel 4 (`B3`). Each knob's light is
    /// a palette index (`B1`), dimmed by the RGB and ring animations (`B2`,
    /// `B5`). No keep-alive. Never matched by USB id.
    MidiFighterTwisterMidi,
    /// DJ TechTools Midi Fighter Spectra on its factory configuration, over
    /// its one MIDI port: the 16 arcade buttons as Note On / Off on channel 3
    /// (`92` / `82`), notes 36-99 in four firmware banks Ryu folds. Each
    /// button's light is a palette index (`92`) written in all four banks,
    /// and the four white ground-effect lights one level (`93 10..13`). No
    /// brightness command (Ryu picks the dim palette index under 50%), no
    /// keep-alive. Never matched by USB id.
    MidiFighterSpectraMidi,
    /// DJ TechTools Midi Fighter 64 on its factory configuration, over its
    /// one MIDI port: the 64 arcade buttons as Note On / Off on channel 3
    /// (`92` / `82`, the upper-left bank) or channel 2 (`91` / `81`, the
    /// upper-right bank), notes 36-99 in two bottom-up 4-wide halves, folded.
    /// Each button's light is a Launchpad palette index (`92`) and a steady
    /// brightness mode 18-33 (`93 <note - 36>`). No SysEx, no keep-alive.
    /// Never matched by USB id.
    MidiFighter64Midi,
    /// Intech Studio Grid modules on their factory default profile, over the
    /// one MIDI port every module type shares: Note On / Off from buttons and
    /// pushes and absolute Control Change from encoders, potentiometers and
    /// faders, numbers 32-47 on channels 1-4 (the module's pages). Input
    /// only: nothing is sent, and there is no brightness command. Never
    /// matched by USB id.
    IntechGridMidi,
    /// PreSonus ATOM in native mode (`8F 00 7F` on, `8F 00 00` off), over its
    /// one MIDI port, on channel 1: Note On / Off from the pads (notes 36-51,
    /// bottom row first), Control Change from the buttons (127 / 0) and
    /// sign-and-size relative Control Change 14-17 from the encoders. Each
    /// pad's light is red, green and blue Note Ons on channels 2-4 then a
    /// channel 1 Note On, 127 lit or 0 off; button lights are their Control
    /// Change, 127 on or 0 off. No brightness command (Ryu scales the pads).
    /// Never matched by USB id.
    PresonusAtomMidi,
    /// PreSonus ATOM SQ in native mode (`8F 00 01` on, `8F 00 00` off, both
    /// firmware toggles off by SysEx), over its one MIDI port: on channel 1,
    /// Note On / Off from the pads (upper notes 52-67, lower 36-51) and Plus
    /// and Minus (notes 0, 1), Control Change from the other buttons and
    /// signed-bit relative Control Change 14-21 and 29 from the encoders; the
    /// touch strip is a 14-bit pitch bend on channel 16. RGB lights are red,
    /// green and blue on channels 2-4 then 127 or 0 on channel 1, other button
    /// lights their Control Change 127 / 0; the text display takes one SysEx
    /// per line. No brightness command (Ryu scales the colours). Never matched
    /// by USB id.
    PresonusAtomSqMidi,
    /// PreSonus' original FaderPort (2007) in native mode, entered by
    /// `91 00 64` once it answers the Device Inquiry, over its one MIDI port:
    /// buttons as Polyphonic Aftertouch (ids 0-23, footswitch 126, fader touch
    /// 127), the encoder a pitch bend per detent, the fader `CC 0` / `CC 32`.
    /// LEDs are `A0 <led> <01 / 00>` with their own numbers. No brightness
    /// command. Never matched by USB id.
    PresonusFaderPortMidi,
    /// PreSonus' FaderPort 2, 8 and 16 in native (Studio One) mode, chosen on
    /// the deck: buttons and fader touches as notes (pressed above velocity
    /// 64), faders as a 14-bit pitch bend per strip, the two encoders as
    /// signed-bit relative `CC 16` / `CC 60`. Lights are `90 <note> 7F / 00`,
    /// RGB buttons' colours `91 / 92 / 93 <note> <7-bit level>`; writes are
    /// paced from the poll loop. The FaderPort 16 takes `A0 00 00` each
    /// second. No brightness command. Never matched by USB id.
    PresonusFaderPort8Midi,
    /// Faderfox EC4 on firmware 2.0, over its one MIDI port, reading the
    /// setup the user loaded: factory setups 1-14 (absolute Control Change
    /// from the encoders, which Ryu turns into steps and puts back to 64 by
    /// feedback, and Note On / Off from their pushes, on the setup's channel)
    /// or DrivenByMoss's template (relative CC and CC pushes on channel 16);
    /// Shift, User 1-4 and the setup and group by Faderfox's fixed SysEx,
    /// asked for at open. Each key's label goes to its encoder's
    /// four-character name on the OLED by the same SysEx, paced from the poll
    /// loop. No lights, brightness command or keep-alive. Never matched by
    /// USB id.
    FaderfoxEc4Midi,
    /// Livid Instruments Base and Base II, over their first MIDI port: the
    /// touch buttons, pads and function buttons as Note On / Off and the nine
    /// touch faders as absolute `CC 1-9`, all on channel 1. At open the pads
    /// are set to Live output (`F0 00 01 61 <product> 42`, 32 x `05`) and
    /// local control turned off (`BF 7A 40`); each light is a palette
    /// velocity by Note On. No brightness command or keep-alive. Never
    /// matched by USB id.
    LividBaseMidi,
    /// Allen & Heath Xone:K2, over its one MIDI port on the channel it was
    /// set to at power-on (taken from its first message; 15 until then):
    /// Note On / Off from the buttons and encoder pushes, relative Control
    /// Change from the four top and two bottom encoders and absolute from
    /// the twelve knobs and four faders (never written: not motorised).
    /// Each button's tri-colour LED is a Note On at its note plus 0, 36 or
    /// 72 (Layer: notes `0C`, `10`, `14`), off by Note Off. No handshake,
    /// keep-alive or brightness command. Never matched by USB id.
    AllenHeathXoneK2Midi,
    /// Intuitive Instruments Exquis in its Developer Mode, over its one MIDI
    /// port: SysEx `F0 00 21 7E 7F <cmd> .. F7`. Setup (`cmd 00`, zone mask
    /// `2F`) takes over everything but Settings and Sound; pads arrive as
    /// Note On / Off and buttons, encoders and the slider as Control Change,
    /// all on channel 16. Each LED takes a 7-bit RGB colour by `cmd 04` in
    /// contiguous runs; brightness is scaled by Ryu. A Refresh from the
    /// device (leaving its menu) repaints. No keep-alive. Never matched by
    /// USB id.
    ExquisMidi,
    /// M-Audio Oxygen Pro 25, 49, 61 and Mini and Hammer 88 Pro in DAW mode,
    /// over their third (`Mackie/HUI`) MIDI port: the universal identity
    /// request and M-Audio SysEx `F0 00 01 05 7F 00 00 <cmd> 00 01 <arg> F7`
    /// selecting DAW mode and software LED control, sent at once. Pads as
    /// Note On / Off on channel 1, buttons as Control Change on channels 1,
    /// 13 (Shift) and 16 (mode buttons; the track buttons latch, so each
    /// message is a tap), absolute knobs and faders and a relative encoder.
    /// Pad lights by a 64-colour palette index, track-button lights on or
    /// off; brightness is scaled by Ryu. LED mode back to firmware on close.
    /// Never matched by USB id.
    MAudioOxygenProMidi,
    /// iCON Platform M+ and Platform M in their Mackie Control mode, over
    /// their one MIDI port: Note On from the buttons, fader touches and
    /// V-Pot pushes, signed-bit relative Control Change from the V-Pots and
    /// jog wheel and pitch bend from the nine motor faders (never sent back);
    /// each button LED off or on by Note On, paced from the poll loop, all on
    /// channel 1. No handshake, keep-alive, display or brightness command.
    /// Never matched by USB id.
    IconPlatformMMidi,
    /// iCON Platform X+ extender in its Mackie Control mode, over its one
    /// MIDI port: Note On from the strip buttons, V-Pot pushes and fader
    /// touches, signed-bit relative Control Change from the V-Pots and pitch
    /// bend from the eight motor faders (never sent back); each strip
    /// button's LED off or on by Note On, paced from the poll loop, all on
    /// channel 1. No handshake, keep-alive, display or brightness command.
    /// Never matched by USB id.
    IconPlatformXPlusMidi,
    /// iCON QCon Pro X and its QCon Pro XS1 extender in their Mackie Control
    /// mode, each over its own MIDI port: Note On from the buttons, fader
    /// touches and V-Pot pushes, sign-magnitude relative Control Change from
    /// the V-Pots and jog wheel and pitch bend from the motor faders (never
    /// sent back); each button LED off or on by Note On and the main LCD's
    /// two lines by Mackie SysEx, paced from the poll loop, all on channel 1.
    /// No handshake, keep-alive or brightness command. Never matched by USB
    /// id.
    IconQconProMidi,
    /// iCON QCon Pro G2 and its QCon EX G2 extender in their Mackie Control
    /// mode, each over its own MIDI port: as [`Self::IconQconProMidi`], plus
    /// the V-Pot rings' centre dot on or off by Control Change and a
    /// wake-up SysEx at open; the EX G2's LCD is written under both Mackie
    /// extender ids until the deck names its own. No second LCD, master
    /// meter, handshake, keep-alive or brightness command. Never matched by
    /// USB id.
    IconQconG2Midi,
    /// iCON P1-Nano in its Mackie Control mode, over its first MIDI port:
    /// Note On from the buttons, fader touches and V-Pot pushes on channel 1
    /// (other channels are its D-5 touchscreen's, ignored), sign-magnitude
    /// relative Control Change from the V-Pots and jog wheel and pitch bend
    /// from its one motor fader on any of channels 1-9 (never sent back);
    /// each button LED off or on by Note On, the LCD's two lines by Mackie
    /// SysEx and the eight scribble strips' RGB backlights (the V-Pots' dial
    /// lights) by one iCON SysEx, paced from the poll loop. No handshake,
    /// keep-alive or brightness command. Never matched by USB id.
    IconP1Midi,
    /// Zoom R16 and R24 recorders in their audio interface mode, as a Mackie
    /// Control surface over their one MIDI port: Note On from the keys,
    /// pitch bend from the eight track faders and the master (no motors;
    /// never sent back) and sign-magnitude relative Control Change from the
    /// jog dial; the track status lights (unconfirmed) on or off by Note On,
    /// all on channel 1. No handshake, SysEx, keep-alive, display or
    /// brightness command. Never matched by USB id.
    ZoomR16Midi,
    /// Mackie MCU Pro in Mackie Control (or Logic Control) mode, over the
    /// first of its four MIDI ports: silent but for Mackie's device query
    /// until the deck answers (Logic Control's challenge answered from the
    /// poll loop); then Note On from the buttons and touches, sign-magnitude
    /// relative Control Change from the V-Pots and jog wheel and pitch bend
    /// from the motor faders (never sent back); button LEDs and V-Pot rings
    /// on or off and a fixed LCD text, paced from the poll loop, all on
    /// channel 1. No keep-alive or brightness command. Never matched by USB
    /// id.
    MackieMcuProMidi,
    /// Mackie XT Pro extender, reached through one of an MCU Pro's three
    /// extra MIDI ports: as [`Self::MackieMcuProMidi`] without the jog
    /// wheel, master fader, global buttons or 7-segment digits, and silent
    /// but for the device query until the deck names itself by SysEx (the
    /// port may carry any MIDI gear).
    MackieXtProMidi,
    /// The MacBook Pro Touch Bar, through AppKit on the main thread (macOS only).
    /// Never matched by USB id: the device loop adds it when the Mac has one.
    TouchBar,
}

impl Protocol {
    /// Reached as a USB serial port rather than a HID device.
    #[must_use]
    pub const fn is_serial(self) -> bool {
        matches!(
            self,
            Self::Loupedeck
                | Self::TourBoxSerial
                | Self::YamahaCc1Serial
                | Self::DjiDuml
                | Self::MonomeMext
                | Self::MonomeSeries
                | Self::Monome40h
        )
    }

    /// Reached as a pair of USB MIDI ports, found by name ([`identify_midi`]).
    #[must_use]
    pub const fn is_midi(self) -> bool {
        matches!(
            self,
            Self::NovationLaunchpadMidi
                | Self::LaunchpadLegacyMidi
                | Self::LaunchpadMk2Midi
                | Self::LaunchpadProMidi
                | Self::NovationDicerMidi
                | Self::NovationLaunchControlXl
                | Self::NovationLaunchControl
                | Self::NovationLaunchControlXl3
                | Self::NovationLaunchControl3
                | Self::LaunchkeyMk3Midi
                | Self::LaunchkeyMk4Midi
                | Self::LaunchkeyMk2Midi
                | Self::LaunchkeyMiniMidi
                | Self::NovationSlMk3Midi
                | Self::AkaiApcMiniMk2Midi
                | Self::AkaiApcMiniMidi
                | Self::AkaiMidimixMidi
                | Self::AkaiLpd8Midi
                | Self::AkaiFireMidi
                | Self::AkaiMpkMk4Midi
                | Self::AkaiMpkMiniPlusMidi
                | Self::AkaiApc40Mk2Midi
                | Self::AkaiApc40Mk1Midi
                | Self::AkaiApc20Midi
                | Self::AkaiApcKey25Midi
                | Self::AkaiApcKey25Mk2Midi
                | Self::AbletonPush1Midi
                | Self::AbletonPush2Midi
                | Self::BehringerXTouchMiniMidi
                | Self::BehringerXTouchOneMidi
                | Self::BehringerBControlMidi
                | Self::KorgNanoKontrol2
                | Self::KorgNanoKontrolStudio
                | Self::KorgPadKontrol
                | Self::ArturiaBeatStepMidi
                | Self::ArturiaMiniLab3Midi
                | Self::ArturiaKeyLabEssential3Midi
                | Self::MidiFighterTwisterMidi
                | Self::MidiFighterSpectraMidi
                | Self::MidiFighter64Midi
                | Self::IntechGridMidi
                | Self::PresonusAtomMidi
                | Self::PresonusAtomSqMidi
                | Self::PresonusFaderPortMidi
                | Self::PresonusFaderPort8Midi
                | Self::FaderfoxEc4Midi
                | Self::LividBaseMidi
                | Self::AllenHeathXoneK2Midi
                | Self::ExquisMidi
                | Self::MAudioOxygenProMidi
                | Self::IconPlatformMMidi
                | Self::IconPlatformXPlusMidi
                | Self::IconQconProMidi
                | Self::IconQconG2Midi
                | Self::IconP1Midi
                | Self::ZoomR16Midi
                | Self::MackieMcuProMidi
                | Self::MackieXtProMidi
        )
    }

    /// A serial deck that takes a DTR drop as the host leaving and then stays
    /// silent until it is replugged (the Yamaha CC121MK2): its port is opened
    /// with DTR and RTS up and closed without dropping DTR.
    #[must_use]
    pub const fn holds_dtr(self) -> bool {
        matches!(self, Self::YamahaCc1Serial)
    }

    /// The USB serial number the host reports is one it made up for the port
    /// (the CC121MK2 has none), so it changes with the socket and is not used.
    #[must_use]
    pub const fn serial_is_invented(self) -> bool {
        matches!(self, Self::YamahaCc1Serial)
    }

    /// The baud rate a serial deck's port is opened at. USB serial mostly
    /// ignores it, but each device documents its own.
    #[must_use]
    pub const fn serial_baud(self) -> u32 {
        match self {
            Self::TourBoxSerial
            | Self::YamahaCc1Serial
            | Self::DjiDuml
            | Self::MonomeMext
            | Self::MonomeSeries
            | Self::Monome40h => 115_200,
            _ => 256_000,
        }
    }

    /// Whether a serial port on these USB ids must answer the protocol's probe
    /// before it is claimed: the id is a generic one other boards also use, or
    /// (a DJI remote) the device lists a second, silent port on the same id,
    /// or (a mext monome) the answer says which grid or arc it is.
    #[must_use]
    pub fn needs_serial_probe(self, vendor_id: u16, product_id: u16) -> bool {
        matches!(self, Self::DjiDuml | Self::MonomeMext)
            || matches!(self, Self::TourBoxSerial)
                && [TOURBOX_NEO_USB_ID, TOURBOX_TINYUSB_USB_ID].contains(&(vendor_id, product_id))
    }

    /// A port that did not answer the probe is asked again later, unless a
    /// sibling port with the same USB serial answered: a DJI remote can be
    /// listed while it is switched off or still letting go of another program,
    /// and a monome may still be held by serialosc.
    #[must_use]
    pub const fn reprobes_silent_ports(self) -> bool {
        matches!(self, Self::DjiDuml | Self::MonomeMext)
    }

    /// Reached on Linux through an input event node rather than hidraw: the
    /// kernel keeps the device on its HID ignore list (the Griffin PowerMate)
    /// and drives it with an input driver of its own.
    #[must_use]
    pub const fn linux_evdev(self) -> bool {
        matches!(self, Self::GriffinPowerMate)
    }

    /// Driven only through a vendor-defined collection (usage page 0xff00 and up),
    /// whatever its exact page.
    #[must_use]
    pub const fn needs_vendor_collection(self) -> bool {
        matches!(
            self,
            Self::LogitechHidpp
                | Self::PrehPosKey
                | Self::LogitechG15
                | Self::LogitechPerKeyLighting2
                | Self::LogitechPerKeyLighting
        )
    }

    /// Asks the keyboard which model it is before it is opened as a deck,
    /// since several models share a product id (the PrehKeyTec keyboards).
    #[must_use]
    pub const fn asks_identity(self) -> bool {
        matches!(self, Self::PrehPosKey)
    }

    /// Driven through two HID interfaces of one device at once: the deck is found
    /// by its control interface, and the display interface is opened beside it.
    #[must_use]
    pub const fn paired_interfaces(self) -> Option<PairedInterfaces> {
        match self {
            Self::MountainDisplayPad => Some(PairedInterfaces {
                control: 3,
                display: 1,
            }),
            _ => None,
        }
    }

    /// Driven through two HID collections of one device told apart by usage
    /// page, not by interface number: the deck is found by its control
    /// collection, and the display (lighting) collection is opened beside it.
    #[must_use]
    pub const fn paired_collections(self) -> Option<PairedCollections> {
        match self {
            Self::WootingAnalog | Self::WootingLegacy => Some(PairedCollections {
                control: &wooting::ANALOG_USAGE_PAGES,
                display: &wooting::LIGHT_USAGE_PAGES,
            }),
            _ => None,
        }
    }

    /// Whether a HID collection on this usage page can be the deck. A Wooting
    /// keyboard is claimed only by its analog collection; its keyboard,
    /// consumer and lighting collections are not decks of their own, and
    /// Atmel's vendor id is shared with boards that have none of them.
    #[must_use]
    pub fn claims_usage_page(self, usage_page: u16) -> bool {
        match self.paired_collections() {
            Some(pair) => pair.control.contains(&usage_page),
            None => true,
        }
    }

    /// The one HID interface (and, where the host reports one, the HID usage)
    /// a deck is driven through, on a device that exposes others on the same
    /// ids: the Galleon 100 SD is a whole keyboard.
    #[must_use]
    pub const fn hid_interface(self) -> Option<HidInterface> {
        match self {
            Self::ElgatoGalleon => Some(HidInterface {
                number: 0,
                usage: 0x01,
            }),
            // The F1's HID collection; it is matched by interface, not by its
            // vendor usage page.
            Self::TraktorF1Hid => Some(HidInterface {
                number: 1,
                usage: 0x01,
            }),
            // The Apex keyboards' OLED interface, never the keyboard's own;
            // its usage is not known, so any is taken.
            Self::SteelSeriesOled => Some(HidInterface {
                number: 1,
                usage: ANY_USAGE,
            }),
            Self::SteelSeriesOledChunked => Some(HidInterface {
                number: 3,
                usage: ANY_USAGE,
            }),
            // The G815, G910, G513 and G Pro list HID++ on interface 1 under
            // usage page 0xFF43: usage 0x0602 takes the 20-byte `0x11`
            // reports, and its 64-byte sibling (0x0604) is not a second deck.
            // The wired G915 is on another interface (`DeviceModel::hid_interface`).
            Self::LogitechPerKeyLighting2 | Self::LogitechPerKeyLighting => Some(HidInterface {
                number: 1,
                usage: LOGITECH_HIDPP_LONG_USAGE,
            }),
            _ => None,
        }
    }

    /// Whether a serial number of all zeros means "no serial": the Galleon may
    /// report one, which two keyboards would share.
    #[must_use]
    pub const fn zero_serial_is_none(self) -> bool {
        matches!(self, Self::ElgatoGalleon)
    }

    /// Whether every unit reports the same serial number, which therefore says
    /// nothing (SurfaceCore hard-codes "0002"): such decks go by their path.
    #[must_use]
    pub const fn serial_is_shared(self) -> bool {
        matches!(self, Self::SurfaceCoreHid)
    }

    /// Takes a brightness setting. The DisplayPad has no known command for one,
    /// the Blackmagic panels' lights are only on or off, and an Xbox controller
    /// or a DJI remote has nothing Ryu lights, the Buzz! domes are only on
    /// or off, and the first APC mini's, the APC40 MKII's, the first Push's
    /// and the first APC Key 25's lights have no dimming, nor do the
    /// MIDImix's, the LPD8's, the X-Touch Mini's or One's, the nanoKONTROLs' or padKONTROL's
    /// or the BeatStep's or the B-Controls' or the iCON Platforms', QCons' or
    /// P1-Nano's, and a
    /// Grid module or a Faderfox EC4 takes no light from Ryu at all, and the
    /// Livid Bases' palette lights and the Xone:K2's tri-colour ones have no
    /// dimming.
    #[must_use]
    pub const fn has_brightness(self) -> bool {
        !matches!(
            self,
            Self::MountainDisplayPad
                | Self::BlackmagicPanel
                | Self::XboxGip
                | Self::DjiDuml
                | Self::SonyBuzz
                | Self::AkaiApcMiniMidi
                | Self::AkaiMidimixMidi
                | Self::AkaiLpd8Midi
                | Self::AkaiMpkMiniPlusMidi
                | Self::AkaiApc40Mk2Midi
                | Self::AkaiApc40Mk1Midi
                | Self::AkaiApc20Midi
                | Self::AkaiApcKey25Midi
                | Self::AbletonPush1Midi
                | Self::BehringerXTouchMiniMidi
                | Self::BehringerXTouchOneMidi
                | Self::BehringerBControlMidi
                | Self::KorgNanoKontrol2
                | Self::KorgNanoKontrolStudio
                | Self::KorgPadKontrol
                | Self::ArturiaBeatStepMidi
                | Self::IntechGridMidi
                | Self::PresonusFaderPortMidi
                | Self::PresonusFaderPort8Midi
                | Self::FaderfoxEc4Midi
                | Self::LividBaseMidi
                | Self::AllenHeathXoneK2Midi
                | Self::IconPlatformMMidi
                | Self::IconPlatformXPlusMidi
                | Self::IconQconProMidi
                | Self::IconQconG2Midi
                | Self::IconP1Midi
                | Self::ZoomR16Midi
                | Self::MackieMcuProMidi
                | Self::MackieXtProMidi
        )
    }

    /// Whether a HID collection, by its usage page and usage, is the one to
    /// drive on a device that lists several on the same ids. An Xbox
    /// controller is driven through its Generic Desktop gamepad, joystick or
    /// multi-axis collection; a collection whose page or usage the host does
    /// not report (zero) is taken too.
    #[must_use]
    pub const fn accepts_collection(self, usage_page: u16, usage: u16) -> bool {
        match self {
            Self::XboxGip => {
                usage_page == 0
                    || (usage_page == GENERIC_DESKTOP_USAGE_PAGE
                        && matches!(usage, 0 | 0x04 | 0x05 | 0x08))
            }
            Self::PlayStationDualSense | Self::PlayStationDualShock4 | Self::PlayStationAccess => {
                usage_page == 0
                    || (usage_page == GENERIC_DESKTOP_USAGE_PAGE && matches!(usage, 0 | 0x05))
            }
            // The firmware's collections are all Consumer Control (0x01).
            Self::AdafruitMacroPadHid => matches!(usage, 0 | 0x01),
            // The configuration collection only (usage 0x40).
            Self::GameSirConfig => usage_page == GAMESIR_USAGE_PAGE && matches!(usage, 0 | 0x40),
            _ => true,
        }
    }

    /// Whether this is one of the Sony PlayStation controllers' dialects.
    #[must_use]
    pub const fn is_playstation(self) -> bool {
        matches!(
            self,
            Self::PlayStationDualSense | Self::PlayStationDualShock4 | Self::PlayStationAccess
        )
    }

    /// Opened exclusively where the host allows it (macOS), so games and the
    /// system stop acting on the presses Ryu takes; shared when that is refused.
    #[must_use]
    pub const fn opens_exclusive_first(self) -> bool {
        matches!(self, Self::XboxGip) || self.is_playstation()
    }

    /// Driven over USB only. The Blackmagic panels also pair over Bluetooth with
    /// the same ids, which Ryu has not been checked against. The PlayStation
    /// controllers send a reduced report over Bluetooth until asked for more,
    /// and take only checksummed output there.
    #[must_use]
    pub const fn wired_only(self) -> bool {
        matches!(self, Self::BlackmagicPanel | Self::GameSirConfig) || self.is_playstation()
    }

    /// Takes a brightness for its dial or deck lights even with no key screens
    /// (the MasterHub knob, slider and base modules, a Vial board's underglow,
    /// a monome arc's rings, a PlayStation controller's light bar, a GameSir
    /// controller's light zones, the PowerMate's base light, the G513 and G Pro
    /// keyboards' backlight, the Midi Fighter Twister's knob lights and rings).
    #[must_use]
    pub const fn dims_its_lights(self) -> bool {
        matches!(
            self,
            Self::MasterHub
                | Self::VialRawHid
                | Self::MonomeMext
                | Self::GameSirConfig
                | Self::GriffinPowerMate
                | Self::LogitechPerKeyLighting2
                | Self::LogitechPerKeyLighting
                | Self::MidiFighterTwisterMidi
        ) || self.is_playstation()
    }

    /// Shows key labels as text on a display of its own, with no key screens
    /// (the Quick Keys' OLED).
    #[must_use]
    pub const fn shows_text_labels(self) -> bool {
        matches!(self, Self::XencelabsQuickKeys)
    }

    /// How long to wait before each further attempt to open a deck that would
    /// not open: the Quick Keys is not always ready right after it appears.
    #[must_use]
    pub const fn open_retry_delays_ms(self) -> &'static [u64] {
        match self {
            Self::XencelabsQuickKeys => &[50, 500],
            _ => &[],
        }
    }

    /// Every path of the deck's collection is kept, not just the first, and
    /// the right ones are picked when it opens: Windows lists the Mystrix's two
    /// vendor collections, and the Adafruit MacroPad's three, as separate paths
    /// on one usage page.
    #[must_use]
    pub const fn keeps_alternate_paths(self) -> bool {
        matches!(self, Self::MystrixRawHid | Self::AdafruitMacroPadHid)
    }

    /// A report only the right collection accepts. Windows lists the Mystrix's
    /// two vendor collections (report 0xFF and report 0xCB) as separate paths
    /// with the same usage page; each is tried with this write until one takes it.
    #[must_use]
    pub const fn open_probe(self) -> Option<&'static [u8]> {
        match self {
            Self::MystrixRawHid => Some(&mystrix::INQUIRY),
            _ => None,
        }
    }
}

/// A [`HidInterface`] usage that takes whatever usage the host reports.
pub const ANY_USAGE: u16 = 0;

/// The HID++ collection of 20-byte `0x11` reports on the Logitech G815, G910,
/// G513 and G Pro keyboard (usage page 0xFF43).
pub const LOGITECH_HIDPP_LONG_USAGE: u16 = 0x0602;

/// The wired G915's HID++ collection: interface 2, usage page 0xFF00, usage 2.
const LOGITECH_G915_HIDPP: HidInterface = HidInterface {
    number: 2,
    usage: 0x0002,
};

/// The HID interface a deck must be found on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HidInterface {
    pub number: i32,
    /// The HID usage of the deck's collection, checked only when the host
    /// reports a usage (nonzero) and this is not [`ANY_USAGE`].
    pub usage: u16,
}

impl HidInterface {
    /// Whether an enumerated collection is this interface.
    #[must_use]
    pub const fn accepts(self, interface: i32, usage: u16) -> bool {
        interface == self.number && (usage == 0 || self.usage == ANY_USAGE || usage == self.usage)
    }
}

/// The usage pages of a deck driven through two HID collections, each list in
/// order of preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PairedCollections {
    /// The collection the deck is found by and reads its input from.
    pub control: &'static [u16],
    /// The collection its lights are written to.
    pub display: &'static [u16],
}

/// The USB interface numbers of a deck that needs two HID interfaces open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PairedInterfaces {
    /// Input reports and commands.
    pub control: i32,
    /// Write-only key-face pixels.
    pub display: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageMode {
    Bmp,
    Jpeg,
    Png,
    /// Raw 16-bit 5-6-5 pixels, little-endian (Loupedeck's framebuffer).
    Rgb565,
    /// Raw 16-bit 5-6-5 pixels, big-endian (the Loupedeck CT wheel's screen).
    Rgb565Be,
    /// Raw 24-bit pixels in blue, green, red order, rows top to bottom (Infinitton).
    Bgr24,
    /// One bit per pixel, set for a bright pixel, in 8-row bands of one byte
    /// per column with bit 0 at the top (the Logitech G13's LCD).
    MonoColumns,
    /// One bit per pixel, set for a bright pixel, row by row with the most
    /// significant bit of each byte the leftmost pixel (the Maschine MK2's displays).
    MonoRows,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rotation {
    None,
    Quarter,
    /// 270 degrees clockwise (the Stream Deck + XL's panels).
    ThreeQuarter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mirror {
    None,
    Horizontal,
    Both,
}

/// Lights on a deck's dials that Ryu can colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DialLights {
    None,
    /// One colour per dial (Stream Deck Studio rings and knobs, AKP05 knobs).
    PerDial,
    /// One colour for them all (the Stream Dock XL's side lights).
    Shared,
}

impl DialLights {
    /// How many colours a layout holds for a deck with `dials` dials.
    #[must_use]
    pub const fn colors(self, dials: u8) -> u8 {
        match self {
            Self::None => 0,
            Self::PerDial => dials,
            Self::Shared => 1,
        }
    }
}

/// Where a model's strip keys sit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StripPlacement {
    /// One row under the key grid (Stream Deck +, AKP05, the Touch Bar).
    Below,
    /// Two columns either side of the key grid, the first half on the left
    /// (Loupedeck Live and CT), each column top to bottom.
    Sides,
    /// One row over the key grid, under the dials (the Galleon 100 SD's screen).
    Above,
}

/// How one key face must be shaped before it is sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyImage {
    pub width: u32,
    pub height: u32,
    pub mode: ImageMode,
    pub rotation: Rotation,
    pub mirror: Mirror,
}

#[derive(Debug, Clone, Copy)]
pub struct DeviceModel {
    /// Stable id used in device ids, the API and the stored bindings.
    pub id: &'static str,
    pub name: &'static str,
    pub family: Family,
    pub vendor_id: u16,
    pub product_ids: &'static [u16],
    /// The same deck sold under other brands, as `(vendor id, product id)`.
    pub more_usb_ids: &'static [(u16, u16)],
    pub protocol: Protocol,
    pub rows: u8,
    pub cols: u8,
    pub keys: u8,
    pub dials: u8,
    /// Trailing keys with no screen, numbered after the screen keys: the Neo's
    /// two touch buttons, a Loupedeck's round buttons, an AKP03's three buttons.
    pub extra_keys: u8,
    /// `None` for a deck without per-key screens (the Pedal, the Work Louder pads).
    pub image: Option<KeyImage>,
    /// A screen of their own on the extra keys (the Mirabox N1's two buttons);
    /// `None` when they have none.
    pub extra_image: Option<KeyImage>,
    /// Touch-strip segments, one by each dial (Stream Deck +, AKP05, the Loupedeck
    /// side strips) or the whole Touch Bar. They are bindable keys numbered after
    /// every grid key, `keys` to `keys + strip_keys`.
    pub strip_keys: u8,
    /// How one strip segment's face is encoded.
    pub strip_image: Option<KeyImage>,
    pub strip_placement: StripPlacement,
    /// A screen on a wheel or knob (the Loupedeck CT wheel, the Mirabox N1 knob):
    /// one more bindable key, numbered after the strip keys.
    pub wheel_image: Option<KeyImage>,
    /// A screen with no touch (the Stream Deck Neo's info screen): one more key,
    /// after the wheel, shown on the deck but run only from the Companion.
    pub info_image: Option<KeyImage>,
    pub dial_lights: DialLights,
    /// Lights on the deck itself, not tied to a key or dial (the M18's 24 RGB
    /// lights); they take one colour.
    pub deck_lights: u8,
    /// Keys without a screen carry a host-settable RGB light.
    pub key_lights: bool,
    /// Restricts the match to one HID collection of a multi-collection device.
    pub usage_page: Option<u16>,
}

const fn image(width: u32, mode: ImageMode, rotation: Rotation, mirror: Mirror) -> KeyImage {
    KeyImage {
        width,
        height: width,
        mode,
        rotation,
        mirror,
    }
}

/// The 128 x 40 OLED of the earlier SteelSeries Apex keyboards: rows of
/// 16 bytes, the leftmost pixel in the top bit.
const APEX_OLED_ROWS: KeyImage = KeyImage {
    width: steelseries::WIDTH,
    height: steelseries::HEIGHT,
    mode: ImageMode::MonoRows,
    rotation: Rotation::None,
    mirror: Mirror::None,
};

/// The same OLED on the Gen 3 boards, in the panel's own order: five 8-row
/// pages of 128 bytes, the top row in bit 0.
const APEX_OLED_PAGES: KeyImage = KeyImage {
    mode: ImageMode::MonoColumns,
    ..APEX_OLED_ROWS
};

/// A touch-strip segment.
const fn strip(width: u32, height: u32, mode: ImageMode, mirror: Mirror) -> KeyImage {
    KeyImage {
        width,
        height,
        mode,
        rotation: Rotation::None,
        mirror,
    }
}

pub const MODELS: &[DeviceModel] = &[
    DeviceModel {
        id: "elgato-original",
        name: "Stream Deck (2017)",
        family: Family::Elgato,
        vendor_id: ELGATO_VENDOR_ID,
        product_ids: &[0x0060],
        more_usb_ids: &[],
        protocol: Protocol::ElgatoOriginal,
        rows: 3,
        cols: 5,
        keys: 15,
        dials: 0,
        extra_keys: 0,
        image: Some(image(72, ImageMode::Bmp, Rotation::None, Mirror::Both)),
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: false,
        usage_page: None,
    },
    DeviceModel {
        id: "elgato-mk2",
        name: "Stream Deck MK.2",
        family: Family::Elgato,
        vendor_id: ELGATO_VENDOR_ID,
        // 2019 refresh, MK.2, MK.2 scissor keys, and the two MK.2 modules.
        product_ids: &[0x006d, 0x0080, 0x00a5, 0x00b9, 0x00e4],
        more_usb_ids: &[],
        protocol: Protocol::ElgatoV2,
        rows: 3,
        cols: 5,
        keys: 15,
        dials: 0,
        extra_keys: 0,
        image: Some(image(72, ImageMode::Jpeg, Rotation::None, Mirror::Both)),
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: false,
        usage_page: None,
    },
    DeviceModel {
        id: "elgato-mini",
        name: "Stream Deck Mini",
        family: Family::Elgato,
        vendor_id: ELGATO_VENDOR_ID,
        // Mini, Mini MK.2, the Discord edition, and the Mini module.
        product_ids: &[0x0063, 0x0090, 0x00b3, 0x00b8],
        more_usb_ids: &[],
        protocol: Protocol::ElgatoMini,
        rows: 2,
        cols: 3,
        keys: 6,
        dials: 0,
        extra_keys: 0,
        image: Some(image(
            80,
            ImageMode::Bmp,
            Rotation::Quarter,
            Mirror::Horizontal,
        )),
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: false,
        usage_page: None,
    },
    DeviceModel {
        id: "elgato-xl",
        name: "Stream Deck XL",
        family: Family::Elgato,
        vendor_id: ELGATO_VENDOR_ID,
        // XL, XL rev. 2, and the XL module.
        product_ids: &[0x006c, 0x008f, 0x00ba],
        more_usb_ids: &[],
        protocol: Protocol::ElgatoV2,
        rows: 4,
        cols: 8,
        keys: 32,
        dials: 0,
        extra_keys: 0,
        image: Some(image(96, ImageMode::Jpeg, Rotation::None, Mirror::Both)),
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: false,
        usage_page: None,
    },
    DeviceModel {
        id: "elgato-neo",
        name: "Stream Deck Neo",
        family: Family::Elgato,
        vendor_id: ELGATO_VENDOR_ID,
        product_ids: &[0x009a],
        more_usb_ids: &[],
        protocol: Protocol::ElgatoV2,
        // Two rows of four screen keys, then the two touch buttons below them.
        rows: 3,
        cols: 4,
        keys: 10,
        dials: 0,
        extra_keys: 2,
        image: Some(image(96, ImageMode::Jpeg, Rotation::None, Mirror::Both)),
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        // The 248 x 58 info screen between the touch buttons, mounted upside down.
        info_image: Some(KeyImage {
            width: 248,
            height: 58,
            mode: ImageMode::Jpeg,
            rotation: Rotation::None,
            mirror: Mirror::Both,
        }),
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: false,
        usage_page: None,
    },
    DeviceModel {
        id: "elgato-plus",
        name: "Stream Deck +",
        family: Family::Elgato,
        vendor_id: ELGATO_VENDOR_ID,
        product_ids: &[0x0084],
        more_usb_ids: &[],
        protocol: Protocol::ElgatoPlus,
        rows: 2,
        cols: 4,
        keys: 8,
        dials: 4,
        extra_keys: 0,
        image: Some(image(120, ImageMode::Jpeg, Rotation::None, Mirror::None)),
        extra_image: None,
        // The 800 x 100 touch strip, split into one segment per dial.
        strip_keys: 4,
        strip_image: Some(strip(200, 100, ImageMode::Jpeg, Mirror::None)),
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: false,
        usage_page: None,
    },
    DeviceModel {
        id: "elgato-plus-xl",
        name: "Stream Deck + XL",
        family: Family::Elgato,
        vendor_id: ELGATO_VENDOR_ID,
        product_ids: &[0x00c6],
        more_usb_ids: &[],
        protocol: Protocol::ElgatoPlus,
        rows: 4,
        cols: 9,
        keys: 36,
        dials: 6,
        extra_keys: 0,
        // The panels are mounted a quarter turn round: keys and strip are
        // turned 270 degrees. python-elgato-streamdeck sends 112 px keys
        // (rust-elgato-streamdeck says 120).
        image: Some(image(
            112,
            ImageMode::Jpeg,
            Rotation::ThreeQuarter,
            Mirror::None,
        )),
        extra_image: None,
        // The 1200 x 100 strip, one 200 x 100 segment per dial.
        strip_keys: 6,
        strip_image: Some(KeyImage {
            width: 200,
            height: 100,
            mode: ImageMode::Jpeg,
            rotation: Rotation::ThreeQuarter,
            mirror: Mirror::None,
        }),
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: false,
        usage_page: None,
    },
    DeviceModel {
        id: "elgato-studio",
        name: "Stream Deck Studio",
        family: Family::Elgato,
        vendor_id: ELGATO_VENDOR_ID,
        product_ids: &[0x00aa],
        more_usb_ids: &[],
        // The Stream Deck + reports: keys, then two dials. The dials' RGB rings
        // and knob lights are not driven.
        protocol: Protocol::ElgatoPlus,
        rows: 2,
        cols: 16,
        keys: 32,
        dials: 2,
        extra_keys: 0,
        // Tall keys: 80 x 120.
        image: Some(KeyImage {
            width: 80,
            height: 120,
            mode: ImageMode::Jpeg,
            rotation: Rotation::None,
            mirror: Mirror::None,
        }),
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::PerDial,
        deck_lights: 0,
        key_lights: false,
        usage_page: None,
    },
    DeviceModel {
        id: "elgato-pedal",
        name: "Stream Deck Pedal",
        family: Family::Elgato,
        vendor_id: ELGATO_VENDOR_ID,
        product_ids: &[0x0086],
        more_usb_ids: &[],
        protocol: Protocol::ElgatoV2,
        rows: 1,
        cols: 3,
        keys: 3,
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: false,
        usage_page: None,
    },
    DeviceModel {
        id: "corsair-galleon-100-sd",
        name: "Corsair Galleon 100 SD",
        family: Family::Corsair,
        vendor_id: CORSAIR_VENDOR_ID,
        product_ids: &[0x2b18],
        more_usb_ids: &[],
        protocol: Protocol::ElgatoGalleon,
        rows: 4,
        cols: 3,
        keys: 12,
        dials: 2,
        extra_keys: 0,
        // Unlike the plain gen-2 decks, the faces are not mirrored.
        image: Some(image(160, ImageMode::Jpeg, Rotation::None, Mirror::None)),
        extra_image: None,
        // The 720 x 384 screen between the dials and the keys, one half under
        // each dial, as Companion splits it.
        strip_keys: 2,
        strip_image: Some(strip(360, 384, ImageMode::Jpeg, Mirror::None)),
        strip_placement: StripPlacement::Above,
        wheel_image: None,
        info_image: None,
        // Four RGB LEDs round each dial, set to one colour.
        dial_lights: DialLights::PerDial,
        deck_lights: 0,
        key_lights: false,
        usage_page: None,
    },
    DeviceModel {
        id: "ulanzi-d200",
        name: "Ulanzi D200",
        family: Family::Ulanzi,
        vendor_id: ULANZI_VENDOR_ID,
        // The D200, D200H and D200X all enumerate with this one id.
        product_ids: &[0x0019],
        more_usb_ids: &[],
        protocol: Protocol::UlanziD200,
        rows: 3,
        cols: 5,
        // Thirteen keys; the last two cells of the bottom row are the status window.
        keys: 13,
        dials: 0,
        extra_keys: 0,
        image: Some(image(196, ImageMode::Png, Rotation::None, Mirror::None)),
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: false,
        usage_page: None,
    },
    DeviceModel {
        id: "worklouder-creator-micro-2",
        name: "Work Louder Creator Micro 2",
        family: Family::WorkLouder,
        vendor_id: WORK_LOUDER_VENDOR_ID,
        product_ids: &[0x8298, 0x8297],
        more_usb_ids: &[],
        protocol: Protocol::WorkLouderRpc,
        rows: 1,
        cols: 6,
        // The six host-driven agent keys (`KV_OAI_AG00`..`AG05`).
        keys: 6,
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: Some(WORK_LOUDER_RPC_USAGE_PAGE),
    },
    DeviceModel {
        id: "openai-codex-micro",
        name: "OpenAI Codex Micro",
        family: Family::WorkLouder,
        vendor_id: WORK_LOUDER_VENDOR_ID,
        product_ids: &[0x8360],
        more_usb_ids: &[],
        protocol: Protocol::WorkLouderRpc,
        rows: 1,
        cols: 6,
        keys: 6,
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: Some(WORK_LOUDER_RPC_USAGE_PAGE),
    },
    DeviceModel {
        id: "loupedeck-live",
        name: "Loupedeck Live / Razer Stream Controller",
        family: Family::Loupedeck,
        vendor_id: LOUPEDECK_VENDOR_ID,
        product_ids: &[0x0004],
        more_usb_ids: &[(RAZER_VENDOR_ID, 0x0d06)],
        protocol: Protocol::Loupedeck,
        // Twelve touch keys, then the eight lit round buttons in two rows of four.
        rows: 5,
        cols: 4,
        keys: 20,
        dials: 6,
        extra_keys: 8,
        image: Some(image(90, ImageMode::Rgb565, Rotation::None, Mirror::None)),
        extra_image: None,
        // The 60 x 270 side strips, three 60 x 90 segments each, one beside
        // each knob.
        strip_keys: 6,
        strip_image: Some(strip(60, 90, ImageMode::Rgb565, Mirror::None)),
        strip_placement: StripPlacement::Sides,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: "loupedeck-live-s",
        name: "Loupedeck Live S",
        family: Family::Loupedeck,
        vendor_id: LOUPEDECK_VENDOR_ID,
        product_ids: &[0x0006],
        more_usb_ids: &[],
        protocol: Protocol::Loupedeck,
        rows: 4,
        cols: 5,
        keys: 19,
        dials: 2,
        extra_keys: 4,
        image: Some(image(90, ImageMode::Rgb565, Rotation::None, Mirror::None)),
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: "loupedeck-ct",
        name: "Loupedeck CT",
        family: Family::Loupedeck,
        vendor_id: LOUPEDECK_VENDOR_ID,
        product_ids: &[0x0003],
        more_usb_ids: &[],
        protocol: Protocol::Loupedeck,
        // Twelve touch keys, the eight round buttons, then the twelve labelled
        // buttons (Home, Enter, Undo, Save, Keyboard, Fn, A, B, C, D, Fn, E).
        rows: 8,
        cols: 4,
        keys: 32,
        // Six knobs and the centre wheel (dial 7).
        dials: 7,
        extra_keys: 20,
        image: Some(image(90, ImageMode::Rgb565, Rotation::None, Mirror::None)),
        extra_image: None,
        // The 60 x 270 side strips, three 60 x 90 segments each, one beside
        // each knob.
        strip_keys: 6,
        strip_image: Some(strip(60, 90, ImageMode::Rgb565, Mirror::None)),
        strip_placement: StripPlacement::Sides,
        // The wheel's round 240 x 240 screen, its own display ("W").
        wheel_image: Some(image(
            240,
            ImageMode::Rgb565Be,
            Rotation::None,
            Mirror::None,
        )),
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: "razer-stream-controller-x",
        name: "Razer Stream Controller X",
        family: Family::Loupedeck,
        vendor_id: RAZER_VENDOR_ID,
        product_ids: &[0x0d09],
        more_usb_ids: &[],
        protocol: Protocol::Loupedeck,
        rows: 3,
        cols: 5,
        keys: 15,
        dials: 0,
        extra_keys: 0,
        image: Some(image(96, ImageMode::Rgb565, Rotation::None, Mirror::None)),
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: false,
        usage_page: None,
    },
    DeviceModel {
        id: "ajazz-akp153",
        name: "Ajazz AKP153 / Mirabox HSV293S",
        family: Family::Mirabox,
        vendor_id: MIRABOX_VENDOR_ID,
        product_ids: &[0x6674, 0x6670],
        // AKP153E and AKP153R, Mars Gaming MSD-ONE, Mad Dog GK150K, Risemode
        // Vision 01, TMICE, Womier D15, Monstargear TS115.
        more_usb_ids: &[
            (AJAZZ_VENDOR_ID, 0x1010),
            (AJAZZ_VENDOR_ID, 0x1020),
            (0x0b00, 0x1000),
            (0x0c00, 0x1000),
            (0x0a00, 0x1001),
            (0x0500, 0x1001),
            (0x0600, 0x1000),
            (0x0400, 0x1000),
        ],
        protocol: Protocol::MiraboxV1,
        rows: 3,
        cols: 6,
        keys: 18,
        dials: 0,
        extra_keys: 0,
        image: Some(image(85, ImageMode::Jpeg, Rotation::Quarter, Mirror::Both)),
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: false,
        usage_page: Some(MIRABOX_USAGE_PAGE),
    },
    DeviceModel {
        id: "ajazz-akp153-v3",
        name: "Ajazz AKP153 (rev. 2) / Mirabox HSV293S V3",
        family: Family::Mirabox,
        vendor_id: AJAZZ_VENDOR_ID,
        product_ids: &[0x3010, 0x3011],
        // Mirabox HSV293S V3 (two ids), Mars Gaming MSD-ONE (2nd batch),
        // Soomfon Stream Controller, Streonor S15.
        more_usb_ids: &[
            (0x6603, 0x1014),
            (0x6603, 0x1005),
            (0x0b00, 0x1005),
            (0x1500, 0x3003),
            (0x1500, 0x3005),
        ],
        protocol: Protocol::MiraboxV3,
        rows: 3,
        cols: 6,
        keys: 18,
        dials: 0,
        extra_keys: 0,
        // The right-hand column is 82 px; the driver resizes for it.
        image: Some(image(95, ImageMode::Jpeg, Rotation::Quarter, Mirror::Both)),
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: false,
        usage_page: Some(MIRABOX_USAGE_PAGE),
    },
    DeviceModel {
        id: "ajazz-akp03",
        name: "Ajazz AKP03 / Mirabox N3",
        family: Family::Mirabox,
        vendor_id: AJAZZ_VENDOR_ID,
        product_ids: &[0x1001, 0x1002, 0x1003],
        // Mirabox N3 (two ids), Mars Gaming MSD-TWO.
        more_usb_ids: &[(0x6602, 0x1000), (0x6602, 0x1002), (0x0b00, 0x1001)],
        protocol: Protocol::MiraboxV2,
        // Six screen keys, then the three plain buttons below them.
        rows: 3,
        cols: 3,
        keys: 9,
        dials: 3,
        extra_keys: 3,
        image: Some(image(60, ImageMode::Jpeg, Rotation::None, Mirror::None)),
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: false,
        usage_page: Some(MIRABOX_USAGE_PAGE),
    },
    DeviceModel {
        id: "ajazz-akp03-v3",
        name: "Ajazz AKP03 (rev. 2) / Mirabox N3 V3",
        family: Family::Mirabox,
        vendor_id: AJAZZ_VENDOR_ID,
        product_ids: &[0x3002, 0x3003],
        // Mirabox N3 (two ids), Soomfon Stream Controller SE, TreasLin N3,
        // Redragon Skyrider SS-551.
        more_usb_ids: &[
            (0x6603, 0x1002),
            (0x6603, 0x1003),
            (0x1500, 0x3001),
            (MIRABOX_VENDOR_ID, 0x1001),
            (0x0200, 0x2000),
        ],
        protocol: Protocol::MiraboxV3,
        rows: 3,
        cols: 3,
        keys: 9,
        dials: 3,
        extra_keys: 3,
        image: Some(image(64, ImageMode::Jpeg, Rotation::Quarter, Mirror::None)),
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: false,
        usage_page: Some(MIRABOX_USAGE_PAGE),
    },
    DeviceModel {
        id: "ajazz-akp815",
        name: "Ajazz AKP815",
        family: Family::Mirabox,
        vendor_id: MIRABOX_VENDOR_ID,
        product_ids: &[0x6672],
        more_usb_ids: &[],
        protocol: Protocol::MiraboxV1,
        // Stands upright: five rows of three.
        rows: 5,
        cols: 3,
        keys: 15,
        dials: 0,
        extra_keys: 0,
        // The panel is mounted upside down.
        image: Some(image(100, ImageMode::Jpeg, Rotation::None, Mirror::Both)),
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: false,
        usage_page: Some(MIRABOX_USAGE_PAGE),
    },
    DeviceModel {
        id: "ajazz-akp05",
        name: "Ajazz AKP05 / Mirabox N4",
        family: Family::Mirabox,
        vendor_id: AJAZZ_VENDOR_ID,
        // AKP05E, AKP05E Pro, AKP05, AKP05CN Pro.
        product_ids: &[0x3004, 0x3013, 0x3006, 0x3014],
        // Mirabox N4E, N4, N4 Pro E and N4 Pro, VSDInside N4 and N4 Pro, Mars
        // Gaming MSD-Pro, Soomfon CN003, Streonor S10 Pro, Redragon SS-552.
        more_usb_ids: &[
            (0x6603, 0x1007),
            (0x6602, 0x1001),
            (MIRABOX_VENDOR_ID, 0x1021),
            (MIRABOX_VENDOR_ID, 0x1008),
            (MIRABOX_VENDOR_ID, 0x1004),
            (MIRABOX_VENDOR_ID, 0x1023),
            (0x0b00, 0x1003),
            (0x1500, 0x3002),
            (0x1500, 0x3007),
            (0x0200, 0x3001),
        ],
        protocol: Protocol::MiraboxV3,
        // Ten screen keys over a touch strip and four knobs.
        rows: 2,
        cols: 5,
        keys: 10,
        dials: 4,
        extra_keys: 0,
        image: Some(image(112, ImageMode::Jpeg, Rotation::None, Mirror::Both)),
        extra_image: None,
        strip_keys: 4,
        strip_image: Some(strip(176, 112, ImageMode::Jpeg, Mirror::Both)),
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::PerDial,
        deck_lights: 0,
        key_lights: false,
        usage_page: Some(MIRABOX_USAGE_PAGE),
    },
    DeviceModel {
        id: "mirabox-293-v3",
        name: "Mirabox Stream Dock 293 V3",
        family: Family::Mirabox,
        vendor_id: 0x6603,
        product_ids: &[0x1006],
        more_usb_ids: &[],
        protocol: Protocol::MiraboxV3,
        rows: 3,
        cols: 5,
        keys: 15,
        dials: 0,
        extra_keys: 0,
        image: Some(image(112, ImageMode::Jpeg, Rotation::None, Mirror::Both)),
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: false,
        usage_page: Some(MIRABOX_USAGE_PAGE),
    },
    DeviceModel {
        id: "mirabox-m18",
        name: "Mirabox M18 / VSD Inside M18",
        family: Family::Mirabox,
        vendor_id: 0x6603,
        // M18 and M18 EN; VSD Inside's M18.
        product_ids: &[0x1009, 0x1012],
        more_usb_ids: &[(MIRABOX_VENDOR_ID, 0x1000)],
        protocol: Protocol::MiraboxV3,
        // Fifteen screen keys, then the three plain buttons below them.
        rows: 4,
        cols: 5,
        keys: 18,
        dials: 0,
        extra_keys: 3,
        image: Some(image(64, ImageMode::Jpeg, Rotation::None, Mirror::None)),
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        // Its 24 RGB lights, set to one colour.
        deck_lights: 24,
        key_lights: false,
        usage_page: Some(MIRABOX_USAGE_PAGE),
    },
    DeviceModel {
        id: "mirabox-stream-dock-xl",
        name: "Mirabox Stream Dock XL",
        family: Family::Mirabox,
        vendor_id: MIRABOX_VENDOR_ID,
        product_ids: &[0x1031],
        more_usb_ids: &[],
        protocol: Protocol::MiraboxV3,
        rows: 4,
        cols: 8,
        keys: 32,
        dials: 2,
        extra_keys: 0,
        image: Some(image(80, ImageMode::Jpeg, Rotation::None, Mirror::Both)),
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::Shared,
        deck_lights: 0,
        key_lights: false,
        usage_page: Some(MIRABOX_USAGE_PAGE),
    },
    DeviceModel {
        id: "mirabox-n1",
        name: "Mirabox N1",
        family: Family::Mirabox,
        vendor_id: 0x6603,
        product_ids: &[0x1000],
        more_usb_ids: &[(AJAZZ_VENDOR_ID, 0x3007)],
        protocol: Protocol::MiraboxV3,
        // Fifteen screen keys (five rows of three), then buttons A and B; the
        // two buttons and the knob each have a small screen of their own.
        rows: 6,
        cols: 3,
        keys: 17,
        dials: 1,
        extra_keys: 2,
        image: Some(KeyImage {
            width: 108,
            height: 104,
            mode: ImageMode::Jpeg,
            rotation: Rotation::None,
            mirror: Mirror::None,
        }),
        extra_image: Some(image(80, ImageMode::Jpeg, Rotation::None, Mirror::None)),
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: Some(image(80, ImageMode::Jpeg, Rotation::None, Mirror::None)),
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: false,
        usage_page: Some(MIRABOX_USAGE_PAGE),
    },
    DeviceModel {
        id: "redragon-ss550",
        name: "Redragon Streamcraft SS550",
        family: Family::Mirabox,
        vendor_id: 0x0200,
        product_ids: &[0x1000],
        more_usb_ids: &[],
        protocol: Protocol::MiraboxV1,
        // First-generation packets, but it reports releases as well as presses.
        rows: 3,
        cols: 5,
        keys: 15,
        dials: 0,
        extra_keys: 0,
        image: Some(image(100, ImageMode::Jpeg, Rotation::None, Mirror::Both)),
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: false,
        usage_page: Some(MIRABOX_USAGE_PAGE),
    },
    DeviceModel {
        id: "redragon-ss550-v3",
        name: "Redragon Streamcraft SS550 v3",
        family: Family::Mirabox,
        vendor_id: 0x0200,
        product_ids: &[0x3000],
        more_usb_ids: &[],
        protocol: Protocol::MiraboxV2,
        // Second-generation packets, but it reports releases as well as presses.
        rows: 3,
        cols: 5,
        keys: 15,
        dials: 0,
        extra_keys: 0,
        image: Some(image(100, ImageMode::Jpeg, Rotation::None, Mirror::Both)),
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: false,
        usage_page: Some(MIRABOX_USAGE_PAGE),
    },
    DeviceModel {
        id: "womier-duo87",
        name: "Womier DUO87 screen module",
        family: Family::Mirabox,
        vendor_id: 0x0600,
        product_ids: &[0x3001],
        more_usb_ids: &[],
        protocol: Protocol::MiraboxV2,
        // The keyboard's detachable 12-key screen module.
        rows: 4,
        cols: 3,
        keys: 12,
        dials: 0,
        extra_keys: 0,
        image: Some(image(80, ImageMode::Jpeg, Rotation::None, Mirror::Both)),
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: false,
        usage_page: Some(MIRABOX_USAGE_PAGE),
    },
    DeviceModel {
        id: "magtran-m3",
        name: "MagTran M3 (ActionRing / VSD Inside)",
        family: Family::Mirabox,
        vendor_id: MIRABOX_VENDOR_ID,
        product_ids: &[0x1038, 0x1020],
        more_usb_ids: &[],
        protocol: Protocol::MiraboxV3,
        rows: 3,
        cols: 5,
        keys: 15,
        dials: 3,
        extra_keys: 0,
        image: Some(image(96, ImageMode::Jpeg, Rotation::Quarter, Mirror::Both)),
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: false,
        usage_page: Some(MIRABOX_USAGE_PAGE),
    },
    DeviceModel {
        id: "infinitton-idisplay",
        name: "Infinitton iDisplay",
        family: Family::Infinitton,
        // A generic vendor id: the iDisplay is told apart by its product ids.
        vendor_id: 0xffff,
        product_ids: &[0x1f40, 0x1f41],
        more_usb_ids: &[],
        protocol: Protocol::Infinitton,
        rows: 3,
        cols: 5,
        keys: 15,
        dials: 0,
        extra_keys: 0,
        // Each row is sent mirrored.
        image: Some(image(
            72,
            ImageMode::Bgr24,
            Rotation::None,
            Mirror::Horizontal,
        )),
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: false,
        usage_page: None,
    },
    DeviceModel {
        id: "xkeys-xk24",
        name: "X-keys XK-24",
        family: Family::XKeys,
        vendor_id: XKEYS_VENDOR_ID,
        // One id per keyboard-emulation mode.
        product_ids: &[0x0405, 0x0403],
        more_usb_ids: &[],
        protocol: Protocol::XKeysLegacy,
        rows: 6,
        cols: 4,
        keys: 24,
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: "xkeys-xk24-rgb",
        name: "X-keys XK-24M-RGB",
        family: Family::XKeys,
        vendor_id: XKEYS_VENDOR_ID,
        // One id per keyboard-emulation mode.
        product_ids: &[0x057c],
        more_usb_ids: &[],
        protocol: Protocol::XKeysRgb24,
        rows: 6,
        cols: 4,
        keys: 24,
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: "xkeys-xk80",
        name: "X-keys XK-80",
        family: Family::XKeys,
        vendor_id: XKEYS_VENDOR_ID,
        // One id per keyboard-emulation mode.
        product_ids: &[0x0441, 0x0443, 0x04c1, 0x04c4],
        more_usb_ids: &[],
        protocol: Protocol::XKeysLegacy,
        rows: 8,
        cols: 10,
        keys: 80,
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: "xkeys-xke128",
        name: "X-keys XKE-128",
        family: Family::XKeys,
        vendor_id: XKEYS_VENDOR_ID,
        // One id per keyboard-emulation mode.
        product_ids: &[0x04cb, 0x04ce],
        more_usb_ids: &[],
        protocol: Protocol::XKeysLegacy,
        rows: 8,
        cols: 16,
        keys: 128,
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: "xkeys-xk16-lcd",
        name: "X-keys XK-16 LCD",
        family: Family::XKeys,
        vendor_id: XKEYS_VENDOR_ID,
        // One id per keyboard-emulation mode.
        product_ids: &[0x0524, 0x0525, 0x0526, 0x0527, 0x0528, 0x0529, 0x052a],
        more_usb_ids: &[],
        protocol: Protocol::XKeysLegacy,
        // Its two-line text display is not driven.
        rows: 4,
        cols: 4,
        keys: 16,
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: "xkeys-xbk-4x6",
        name: "X-keys X-blox XBK-4x6",
        family: Family::XKeys,
        vendor_id: XKEYS_VENDOR_ID,
        // One id per keyboard-emulation mode.
        product_ids: &[0x0555, 0x0556, 0x0557, 0x0558, 0x0559, 0x055a, 0x055b],
        more_usb_ids: &[],
        protocol: Protocol::XKeysRgbBanks,
        rows: 6,
        cols: 4,
        keys: 24,
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: "xkeys-xbk-3x6",
        name: "X-keys X-blox XBK-3x6",
        family: Family::XKeys,
        vendor_id: XKEYS_VENDOR_ID,
        // One id per keyboard-emulation mode.
        product_ids: &[0x0562, 0x0563, 0x0564, 0x0565, 0x0566, 0x0567, 0x0568],
        more_usb_ids: &[],
        protocol: Protocol::XKeysRgbBanks,
        rows: 6,
        cols: 3,
        keys: 18,
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: "xkeys-xbk-16x6",
        name: "X-keys X-blox XBK-16x6",
        family: Family::XKeys,
        vendor_id: XKEYS_VENDOR_ID,
        // One id per keyboard-emulation mode.
        product_ids: &[0x05d8, 0x05d9, 0x05da, 0x05db, 0x05dc, 0x05dd, 0x05de],
        more_usb_ids: &[],
        protocol: Protocol::XKeysRgbBanks,
        rows: 6,
        cols: 16,
        keys: 96,
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: "xkeys-xk12-jog",
        name: "X-keys XK-12 Jog-Shuttle",
        family: Family::XKeys,
        vendor_id: XKEYS_VENDOR_ID,
        // One id per keyboard-emulation mode.
        product_ids: &[0x0426, 0x0428],
        more_usb_ids: &[],
        protocol: Protocol::XKeysLegacy,
        // The jog wheel is dial 1 and the shuttle ring dial 2.
        rows: 3,
        cols: 4,
        keys: 12,
        dials: 2,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: "xkeys-xba-4x3-jog",
        name: "X-keys X-blox XBA-4x3 Jog-Shuttle",
        family: Family::XKeys,
        vendor_id: XKEYS_VENDOR_ID,
        // One id per keyboard-emulation mode.
        product_ids: &[0x056c, 0x056d, 0x056e, 0x056f, 0x0570, 0x0571, 0x0572],
        more_usb_ids: &[],
        protocol: Protocol::XKeysRgbBanks,
        // The jog wheel is dial 1 and the shuttle ring dial 2.
        rows: 3,
        cols: 4,
        keys: 12,
        dials: 2,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: "logitech-mx-keypad",
        name: "Logitech MX Creative Keypad",
        family: Family::Logitech,
        vendor_id: LOGITECH_VENDOR_ID,
        product_ids: &[0xc354],
        more_usb_ids: &[],
        protocol: Protocol::LogitechHidpp,
        // Nine screen keys in a 3 x 3 grid, then the two page buttons below the
        // left and middle columns.
        rows: 4,
        cols: 3,
        keys: 11,
        dials: 0,
        extra_keys: 2,
        image: Some(image(118, ImageMode::Jpeg, Rotation::None, Mirror::None)),
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: false,
        usage_page: None,
    },
    DeviceModel {
        id: "logitech-g13",
        name: "Logitech G13 Advanced Gameboard",
        family: Family::Logitech,
        vendor_id: LOGITECH_VENDOR_ID,
        product_ids: &[0xc21c],
        more_usb_ids: &[],
        protocol: Protocol::LogitechG13,
        // G1 to G22, M1 to MR, the round button and L1 to L4 under the LCD, the
        // two thumb buttons, the stick click and the stick's four directions,
        // placed as on the board (`g13::KEY_PLACES`). The LCD is key 38.
        rows: 7,
        cols: 7,
        keys: 38,
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        // The 160 x 43 monochrome LCD, one bit per pixel.
        info_image: Some(KeyImage {
            width: 160,
            height: 43,
            mode: ImageMode::MonoColumns,
            rotation: Rotation::None,
            mirror: Mirror::None,
        }),
        dial_lights: DialLights::None,
        // The RGB backlight, which also lights the LCD.
        deck_lights: 1,
        // Only M1, M2, M3 and MR, each on or off (`g13::LIT_KEYS`).
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: g15::G15_ID,
        name: "Logitech G15 (G-keys and LCD)",
        family: Family::Logitech,
        vendor_id: LOGITECH_VENDOR_ID,
        // The G-key and LCD device behind the keyboard's own hub.
        product_ids: &[0xc222],
        more_usb_ids: &[],
        protocol: Protocol::LogitechG15,
        // G1 to G18, M1 to MR, the round button and L2 to L5 under the LCD and
        // the backlight button (`g15::key_names`). The LCD is key 28.
        rows: 8,
        cols: 5,
        keys: 28,
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        // The 160 x 43 monochrome LCD, one bit per pixel.
        info_image: Some(KeyImage {
            width: 160,
            height: 43,
            mode: ImageMode::MonoColumns,
            rotation: Rotation::None,
            mirror: Mirror::None,
        }),
        dial_lights: DialLights::None,
        // Backlight levels only, set by the brightness.
        deck_lights: 0,
        // Only M1, M2, M3 and MR, each on or off (`g15::lit_keys`).
        key_lights: true,
        // The vendor collection (usage page 0xFF00); see `needs_vendor_collection`.
        usage_page: None,
    },
    DeviceModel {
        id: g15::G15_V2_ID,
        name: "Logitech G15 v2 (G-keys and LCD)",
        family: Family::Logitech,
        vendor_id: LOGITECH_VENDOR_ID,
        // The orange-lit G15: its G-key and LCD device behind the keyboard's hub.
        product_ids: &[0xc227],
        more_usb_ids: &[],
        protocol: Protocol::LogitechG15,
        // G1 to G6, M1 to MR, the round button and L2 to L5, and the backlight
        // button. The LCD is key 16.
        rows: 8,
        cols: 5,
        keys: 16,
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        // The 160 x 43 monochrome LCD, one bit per pixel.
        info_image: Some(KeyImage {
            width: 160,
            height: 43,
            mode: ImageMode::MonoColumns,
            rotation: Rotation::None,
            mirror: Mirror::None,
        }),
        dial_lights: DialLights::None,
        // Backlight levels only, set by the brightness.
        deck_lights: 0,
        // Only M1, M2, M3 and MR, each on or off (`g15::lit_keys`).
        key_lights: true,
        // The vendor collection (usage page 0xFF00); see `needs_vendor_collection`.
        usage_page: None,
    },
    DeviceModel {
        id: g15::G11_ID,
        name: "Logitech G11 (G-keys)",
        family: Family::Logitech,
        vendor_id: LOGITECH_VENDOR_ID,
        product_ids: &[0xc225],
        more_usb_ids: &[],
        protocol: Protocol::LogitechG15,
        // G1 to G18, M1 to MR and the backlight button; no LCD.
        rows: 7,
        cols: 4,
        keys: 23,
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        // The keyboard backlight's levels only, set by the brightness.
        deck_lights: 0,
        // Only M1, M2, M3 and MR, each on or off (`g15::lit_keys`).
        key_lights: true,
        // The vendor collection (usage page 0xFF00); see `needs_vendor_collection`.
        usage_page: None,
    },
    DeviceModel {
        id: g15::G510_ID,
        name: "Logitech G510 (G-keys and LCD)",
        family: Family::Logitech,
        vendor_id: LOGITECH_VENDOR_ID,
        // Also the G510s; c22e is either with its USB audio switched on.
        product_ids: &[0xc22d, 0xc22e],
        more_usb_ids: &[],
        protocol: Protocol::LogitechG15,
        // G1 to G18, M1 to MR, L1 to L5, the backlight button, the game-mode
        // switch and the headphone and mic mute keys. The LCD is key 31.
        rows: 9,
        cols: 5,
        keys: 31,
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        // The 160 x 43 monochrome LCD, one bit per pixel.
        info_image: Some(KeyImage {
            width: 160,
            height: 43,
            mode: ImageMode::MonoColumns,
            rotation: Rotation::None,
            mirror: Mirror::None,
        }),
        dial_lights: DialLights::None,
        // The RGB backlight of the keys and the LCD.
        deck_lights: 1,
        // Only M1, M2, M3 and MR, each on or off (`g15::lit_keys`).
        key_lights: true,
        // The vendor collection (usage page 0xFF00); see `needs_vendor_collection`.
        usage_page: None,
    },
    DeviceModel {
        id: g15::G110_ID,
        name: "Logitech G110 (G-keys)",
        family: Family::Logitech,
        vendor_id: LOGITECH_VENDOR_ID,
        product_ids: &[0xc22b],
        more_usb_ids: &[],
        protocol: Protocol::LogitechG15,
        // G1 to G12, M1 to MR and the backlight button.
        rows: 5,
        cols: 4,
        keys: 17,
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        // Its red-to-blue backlight's level only, set by the brightness.
        deck_lights: 0,
        // Only M1, M2, M3 and MR, each on or off (`g15::lit_keys`).
        key_lights: true,
        // The vendor collection (usage page 0xFF00); see `needs_vendor_collection`.
        usage_page: None,
    },
    DeviceModel {
        id: "logitech-g915",
        name: "Logitech G915 (G-keys, wired)",
        family: Family::Logitech,
        vendor_id: LOGITECH_VENDOR_ID,
        product_ids: &[0xc33e],
        more_usb_ids: &[],
        protocol: Protocol::LogitechPerKeyLighting2,
        // G1 to G5, top to bottom down the left edge. Wired only: the
        // LIGHTSPEED receiver and Bluetooth are not driven.
        rows: 5,
        cols: 1,
        keys: 5,
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        // The whole keyboard's backlight, one colour.
        deck_lights: 1,
        // Each G-key's RGB light.
        key_lights: true,
        // The HID++ collection on interface 2 (`DeviceModel::hid_interface`).
        usage_page: None,
    },
    DeviceModel {
        id: "logitech-g815",
        name: "Logitech G815 (G-keys)",
        family: Family::Logitech,
        vendor_id: LOGITECH_VENDOR_ID,
        product_ids: &[0xc33f],
        more_usb_ids: &[],
        protocol: Protocol::LogitechPerKeyLighting2,
        // G1 to G5, top to bottom down the left edge.
        rows: 5,
        cols: 1,
        keys: 5,
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        // The whole keyboard's backlight, one colour.
        deck_lights: 1,
        // Each G-key's RGB light.
        key_lights: true,
        // The HID++ collection on interface 1 (`Protocol::hid_interface`).
        usage_page: None,
    },
    DeviceModel {
        id: "logitech-g910",
        name: "Logitech G910 Orion (G-keys)",
        family: Family::Logitech,
        vendor_id: LOGITECH_VENDOR_ID,
        product_ids: &[0xc32b, 0xc335],
        more_usb_ids: &[],
        protocol: Protocol::LogitechPerKeyLighting,
        // Orion Spark and Orion Spectrum. G1 to G9; G6 to G9 are read from
        // bits the reference drivers do not cover.
        rows: 1,
        cols: 9,
        keys: 9,
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        // The keys' and logo's backlight, one colour.
        deck_lights: 1,
        // Each G-key's RGB light.
        key_lights: true,
        // The HID++ collection on interface 1 (`Protocol::hid_interface`).
        usage_page: None,
    },
    DeviceModel {
        id: "logitech-g513",
        name: "Logitech G513 (backlight)",
        family: Family::Logitech,
        vendor_id: LOGITECH_VENDOR_ID,
        product_ids: &[0xc33c],
        more_usb_ids: &[],
        protocol: Protocol::LogitechPerKeyLighting,
        // No G-keys: only the backlight, as the deck light.
        rows: 0,
        cols: 0,
        keys: 0,
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        // The keys' backlight, one colour (no logo light).
        deck_lights: 1,
        key_lights: false,
        // The HID++ collection on interface 1 (`Protocol::hid_interface`).
        usage_page: None,
    },
    DeviceModel {
        id: "logitech-g-pro-keyboard",
        name: "Logitech G Pro keyboard (backlight)",
        family: Family::Logitech,
        vendor_id: LOGITECH_VENDOR_ID,
        product_ids: &[0xc339],
        more_usb_ids: &[],
        protocol: Protocol::LogitechPerKeyLighting,
        // No G-keys: only the backlight, as the deck light.
        rows: 0,
        cols: 0,
        keys: 0,
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        // The keys' and logo's backlight, one colour.
        deck_lights: 1,
        key_lights: false,
        // The HID++ collection on interface 1 (`Protocol::hid_interface`).
        usage_page: None,
    },
    DeviceModel {
        id: maschine::MK2_ID,
        name: "Native Instruments Maschine MK2",
        family: Family::NativeInstruments,
        vendor_id: NATIVE_INSTRUMENTS_VENDOR_ID,
        product_ids: &[0x1140],
        more_usb_ids: &[],
        protocol: Protocol::MaschineHid,
        // The 16 pads (top-left first), then the 48 buttons in report order
        // (`maschine::key_names`), placed by `maschine::key_cells`. The eight
        // knobs are dials 1 to 8 and the master encoder dial 9.
        rows: 10,
        cols: 8,
        keys: 64,
        dials: 9,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        // The two 256 x 64 monochrome displays side by side, one bit per pixel.
        info_image: Some(KeyImage {
            width: 512,
            height: 64,
            mode: ImageMode::MonoRows,
            rotation: Rotation::None,
            mirror: Mirror::None,
        }),
        dial_lights: DialLights::None,
        deck_lights: 0,
        // RGB pads and group buttons; one-colour LEDs on the other buttons but Main.
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: maschine::MIKRO_MK2_ID,
        name: "Native Instruments Maschine Mikro MK2",
        family: Family::NativeInstruments,
        vendor_id: NATIVE_INSTRUMENTS_VENDOR_ID,
        product_ids: &[0x1200],
        more_usb_ids: &[],
        protocol: Protocol::MaschineHid,
        // The 16 pads, then the 28 buttons; the encoder is dial 1.
        rows: 8,
        cols: 8,
        keys: 44,
        dials: 1,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        // The 128 x 64 monochrome display, in 8-row pages.
        info_image: Some(KeyImage {
            width: 128,
            height: 64,
            mode: ImageMode::MonoColumns,
            rotation: Rotation::None,
            mirror: Mirror::None,
        }),
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: maschine::JAM_ID,
        name: "Native Instruments Maschine Jam",
        family: Family::NativeInstruments,
        vendor_id: NATIVE_INSTRUMENTS_VENDOR_ID,
        product_ids: &[0x1500],
        more_usb_ids: &[],
        protocol: Protocol::MaschineHid,
        // The 8 x 8 matrix, the scene and group buttons, then 35 more buttons.
        // The eight touch strips are dials 1 to 8, the encoder dial 9.
        rows: 15,
        cols: 8,
        keys: 115,
        dials: 9,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        // Palette colours on the matrix, scene and group buttons; one colour on the rest.
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: traktor::F1_ID,
        name: "Native Instruments Traktor Kontrol F1",
        family: Family::NativeInstruments,
        vendor_id: NATIVE_INSTRUMENTS_VENDOR_ID,
        product_ids: &[0x1120],
        more_usb_ids: &[],
        protocol: Protocol::TraktorF1Hid,
        // The 4 x 4 pads (top-left first), then Browse to Sync and Stop 1 to 4
        // (`traktor::KEY_NAMES`), placed by `traktor::KEY_PLACES`. The encoder
        // is dial 1 (its push presses it), the knobs dials 2 to 5 and the
        // faders dials 6 to 9.
        rows: 7,
        cols: 4,
        keys: 28,
        dials: 9,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        // The 2-digit 7-segment display is left dark.
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        // RGB pads; one-colour LEDs on the twelve buttons.
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: komplete::S25_ID,
        name: "Native Instruments Komplete Kontrol S25 (MK1)",
        family: Family::NativeInstruments,
        vendor_id: NATIVE_INSTRUMENTS_VENDOR_ID,
        product_ids: &[0x1340],
        more_usb_ids: &[],
        protocol: Protocol::KompleteKontrolHid,
        // The 23 buttons (`komplete::KEY_NAMES`), placed by
        // `komplete::KEY_PLACES`. The browse encoder is dial 1 (its push
        // presses it), the eight parameter encoders dials 2 to 9.
        rows: 5,
        cols: 7,
        keys: 23,
        dials: 9,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        // The nine segment LCDs are not driven.
        info_image: None,
        dial_lights: DialLights::None,
        // The light guide over the 25 piano keys, in one colour.
        deck_lights: 25,
        // One-colour LEDs on every button.
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: komplete::S49_ID,
        name: "Native Instruments Komplete Kontrol S49 (MK1)",
        family: Family::NativeInstruments,
        vendor_id: NATIVE_INSTRUMENTS_VENDOR_ID,
        product_ids: &[0x1350],
        more_usb_ids: &[],
        protocol: Protocol::KompleteKontrolHid,
        // The 23 buttons (`komplete::KEY_NAMES`), placed by
        // `komplete::KEY_PLACES`. The browse encoder is dial 1 (its push
        // presses it), the eight parameter encoders dials 2 to 9.
        rows: 5,
        cols: 7,
        keys: 23,
        dials: 9,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        // The nine segment LCDs are not driven.
        info_image: None,
        dial_lights: DialLights::None,
        // The light guide over the 49 piano keys, in one colour.
        deck_lights: 49,
        // One-colour LEDs on every button.
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: komplete::S61_ID,
        name: "Native Instruments Komplete Kontrol S61 (MK1)",
        family: Family::NativeInstruments,
        vendor_id: NATIVE_INSTRUMENTS_VENDOR_ID,
        product_ids: &[0x1360],
        more_usb_ids: &[],
        protocol: Protocol::KompleteKontrolHid,
        // The 23 buttons (`komplete::KEY_NAMES`), placed by
        // `komplete::KEY_PLACES`. The browse encoder is dial 1 (its push
        // presses it), the eight parameter encoders dials 2 to 9.
        rows: 5,
        cols: 7,
        keys: 23,
        dials: 9,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        // The nine segment LCDs are not driven.
        info_image: None,
        dial_lights: DialLights::None,
        // The light guide over the 61 piano keys, in one colour.
        deck_lights: 61,
        // One-colour LEDs on every button.
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: komplete::S88_ID,
        name: "Native Instruments Komplete Kontrol S88 (MK1)",
        family: Family::NativeInstruments,
        vendor_id: NATIVE_INSTRUMENTS_VENDOR_ID,
        product_ids: &[0x1410],
        more_usb_ids: &[],
        protocol: Protocol::KompleteKontrolHid,
        // The 23 buttons (`komplete::KEY_NAMES`), placed by
        // `komplete::KEY_PLACES`. The browse encoder is dial 1 (its push
        // presses it), the eight parameter encoders dials 2 to 9.
        rows: 5,
        cols: 7,
        keys: 23,
        dials: 9,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        // The nine segment LCDs are not driven.
        info_image: None,
        dial_lights: DialLights::None,
        // The light guide over the 88 piano keys, in one colour.
        deck_lights: 88,
        // One-colour LEDs on every button.
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: "mountain-displaypad",
        name: "Mountain DisplayPad",
        family: Family::Mountain,
        vendor_id: MOUNTAIN_VENDOR_ID,
        product_ids: &[0x0009],
        more_usb_ids: &[],
        protocol: Protocol::MountainDisplayPad,
        rows: 2,
        cols: 6,
        keys: 12,
        dials: 0,
        extra_keys: 0,
        image: Some(image(102, ImageMode::Bgr24, Rotation::None, Mirror::None)),
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: false,
        // Told apart by interface number (see `Protocol::paired_interfaces`).
        usage_page: None,
    },
    DeviceModel {
        id: "logitech-mx-dialpad",
        name: "Logitech MX Creative Dialpad",
        family: Family::Logitech,
        vendor_id: LOGITECH_VENDOR_ID,
        // Its Bluetooth identity; the Logi Bolt receiver route is not supported.
        product_ids: &[0xbc00],
        more_usb_ids: &[],
        protocol: Protocol::LogitechHidpp,
        // Four buttons (top left, top middle, bottom left, bottom right); the
        // small roller is dial 1 and the large dial dial 2.
        rows: 1,
        cols: 4,
        keys: 4,
        dials: 2,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: false,
        usage_page: None,
    },
    DeviceModel {
        id: "contour-shuttlexpress",
        name: "Contour ShuttleXpress",
        family: Family::Contour,
        vendor_id: CONTOUR_VENDOR_ID,
        product_ids: &[0x0020],
        more_usb_ids: &[],
        protocol: Protocol::ContourShuttle,
        // The jog wheel is dial 1 and the shuttle ring dial 2.
        rows: 1,
        cols: 5,
        keys: 5,
        dials: 2,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: false,
        usage_page: None,
    },
    DeviceModel {
        id: "contour-shuttlepro-v2",
        name: "Contour ShuttlePro v2",
        family: Family::Contour,
        vendor_id: CONTOUR_VENDOR_ID,
        product_ids: &[0x0030],
        more_usb_ids: &[],
        protocol: Protocol::ContourShuttle,
        // The jog wheel is dial 1 and the shuttle ring dial 2.
        rows: 3,
        cols: 5,
        keys: 15,
        dials: 2,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: false,
        usage_page: None,
    },
    DeviceModel {
        id: "contour-shuttlepro-v1",
        name: "Contour ShuttlePro v1",
        family: Family::Contour,
        vendor_id: CONTOUR_VENDOR_ID,
        product_ids: &[0x0010],
        // Early units used X-keys' vendor id.
        more_usb_ids: &[(XKEYS_VENDOR_ID, 0x0240)],
        protocol: Protocol::ContourShuttle,
        // The jog wheel is dial 1 and the shuttle ring dial 2.
        rows: 3,
        cols: 5,
        keys: 13,
        dials: 2,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: false,
        usage_page: None,
    },
    DeviceModel {
        id: "griffin-powermate",
        name: "Griffin PowerMate (USB)",
        family: Family::Griffin,
        vendor_id: GRIFFIN_VENDOR_ID,
        // Linux's powermate driver names 0x04AA the Griffin SoundKnob and
        // drives it exactly like 0x0410; its reports may be 3 bytes, not 6.
        product_ids: &[0x0410, 0x04aa],
        more_usb_ids: &[],
        protocol: Protocol::GriffinPowerMate,
        // No keys: the knob is dial 1, pushed and turned.
        rows: 0,
        cols: 0,
        keys: 0,
        dials: 1,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        // The blue light in the base, at 256 levels.
        deck_lights: 1,
        key_lights: false,
        // One plain HID interface with no report ids.
        usage_page: None,
    },
    DeviceModel {
        id: "vec-footpedal",
        name: "VEC Footpedal",
        family: Family::Vec,
        // P.I. Engineering makes it, on the X-keys vendor id; matched only on
        // its own product id.
        vendor_id: XKEYS_VENDOR_ID,
        product_ids: &[0x00ff],
        more_usb_ids: &[],
        protocol: Protocol::VecPedal,
        // Three pedals, bit order from column 0. Which pedal is which bit is
        // not documented.
        rows: 1,
        cols: 3,
        keys: 3,
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: false,
        usage_page: None,
    },
    DeviceModel {
        id: "tourbox-elite",
        name: "TourBox Elite / Elite Plus",
        family: Family::TourBox,
        vendor_id: TOURBOX_VENDOR_ID,
        product_ids: &[0x2005],
        more_usb_ids: &[],
        protocol: Protocol::TourBoxSerial,
        // Over its USB-C cable. Eleven buttons in reading order (Tall, Side,
        // Top, Short, C1, C2, Tour, then the D-pad); the knob, scroll wheel and
        // dial are dials 1 to 3, and pushing one is its dial press. Its haptics
        // stay off.
        rows: 3,
        cols: 4,
        keys: 11,
        dials: 3,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: false,
        usage_page: None,
    },
    DeviceModel {
        id: "tourbox-neo",
        name: "TourBox Neo",
        family: Family::TourBox,
        vendor_id: TOURBOX_NEO_USB_ID.0,
        product_ids: &[TOURBOX_NEO_USB_ID.1],
        more_usb_ids: &[],
        protocol: Protocol::TourBoxSerial,
        // The Elite's controls, without haptics.
        rows: 3,
        cols: 4,
        keys: 11,
        dials: 3,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: false,
        usage_page: None,
    },
    DeviceModel {
        id: "tourbox-neo-unconfirmed",
        name: "TourBox Neo (unconfirmed id)",
        family: Family::TourBox,
        vendor_id: TOURBOX_TINYUSB_USB_ID.0,
        product_ids: &[TOURBOX_TINYUSB_USB_ID.1],
        more_usb_ids: &[],
        protocol: Protocol::TourBoxSerial,
        // Laid out as a Neo; a TourBox Lite may use this id too, and then
        // only its six buttons, scroll wheel and knob answer.
        rows: 3,
        cols: 4,
        keys: 11,
        dials: 3,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: false,
        usage_page: None,
    },
    DeviceModel {
        id: yamaha::MODEL_ID,
        name: "Yamaha CC121MK2 (CC1)",
        family: Family::Yamaha,
        vendor_id: YAMAHA_VENDOR_ID,
        product_ids: &[0x140f],
        more_usb_ids: &[],
        protocol: Protocol::YamahaCc1Serial,
        // The twelve keys behind the screen (three rows of four), then the
        // seventeen buttons by their printed numbers (1, 2, 15 to 29), of which
        // all but 1 and 2 have a ten-colour light. RC1 to RC6 are dials 1 to 6
        // (RC6 is the jog wheel); the motorised fader is dial 7.
        rows: 8,
        cols: 4,
        keys: 29,
        dials: 7,
        extra_keys: 17,
        image: Some(yamaha::KEY_IMAGE),
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: dji::MODEL_ID,
        name: "DJI RC-N1 (RC231)",
        family: Family::Dji,
        vendor_id: DJI_VENDOR_ID,
        product_ids: &[0x1020],
        more_usb_ids: &[],
        protocol: Protocol::DjiDuml,
        // Over its bottom USB-C port (it reports itself as "DJI C5"). Ten
        // direction keys: each stick's up, down, left and right, then the
        // gimbal wheel's up and down. The five axes (left X, left Y, right X,
        // right Y, wheel) are also dials 1 to 5. Only the left stick's channels
        // are confirmed on a remote. Its buttons are not read.
        rows: 3,
        cols: 4,
        keys: 10,
        dials: 5,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: false,
        usage_page: None,
    },
    DeviceModel {
        id: "blackmagic-speed-editor",
        name: "DaVinci Resolve Speed Editor",
        family: Family::Blackmagic,
        vendor_id: BLACKMAGIC_VENDOR_ID,
        product_ids: &[0xda0e],
        more_usb_ids: &[],
        protocol: Protocol::BlackmagicPanel,
        // Keys sit where they are on the panel; the jog wheel is dial 1. 21 keys
        // have a light, on or off.
        rows: 6,
        cols: 10,
        keys: 43,
        dials: 1,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: "blackmagic-replay-editor",
        name: "DaVinci Resolve Replay Editor",
        family: Family::Blackmagic,
        vendor_id: BLACKMAGIC_VENDOR_ID,
        product_ids: &[0xda11],
        more_usb_ids: &[],
        protocol: Protocol::BlackmagicPanel,
        // The jog wheel is dial 1 and the T-bar dial 2; the deck lights are the
        // T-bar's 16 segments. 46 keys have a light, on or off.
        rows: 9,
        cols: 15,
        keys: 73,
        dials: 2,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 16,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: "blackmagic-atem-micro-panel",
        name: "ATEM Micro Panel",
        family: Family::Blackmagic,
        vendor_id: BLACKMAGIC_VENDOR_ID,
        product_ids: &[0xbef0],
        more_usb_ids: &[],
        protocol: Protocol::BlackmagicPanel,
        // The T-bar is dial 1 and its 16 segments the deck lights. Every key has
        // a light of eight colours (each of red, green and blue on or off).
        rows: 4,
        cols: 17,
        keys: 50,
        dials: 1,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 16,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: "203systems-mystrix",
        name: "203 Systems Mystrix",
        family: Family::Systems203,
        vendor_id: SYSTEMS203_VENDOR_ID,
        product_ids: &MYSTRIX_PRODUCT_IDS,
        more_usb_ids: &[],
        protocol: Protocol::MystrixRawHid,
        // The 8 x 8 grid, row by row from the top left; each pad has an RGB
        // light. The FN key and the touch bars stay with Matrix OS.
        rows: 8,
        cols: 8,
        keys: 64,
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: Some(MYSTRIX_USAGE_PAGE),
    },
    DeviceModel {
        id: "framework-16-macropad",
        name: "Framework Laptop 16 RGB Macropad",
        family: Family::Framework,
        vendor_id: FRAMEWORK_VENDOR_ID,
        product_ids: &[0x0013],
        more_usb_ids: &[],
        protocol: Protocol::FrameworkRawHid,
        // Four columns by six rows, row by row from the top left, each key with
        // an RGB light. The top row is shorter and the bottom-left key wider,
        // but the grid is uniform. Only the Companion raw-HID firmware answers.
        rows: 6,
        cols: 4,
        keys: 24,
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: Some(FRAMEWORK_MACROPAD_USAGE_PAGE),
    },
    DeviceModel {
        id: macropad::MODEL_ID,
        name: "Adafruit MacroPad RP2040 (Companion CircuitPython firmware)",
        family: Family::Adafruit,
        vendor_id: ADAFRUIT_VENDOR_ID,
        // CircuitPython's MacroPad id, shared with stock CircuitPython: only a
        // collection whose descriptor is the Companion firmware's is driven.
        product_ids: &[0x8108],
        more_usb_ids: &[],
        protocol: Protocol::AdafruitMacroPadHid,
        // Three columns by four rows, KEY1 at the top left, row by row, each
        // over an RGB NeoPixel. The encoder, OLED and speaker are not read or
        // driven by the firmware.
        rows: 4,
        cols: 3,
        keys: 12,
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: Some(CONSUMER_USAGE_PAGE),
    },
    DeviceModel {
        id: surfacecore::MODEL_ID,
        name: "Haute42 B16 (SurfaceCore firmware)",
        family: Family::Haute42,
        vendor_id: SURFACECORE_USB_ID.0,
        product_ids: &[SURFACECORE_USB_ID.1],
        more_usb_ids: &[],
        protocol: Protocol::SurfaceCoreHid,
        // Only once SoundMinded's SurfaceCore firmware is flashed to the B16's
        // RP2040: the stock Haute42 (GP2040-CE) firmware enumerates as a game
        // controller on other ids and is not matched. Keys 0 to 15 are B01 to
        // B16, each with an RGB light; 16 to 20 the five unlit function
        // buttons. Placed on a 4 x 15 grid after the SurfaceCore Companion
        // module's layout, which only approximates the real one.
        rows: 4,
        cols: 15,
        keys: 21,
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: Some(SURFACECORE_USAGE_PAGE),
    },
    DeviceModel {
        id: "xencelabs-quick-keys",
        name: "Xencelabs Quick Keys",
        family: Family::Xencelabs,
        vendor_id: XENCELABS_VENDOR_ID,
        product_ids: &[0x5202],
        more_usb_ids: &[],
        protocol: Protocol::XencelabsQuickKeys,
        // Eight labelled keys in two rows, then the unlabelled button; the
        // wheel is dial 1 (its push is the dial press) and its ring the dial light.
        rows: 3,
        cols: 4,
        keys: 9,
        dials: 1,
        extra_keys: 1,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::PerDial,
        deck_lights: 0,
        key_lights: false,
        usage_page: Some(XENCELABS_USAGE_PAGE),
    },
    DeviceModel {
        id: "xencelabs-quick-keys-wireless",
        name: "Xencelabs Quick Keys (wireless)",
        family: Family::Xencelabs,
        vendor_id: XENCELABS_VENDOR_ID,
        product_ids: &[0x5203],
        more_usb_ids: &[],
        protocol: Protocol::XencelabsQuickKeys,
        // One dongle can carry several decks; Ryu drives the first it reports.
        // Eight labelled keys in two rows, then the unlabelled button; the
        // wheel is dial 1 (its push is the dial press) and its ring the dial light.
        rows: 3,
        cols: 4,
        keys: 9,
        dials: 1,
        extra_keys: 1,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::PerDial,
        deck_lights: 0,
        key_lights: false,
        usage_page: Some(XENCELABS_USAGE_PAGE),
    },
    DeviceModel {
        id: "apple-touch-bar",
        name: "MacBook Pro Touch Bar",
        family: Family::Apple,
        vendor_id: APPLE_VENDOR_ID,
        product_ids: &[],
        more_usb_ids: &[],
        protocol: Protocol::TouchBar,
        // No grid: the whole deck is one strip of keys, opened from the Control Strip.
        rows: 0,
        cols: 0,
        keys: 0,
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 8,
        // 112 x 30 points at 2x.
        strip_image: Some(KeyImage {
            width: 224,
            height: 60,
            mode: ImageMode::Png,
            rotation: Rotation::None,
            mirror: Mirror::None,
        }),
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: false,
        usage_page: None,
    },
    DeviceModel {
        id: "coolermaster-masterhub-base",
        name: "MasterHub Base M0064",
        family: Family::CoolerMaster,
        vendor_id: COOLER_MASTER_VENDOR_ID,
        product_ids: &[masterhub::BASE_PRODUCT_ID],
        more_usb_ids: &[],
        protocol: Protocol::MasterHub,
        // Nothing to press: one RGB light on the base, and a heartbeat
        // that is ignored.
        rows: 0,
        cols: 0,
        keys: 0,
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 1,
        key_lights: false,
        usage_page: None,
    },
    DeviceModel {
        id: "coolermaster-masterhub-keypad",
        name: "MasterHub Keypad M4315",
        family: Family::CoolerMaster,
        vendor_id: COOLER_MASTER_VENDOR_ID,
        product_ids: &[masterhub::KEYPAD_PRODUCT_ID],
        more_usb_ids: &[],
        protocol: Protocol::MasterHub,
        // Fifteen screen keys, five to a row. No per-key lights.
        rows: 3,
        cols: 5,
        keys: 15,
        dials: 0,
        extra_keys: 0,
        image: Some(image(72, ImageMode::Jpeg, Rotation::None, Mirror::None)),
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: false,
        usage_page: None,
    },
    DeviceModel {
        id: "coolermaster-masterhub-slider",
        name: "MasterHub Slider M005S",
        family: Family::CoolerMaster,
        vendor_id: COOLER_MASTER_VENDOR_ID,
        product_ids: &[masterhub::SLIDER_PRODUCT_ID],
        more_usb_ids: &[],
        protocol: Protocol::MasterHub,
        // Five faders, leftmost first, each turning its dial by how far it
        // moved; one RGB light per fader.
        rows: 0,
        cols: 0,
        keys: 0,
        dials: 5,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::PerDial,
        deck_lights: 0,
        key_lights: false,
        usage_page: None,
    },
    DeviceModel {
        id: "coolermaster-masterhub-knob",
        name: "MasterHub Knob M003K",
        family: Family::CoolerMaster,
        vendor_id: COOLER_MASTER_VENDOR_ID,
        product_ids: &[masterhub::KNOB_PRODUCT_ID],
        more_usb_ids: &[],
        protocol: Protocol::MasterHub,
        // Three knobs that turn and push, one RGB light each.
        rows: 0,
        cols: 0,
        keys: 0,
        dials: 3,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::PerDial,
        deck_lights: 0,
        key_lights: false,
        usage_page: None,
    },
    DeviceModel {
        id: "coolermaster-masterhub-roller",
        name: "MasterHub Roller M032LE",
        family: Family::CoolerMaster,
        vendor_id: COOLER_MASTER_VENDOR_ID,
        product_ids: &[masterhub::ROLLER_PRODUCT_ID],
        more_usb_ids: &[],
        protocol: Protocol::MasterHub,
        // Two rollers, left first; no lights.
        rows: 0,
        cols: 0,
        keys: 0,
        dials: 2,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: false,
        usage_page: None,
    },
    DeviceModel {
        id: "coolermaster-masterhub-dial",
        name: "MasterHub 1xDial",
        family: Family::CoolerMaster,
        vendor_id: COOLER_MASTER_VENDOR_ID,
        product_ids: &[masterhub::DIAL_PRODUCT_ID],
        more_usb_ids: &[],
        protocol: Protocol::MasterHub,
        // One dial that turns and pushes, with a round 240 x 240 screen
        // drawn as its wheel key.
        rows: 0,
        cols: 0,
        keys: 0,
        dials: 1,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: Some(image(240, ImageMode::Jpeg, Rotation::None, Mirror::None)),
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: false,
        usage_page: None,
    },
    DeviceModel {
        id: "3dconnexion-spacenavigator",
        name: "3Dconnexion SpaceNavigator",
        family: Family::ThreeDconnexion,
        vendor_id: LOGITECH_VENDOR_ID,
        product_ids: &[0xc626, 0xc628],
        more_usb_ids: &[],
        protocol: Protocol::SpaceMouseHid,
        // c628 is the SpaceNavigator for Notebooks, given the same two buttons.
        // The puck's six axes are dials 1 to 6; the blue LED is the deck light.
        rows: 1,
        cols: 2,
        keys: 2,
        dials: 6,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 1,
        key_lights: false,
        usage_page: Some(GENERIC_DESKTOP_USAGE_PAGE),
    },
    DeviceModel {
        id: "3dconnexion-spacemouse-compact",
        name: "3Dconnexion SpaceMouse Compact",
        family: Family::ThreeDconnexion,
        vendor_id: THREEDCONNEXION_VENDOR_ID,
        product_ids: &[0xc635],
        more_usb_ids: &[],
        protocol: Protocol::SpaceMouseHid,
        // The puck's six axes are dials 1 to 6; the LED is the deck light.
        rows: 1,
        cols: 2,
        keys: 2,
        dials: 6,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 1,
        key_lights: false,
        usage_page: Some(GENERIC_DESKTOP_USAGE_PAGE),
    },
    DeviceModel {
        id: "3dconnexion-spacemouse-wireless",
        name: "3Dconnexion SpaceMouse Wireless",
        family: Family::ThreeDconnexion,
        vendor_id: THREEDCONNEXION_VENDOR_ID,
        product_ids: &[0xc62e, 0xc63a, 0xc62f],
        more_usb_ids: &[],
        protocol: Protocol::SpaceMouseHid,
        // c62e on its cable, c63a the newer revision, c62f through its receiver.
        // The puck's six axes are dials 1 to 6; the LED is the deck light.
        rows: 1,
        cols: 2,
        keys: 2,
        dials: 6,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 1,
        key_lights: false,
        usage_page: Some(GENERIC_DESKTOP_USAGE_PAGE),
    },
    DeviceModel {
        id: "3dconnexion-spacemouse-pro",
        name: "3Dconnexion SpaceMouse Pro",
        family: Family::ThreeDconnexion,
        vendor_id: LOGITECH_VENDOR_ID,
        product_ids: &[0xc62b],
        more_usb_ids: &[],
        protocol: Protocol::SpaceMouseHid,
        // The puck's six axes are dials 1 to 6; the LED is the deck light. The
        // keys have no lights of their own.
        rows: 3,
        cols: 5,
        keys: 15,
        dials: 6,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 1,
        key_lights: false,
        usage_page: Some(GENERIC_DESKTOP_USAGE_PAGE),
    },
    DeviceModel {
        id: "3dconnexion-spacemouse-pro-wireless",
        name: "3Dconnexion SpaceMouse Pro Wireless",
        family: Family::ThreeDconnexion,
        vendor_id: THREEDCONNEXION_VENDOR_ID,
        product_ids: &[0xc631, 0xc632, 0xc638],
        more_usb_ids: &[],
        protocol: Protocol::SpaceMouseHid,
        // c631 on its cable, c632 through its receiver, c638 over Bluetooth.
        rows: 3,
        cols: 5,
        keys: 15,
        dials: 6,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 1,
        key_lights: false,
        usage_page: Some(GENERIC_DESKTOP_USAGE_PAGE),
    },
    DeviceModel {
        id: "3dconnexion-universal-receiver",
        name: "3Dconnexion Universal Receiver",
        family: Family::ThreeDconnexion,
        vendor_id: THREEDCONNEXION_VENDOR_ID,
        product_ids: &[0xc652],
        more_usb_ids: &[],
        protocol: Protocol::SpaceMouseHid,
        // Forwards whichever 3D mouse is paired, in the SpaceMouse Pro's layout
        // (a SpaceMouse Wireless's two buttons are keys 1 and 2). Universal
        // Receiver 2 (c65a, c65c) is left out: no reference gives its reports.
        rows: 3,
        cols: 5,
        keys: 15,
        dials: 6,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 1,
        key_lights: false,
        usage_page: Some(GENERIC_DESKTOP_USAGE_PAGE),
    },
    DeviceModel {
        id: "3dconnexion-spacepilot-pro",
        name: "3Dconnexion SpacePilot PRO",
        family: Family::ThreeDconnexion,
        vendor_id: LOGITECH_VENDOR_ID,
        product_ids: &[0xc629],
        more_usb_ids: &[],
        protocol: Protocol::SpaceMouseHid,
        // Its colour LCD speaks a closed protocol, so it stays as 3DxWare left it.
        rows: 3,
        cols: 7,
        keys: 21,
        dials: 6,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 1,
        key_lights: false,
        usage_page: Some(GENERIC_DESKTOP_USAGE_PAGE),
    },
    DeviceModel {
        id: "3dconnexion-spacemouse-enterprise",
        name: "3Dconnexion SpaceMouse Enterprise",
        family: Family::ThreeDconnexion,
        vendor_id: THREEDCONNEXION_VENDOR_ID,
        product_ids: &[0xc633],
        more_usb_ids: &[],
        protocol: Protocol::SpaceMouseHid,
        // 22 of its 31 keys: the references map no others. The LCD that labels
        // its keys speaks a closed protocol and is not drawn.
        rows: 4,
        cols: 6,
        keys: 22,
        dials: 6,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 1,
        key_lights: false,
        usage_page: Some(GENERIC_DESKTOP_USAGE_PAGE),
    },
    DeviceModel {
        id: "3dconnexion-spaceexplorer",
        name: "3Dconnexion SpaceExplorer",
        family: Family::ThreeDconnexion,
        vendor_id: LOGITECH_VENDOR_ID,
        product_ids: &[0xc627],
        more_usb_ids: &[],
        protocol: Protocol::SpaceMouseHid,
        // The puck's six axes are dials 1 to 6; the LED is the deck light.
        rows: 3,
        cols: 5,
        keys: 15,
        dials: 6,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 1,
        key_lights: false,
        usage_page: Some(GENERIC_DESKTOP_USAGE_PAGE),
    },
    DeviceModel {
        id: "3dconnexion-spacepilot",
        name: "3Dconnexion SpacePilot",
        family: Family::ThreeDconnexion,
        vendor_id: LOGITECH_VENDOR_ID,
        product_ids: &[0xc625],
        more_usb_ids: &[],
        protocol: Protocol::SpaceMouseHid,
        // No LED, and its LCD is not drawn.
        rows: 3,
        cols: 7,
        keys: 21,
        dials: 6,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: false,
        usage_page: Some(GENERIC_DESKTOP_USAGE_PAGE),
    },
    DeviceModel {
        id: "3dconnexion-spacemouse-usb",
        name: "3Dconnexion SpaceMouse USB",
        family: Family::ThreeDconnexion,
        vendor_id: THREEDCONNEXION_VENDOR_ID,
        product_ids: &[0xc641],
        more_usb_ids: &[],
        protocol: Protocol::SpaceMouseHid,
        // Only the puck: no buttons and no LED, so it is six dials.
        rows: 0,
        cols: 0,
        keys: 0,
        dials: 6,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: false,
        usage_page: Some(GENERIC_DESKTOP_USAGE_PAGE),
    },
    DeviceModel {
        id: "microsoft-xbox-series",
        name: "Xbox Wireless Controller (Series X|S)",
        family: Family::Microsoft,
        vendor_id: MICROSOFT_VENDOR_ID,
        // The product id differs by transport (USB or Bluetooth); all are this pad.
        product_ids: &[0x0b12, 0x0b13, 0x0b20, 0x0b21],
        more_usb_ids: &[],
        protocol: Protocol::XboxGip,
        // Buttons and triggers, the d-pad and the four menu buttons, then each
        // stick's four directions; the sticks' axes are dials 1 to 4 and their
        // clicks press dials 1 and 3.
        rows: 3,
        cols: 8,
        keys: 24,
        dials: 4,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: false,
        // Matched by collection in `Protocol::accepts_collection`.
        usage_page: None,
    },
    DeviceModel {
        id: "microsoft-xbox-one-s",
        name: "Xbox One S Controller",
        family: Family::Microsoft,
        vendor_id: MICROSOFT_VENDOR_ID,
        product_ids: &[0x02e0, 0x02ea, 0x02fd],
        more_usb_ids: &[],
        protocol: Protocol::XboxGip,
        // The Series controller's reports and layout.
        rows: 3,
        cols: 8,
        keys: 24,
        dials: 4,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: false,
        usage_page: None,
    },
    DeviceModel {
        id: "microsoft-xbox-elite-2",
        name: "Xbox Elite Wireless Controller Series 2",
        family: Family::Microsoft,
        vendor_id: MICROSOFT_VENDOR_ID,
        product_ids: &[0x0b00, 0x0b05, 0x0b22],
        more_usb_ids: &[],
        protocol: Protocol::XboxGip,
        // The standard pad's keys and dials; the back paddles are not decoded.
        rows: 3,
        cols: 8,
        keys: 24,
        dials: 4,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: false,
        usage_page: None,
    },
    DeviceModel {
        id: "sony-dualshock-4",
        name: "DualShock 4",
        family: Family::Sony,
        vendor_id: SONY_VENDOR_ID,
        product_ids: &[0x05c4, 0x09cc],
        more_usb_ids: &[],
        protocol: Protocol::PlayStationDualShock4,
        // The first (CUH-ZCT1) and second (CUH-ZCT2) versions. The face
        // buttons, shoulders and triggers, the d-pad, Share, Options, PS and
        // the touchpad click, then each stick's four directions; the stick axes
        // are dials 1 to 4 and their clicks press dials 1 and 3.
        rows: 3,
        cols: 8,
        keys: 24,
        dials: 4,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        // The light bar.
        deck_lights: 1,
        key_lights: false,
        // Matched by collection in `Protocol::accepts_collection`.
        usage_page: None,
    },
    DeviceModel {
        id: "sony-dualsense",
        name: "DualSense",
        family: Family::Sony,
        vendor_id: SONY_VENDOR_ID,
        product_ids: &[0x0ce6],
        more_usb_ids: &[],
        protocol: Protocol::PlayStationDualSense,
        // The DualShock 4's keys and dials (Create for Share), then Mute.
        rows: 3,
        cols: 9,
        keys: 25,
        dials: 4,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        // The light bar.
        deck_lights: 1,
        key_lights: false,
        // Matched by collection in `Protocol::accepts_collection`.
        usage_page: None,
    },
    DeviceModel {
        id: "sony-dualsense-edge",
        name: "DualSense Edge",
        family: Family::Sony,
        vendor_id: SONY_VENDOR_ID,
        product_ids: &[0x0df2],
        more_usb_ids: &[],
        protocol: Protocol::PlayStationDualSense,
        // The DualSense's keys, then the two Fn buttons and the two back paddles.
        rows: 3,
        cols: 10,
        keys: 29,
        dials: 4,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        // The light bar.
        deck_lights: 1,
        key_lights: false,
        // Matched by collection in `Protocol::accepts_collection`.
        usage_page: None,
    },
    DeviceModel {
        id: "sony-access-controller",
        name: "PlayStation Access Controller",
        family: Family::Sony,
        vendor_id: SONY_VENDOR_ID,
        product_ids: &[0x0e5f],
        more_usb_ids: &[],
        protocol: Protocol::PlayStationAccess,
        // The eight perimeter buttons, the centre, PS and profile buttons (as
        // pressed, whatever the active profile maps them to); the stick's
        // axes are dials 1 and 2 and its click presses dial 1.
        rows: 3,
        cols: 4,
        keys: 11,
        dials: 2,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        // The light bar.
        deck_lights: 1,
        key_lights: false,
        // Matched by collection in `Protocol::accepts_collection`.
        usage_page: None,
    },
    DeviceModel {
        id: "sony-buzz",
        name: "Sony Buzz! Buzzers",
        family: Family::Sony,
        vendor_id: SONY_VENDOR_ID,
        // Linux's hid-sony names 0x0002 the wired buzzers and 0x1000 the
        // wireless ones; both speak the same reports.
        product_ids: &[0x0002, 0x1000],
        more_usb_ids: &[],
        protocol: Protocol::SonyBuzz,
        // One row per handset, top to bottom: the red Buzz! dome, then blue,
        // orange, green and yellow.
        rows: 4,
        cols: 5,
        keys: 20,
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        // Each red dome's on/off LED.
        key_lights: true,
        // One plain HID interface.
        usage_page: None,
    },
    DeviceModel {
        id: "gamesir-cyclone-2",
        name: "GameSir Cyclone 2",
        family: Family::GameSir,
        vendor_id: GAMESIR_VENDOR_ID,
        // 0x100B through the 2.4 GHz dongle and 0x1053 by cable on firmware
        // 3.x, both checked by the reference; 0x101D (cable, firmware 1.x) and
        // 0x102A (another dongle) are not. 0x0575 is the dongle with no
        // controller paired, which answers nothing, so it is left out.
        product_ids: &[0x100b, 0x1053, 0x101d, 0x102a],
        more_usb_ids: &[],
        protocol: Protocol::GameSirConfig,
        // The face buttons, shoulders and triggers, the d-pad, View, Menu,
        // Home and Capture, the back buttons L4, R4 and M, then each stick's
        // four directions; the stick axes are dials 1 to 4 and their clicks
        // press dials 1 and 3.
        rows: 3,
        cols: 9,
        keys: 27,
        dials: 4,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        // The animation's five zones (lower left, lower right, upper left,
        // Home and upper right), in one colour. The column of LEDs between
        // the sticks keeps showing the game profile number and is not driven.
        deck_lights: 4,
        key_lights: false,
        usage_page: Some(GAMESIR_USAGE_PAGE),
    },
    DeviceModel {
        id: "steelseries-apex-pro",
        name: "SteelSeries Apex Pro",
        family: Family::SteelSeries,
        vendor_id: STEELSERIES_VENDOR_ID,
        // The first Apex Pro (full size).
        product_ids: &[0x1610],
        more_usb_ids: &[],
        protocol: Protocol::SteelSeriesOled,
        // A whole keyboard whose keys type as usual: the OLED is the only
        // thing Ryu drives, as the info screen, run from the Companion.
        rows: 0,
        cols: 0,
        keys: 0,
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: Some(APEX_OLED_ROWS),
        dial_lights: DialLights::None,
        // No lighting or brightness command is known; the per-key RGB is not driven.
        deck_lights: 0,
        key_lights: false,
        // Matched by interface (`Protocol::hid_interface`), whatever its usage.
        usage_page: None,
    },
    DeviceModel {
        id: "steelseries-apex-7",
        name: "SteelSeries Apex 7",
        family: Family::SteelSeries,
        vendor_id: STEELSERIES_VENDOR_ID,
        // The reference marks this id as never tested.
        product_ids: &[0x1612],
        more_usb_ids: &[],
        protocol: Protocol::SteelSeriesOled,
        // A whole keyboard whose keys type as usual: the OLED is the only
        // thing Ryu drives, as the info screen, run from the Companion.
        rows: 0,
        cols: 0,
        keys: 0,
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: Some(APEX_OLED_ROWS),
        dial_lights: DialLights::None,
        // No lighting or brightness command is known; the per-key RGB is not driven.
        deck_lights: 0,
        key_lights: false,
        // Matched by interface (`Protocol::hid_interface`), whatever its usage.
        usage_page: None,
    },
    DeviceModel {
        id: "steelseries-apex-pro-tkl",
        name: "SteelSeries Apex Pro TKL",
        family: Family::SteelSeries,
        vendor_id: STEELSERIES_VENDOR_ID,
        // The Apex Pro TKL (2019).
        product_ids: &[0x1614],
        more_usb_ids: &[],
        protocol: Protocol::SteelSeriesOled,
        // A whole keyboard whose keys type as usual: the OLED is the only
        // thing Ryu drives, as the info screen, run from the Companion.
        rows: 0,
        cols: 0,
        keys: 0,
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: Some(APEX_OLED_ROWS),
        dial_lights: DialLights::None,
        // No lighting or brightness command is known; the per-key RGB is not driven.
        deck_lights: 0,
        key_lights: false,
        // Matched by interface (`Protocol::hid_interface`), whatever its usage.
        usage_page: None,
    },
    DeviceModel {
        id: "steelseries-apex-7-tkl",
        name: "SteelSeries Apex 7 TKL",
        family: Family::SteelSeries,
        vendor_id: STEELSERIES_VENDOR_ID,
        // The Apex 7 TKL.
        product_ids: &[0x1618],
        more_usb_ids: &[],
        protocol: Protocol::SteelSeriesOled,
        // A whole keyboard whose keys type as usual: the OLED is the only
        // thing Ryu drives, as the info screen, run from the Companion.
        rows: 0,
        cols: 0,
        keys: 0,
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: Some(APEX_OLED_ROWS),
        dial_lights: DialLights::None,
        // No lighting or brightness command is known; the per-key RGB is not driven.
        deck_lights: 0,
        key_lights: false,
        // Matched by interface (`Protocol::hid_interface`), whatever its usage.
        usage_page: None,
    },
    DeviceModel {
        id: "steelseries-apex-5",
        name: "SteelSeries Apex 5",
        family: Family::SteelSeries,
        vendor_id: STEELSERIES_VENDOR_ID,
        // The Apex 5.
        product_ids: &[0x161c],
        more_usb_ids: &[],
        protocol: Protocol::SteelSeriesOled,
        // A whole keyboard whose keys type as usual: the OLED is the only
        // thing Ryu drives, as the info screen, run from the Companion.
        rows: 0,
        cols: 0,
        keys: 0,
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: Some(APEX_OLED_ROWS),
        dial_lights: DialLights::None,
        // No lighting or brightness command is known; the per-key RGB is not driven.
        deck_lights: 0,
        key_lights: false,
        // Matched by interface (`Protocol::hid_interface`), whatever its usage.
        usage_page: None,
    },
    DeviceModel {
        id: "steelseries-apex-pro-gen3",
        name: "SteelSeries Apex Pro Gen 3",
        family: Family::SteelSeries,
        vendor_id: STEELSERIES_VENDOR_ID,
        // The Apex Pro Gen 3 (full size): one report, as on the earlier boards,
        // but the panel's own 8-row page order.
        product_ids: &[0x1640],
        more_usb_ids: &[],
        protocol: Protocol::SteelSeriesOled,
        // A whole keyboard whose keys type as usual: the OLED is the only
        // thing Ryu drives, as the info screen, run from the Companion.
        rows: 0,
        cols: 0,
        keys: 0,
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: Some(APEX_OLED_PAGES),
        dial_lights: DialLights::None,
        // No lighting or brightness command is known; the per-key RGB is not driven.
        deck_lights: 0,
        key_lights: false,
        // Matched by interface (`Protocol::hid_interface`), whatever its usage.
        usage_page: None,
    },
    DeviceModel {
        id: "steelseries-apex-pro-tkl-wireless-gen3",
        name: "SteelSeries Apex Pro TKL Wireless Gen 3",
        family: Family::SteelSeries,
        vendor_id: STEELSERIES_VENDOR_ID,
        // The Apex Pro TKL Wireless Gen 3 by cable. Through its 2.4 GHz dongle
        // it is another row, since the frame command differs.
        product_ids: &[0x1646],
        more_usb_ids: &[],
        protocol: Protocol::SteelSeriesOledChunked,
        // A whole keyboard whose keys type as usual: the OLED is the only
        // thing Ryu drives, as the info screen, run from the Companion.
        rows: 0,
        cols: 0,
        keys: 0,
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: Some(APEX_OLED_PAGES),
        dial_lights: DialLights::None,
        // No lighting or brightness command is known; the per-key RGB is not driven.
        deck_lights: 0,
        key_lights: false,
        // Matched by interface (`Protocol::hid_interface`), whatever its usage.
        usage_page: None,
    },
    DeviceModel {
        id: steelseries::DONGLE_ID,
        name: "SteelSeries Apex Pro TKL Wireless Gen 3 (2.4 GHz dongle)",
        family: Family::SteelSeries,
        vendor_id: STEELSERIES_VENDOR_ID,
        // The Apex Pro TKL Wireless Gen 3 through its 2.4 GHz dongle.
        product_ids: &[0x1644],
        more_usb_ids: &[],
        protocol: Protocol::SteelSeriesOledChunked,
        // A whole keyboard whose keys type as usual: the OLED is the only
        // thing Ryu drives, as the info screen, run from the Companion.
        rows: 0,
        cols: 0,
        keys: 0,
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: Some(APEX_OLED_PAGES),
        dial_lights: DialLights::None,
        // No lighting or brightness command is known; the per-key RGB is not driven.
        deck_lights: 0,
        key_lights: false,
        // Matched by interface (`Protocol::hid_interface`), whatever its usage.
        usage_page: None,
    },
    DeviceModel {
        id: "prehkeytec-mci-84",
        name: "PrehKeyTec MCI 84",
        family: Family::PrehKeyTec,
        vendor_id: PREHKEYTEC_VENDOR_ID,
        // What product id 0x0b01 is taken for when the keyboard does not say
        // which model it is. Hardware tested by the reference.
        product_ids: &[0x0b01],
        more_usb_ids: &[],
        protocol: Protocol::PrehPosKey,
        rows: 7,
        cols: 12,
        keys: 84,
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: false,
        usage_page: Some(PREHKEYTEC_USAGE_PAGE),
    },
    DeviceModel {
        id: "prehkeytec-mci-128",
        name: "PrehKeyTec MCI 128",
        family: Family::PrehKeyTec,
        vendor_id: PREHKEYTEC_VENDOR_ID,
        // What product id 0x0b06 is taken for when the keyboard does not say
        // which model it is. Hardware tested by the reference.
        product_ids: &[0x0b06],
        more_usb_ids: &[],
        protocol: Protocol::PrehPosKey,
        rows: 8,
        cols: 16,
        keys: 128,
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: false,
        usage_page: Some(PREHKEYTEC_USAGE_PAGE),
    },
    DeviceModel {
        id: "prehkeytec-mci-30",
        name: "PrehKeyTec MCI 30",
        family: Family::PrehKeyTec,
        vendor_id: PREHKEYTEC_VENDOR_ID,
        // Found only by the identity query (it shares a product id). Hardware
        // tested by the reference.
        product_ids: &[],
        more_usb_ids: &[],
        protocol: Protocol::PrehPosKey,
        rows: 6,
        cols: 5,
        keys: 30,
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: false,
        usage_page: Some(PREHKEYTEC_USAGE_PAGE),
    },
    DeviceModel {
        id: "prehkeytec-mci-60",
        name: "PrehKeyTec MCI 60",
        family: Family::PrehKeyTec,
        vendor_id: PREHKEYTEC_VENDOR_ID,
        // Found only by the identity query. Hardware tested by the reference.
        product_ids: &[],
        more_usb_ids: &[],
        protocol: Protocol::PrehPosKey,
        rows: 5,
        cols: 12,
        keys: 60,
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: false,
        usage_page: Some(PREHKEYTEC_USAGE_PAGE),
    },
    DeviceModel {
        id: "prehkeytec-mci-96",
        name: "PrehKeyTec MCI 96",
        family: Family::PrehKeyTec,
        vendor_id: PREHKEYTEC_VENDOR_ID,
        // Found only by the identity query. Only its identity is tested by the
        // reference.
        product_ids: &[],
        more_usb_ids: &[],
        protocol: Protocol::PrehPosKey,
        rows: 6,
        cols: 16,
        keys: 96,
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: false,
        usage_page: Some(PREHKEYTEC_USAGE_PAGE),
    },
    DeviceModel {
        id: "prehkeytec-mci-128a",
        name: "PrehKeyTec MCI 128 Alpha (POS section)",
        family: Family::PrehKeyTec,
        vendor_id: PREHKEYTEC_VENDOR_ID,
        // Found only by the identity query; the POS section only. Layout only,
        // untested on hardware.
        product_ids: &[],
        more_usb_ids: &[],
        protocol: Protocol::PrehPosKey,
        rows: 2,
        cols: 16,
        keys: 32,
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: false,
        usage_page: Some(PREHKEYTEC_USAGE_PAGE),
    },
    DeviceModel {
        id: "prehkeytec-mci-3000",
        name: "PrehKeyTec MCI 3000 (POS section)",
        family: Family::PrehKeyTec,
        vendor_id: PREHKEYTEC_VENDOR_ID,
        // Found only by the identity query; the POS section only. Layout only,
        // untested on hardware.
        product_ids: &[],
        more_usb_ids: &[],
        protocol: Protocol::PrehPosKey,
        rows: 2,
        cols: 4,
        keys: 8,
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: false,
        usage_page: Some(PREHKEYTEC_USAGE_PAGE),
    },
    DeviceModel {
        id: "prehkeytec-mci-3100",
        name: "PrehKeyTec MCI 3100 (POS section)",
        family: Family::PrehKeyTec,
        vendor_id: PREHKEYTEC_VENDOR_ID,
        // Found only by the identity query. A sparse POS section: 22 keys along
        // the bottom row and four at the right of the top row (see
        // `prehkeytec::key_cells`). Layout only, untested on hardware.
        product_ids: &[],
        more_usb_ids: &[],
        protocol: Protocol::PrehPosKey,
        rows: 2,
        cols: 22,
        keys: 26,
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: false,
        usage_page: Some(PREHKEYTEC_USAGE_PAGE),
    },
    DeviceModel {
        id: "prehkeytec-msi-60",
        name: "PrehKeyTec MSI 60",
        family: Family::PrehKeyTec,
        vendor_id: PREHKEYTEC_VENDOR_ID,
        // Found only by the identity query. Layout only, untested on hardware.
        product_ids: &[],
        more_usb_ids: &[],
        protocol: Protocol::PrehPosKey,
        rows: 6,
        cols: 10,
        keys: 60,
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: false,
        usage_page: Some(PREHKEYTEC_USAGE_PAGE),
    },
    DeviceModel {
        id: monome::GRID_64_ID,
        name: "monome grid 64",
        family: Family::Monome,
        vendor_id: FTDI_VENDOR_ID,
        // Found by its USB serial (`m` and digits) and a size query, never by USB id.
        product_ids: &[],
        more_usb_ids: &[],
        protocol: Protocol::MonomeMext,
        rows: 8,
        cols: 8,
        keys: 64,
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: monome::GRID_128_ID,
        name: "monome grid 128",
        family: Family::Monome,
        vendor_id: FTDI_VENDOR_ID,
        // Found by its USB serial or USB strings and a size query, never by USB id.
        product_ids: &[],
        more_usb_ids: &[],
        protocol: Protocol::MonomeMext,
        rows: 8,
        cols: 16,
        keys: 128,
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: monome::ARC_2_ID,
        name: "monome arc 2",
        family: Family::Monome,
        vendor_id: FTDI_VENDOR_ID,
        // Found by its USB serial and the query's encoder count, never by USB id.
        product_ids: &[],
        more_usb_ids: &[],
        protocol: Protocol::MonomeMext,
        rows: 0,
        cols: 0,
        keys: 0,
        dials: 2,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::PerDial,
        deck_lights: 0,
        key_lights: false,
        usage_page: None,
    },
    DeviceModel {
        id: monome::ARC_4_ID,
        name: "monome arc 4",
        family: Family::Monome,
        vendor_id: FTDI_VENDOR_ID,
        // Found by its USB serial and the query's encoder count, never by USB id.
        product_ids: &[],
        more_usb_ids: &[],
        protocol: Protocol::MonomeMext,
        rows: 0,
        cols: 0,
        keys: 0,
        dials: 4,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::PerDial,
        deck_lights: 0,
        key_lights: false,
        usage_page: None,
    },
    DeviceModel {
        id: monome::SERIES_64_ID,
        name: "monome 64 (series)",
        family: Family::Monome,
        vendor_id: FTDI_VENDOR_ID,
        // Found by its USB serial (`m64-` and digits), never by USB id.
        product_ids: &[],
        more_usb_ids: &[],
        protocol: Protocol::MonomeSeries,
        rows: 8,
        cols: 8,
        keys: 64,
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: monome::SERIES_128_ID,
        name: "monome 128 (series)",
        family: Family::Monome,
        vendor_id: FTDI_VENDOR_ID,
        // Found by its USB serial (`m128-` and digits), never by USB id.
        product_ids: &[],
        more_usb_ids: &[],
        protocol: Protocol::MonomeSeries,
        rows: 8,
        cols: 16,
        keys: 128,
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: monome::FORTY_H_ID,
        name: "monome 40h",
        family: Family::Monome,
        vendor_id: FTDI_VENDOR_ID,
        // Found by its USB serial (`m40h` and digits), never by USB id.
        product_ids: &[],
        more_usb_ids: &[],
        protocol: Protocol::Monome40h,
        rows: 8,
        cols: 8,
        keys: 64,
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: monome::ARDUINOME_ID,
        name: "arduinome (40h protocol)",
        family: Family::Monome,
        vendor_id: FTDI_VENDOR_ID,
        // Found by its USB serial (`a40h-` and digits), never by USB id; 57600 baud.
        product_ids: &[],
        more_usb_ids: &[],
        protocol: Protocol::Monome40h,
        rows: 8,
        cols: 8,
        keys: 64,
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: "wooting-one-legacy",
        name: "Wooting One (legacy firmware)",
        family: Family::Wooting,
        vendor_id: ATMEL_VENDOR_ID,
        // The first firmware: input on page 0xFF54 only, mapped from HID usages.
        product_ids: &[0xff01],
        more_usb_ids: &[],
        protocol: Protocol::WootingLegacy,
        // Every cell of the 6-row switch matrix is a key, including the cells
        // with no switch, which never press.
        rows: 6,
        cols: 17,
        keys: 102,
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        // Claimed by its analog collection (`Protocol::paired_collections`).
        usage_page: None,
    },
    DeviceModel {
        id: "wooting-two-legacy",
        name: "Wooting Two (legacy firmware)",
        family: Family::Wooting,
        vendor_id: ATMEL_VENDOR_ID,
        product_ids: &[0xff02],
        more_usb_ids: &[],
        protocol: Protocol::WootingLegacy,
        rows: 6,
        cols: 21,
        keys: 126,
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: "wooting-one",
        name: "Wooting One",
        family: Family::Wooting,
        vendor_id: WOOTING_VENDOR_ID,
        // Each Wooting answers to its base product id plus 0, 1 or 2 for the
        // gamepad mode it is in.
        product_ids: &[0x1100, 0x1101, 0x1102],
        more_usb_ids: &[],
        protocol: Protocol::WootingAnalog,
        rows: 6,
        cols: 17,
        keys: 102,
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: "wooting-two",
        name: "Wooting Two",
        family: Family::Wooting,
        vendor_id: WOOTING_VENDOR_ID,
        product_ids: &[0x1200, 0x1201, 0x1202],
        more_usb_ids: &[],
        protocol: Protocol::WootingAnalog,
        rows: 6,
        cols: 21,
        keys: 126,
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: "wooting-two-lekker",
        name: "Wooting Two Lekker Edition",
        family: Family::Wooting,
        vendor_id: WOOTING_VENDOR_ID,
        product_ids: &[0x1210, 0x1211, 0x1212],
        more_usb_ids: &[],
        protocol: Protocol::WootingAnalog,
        rows: 6,
        cols: 21,
        keys: 126,
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: "wooting-two-he",
        name: "Wooting Two HE",
        family: Family::Wooting,
        vendor_id: WOOTING_VENDOR_ID,
        product_ids: &[0x1220, 0x1221, 0x1222],
        more_usb_ids: &[],
        protocol: Protocol::WootingAnalog,
        rows: 6,
        cols: 21,
        keys: 126,
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: "wooting-two-he-arm",
        name: "Wooting Two HE (ARM)",
        family: Family::Wooting,
        vendor_id: WOOTING_VENDOR_ID,
        product_ids: &[0x1230, 0x1231, 0x1232],
        more_usb_ids: &[],
        protocol: Protocol::WootingAnalog,
        rows: 6,
        cols: 21,
        keys: 126,
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: "wooting-60he",
        name: "Wooting 60HE",
        family: Family::Wooting,
        vendor_id: WOOTING_VENDOR_ID,
        // Five rows of keys in a six-row matrix: one row stays empty.
        product_ids: &[0x1300, 0x1301, 0x1302],
        more_usb_ids: &[],
        protocol: Protocol::WootingAnalog,
        rows: 6,
        cols: 14,
        keys: 84,
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: "wooting-60he-arm",
        name: "Wooting 60HE (ARM)",
        family: Family::Wooting,
        vendor_id: WOOTING_VENDOR_ID,
        product_ids: &[0x1310, 0x1311, 0x1312],
        more_usb_ids: &[],
        protocol: Protocol::WootingAnalog,
        rows: 6,
        cols: 14,
        keys: 84,
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: "wooting-60he-plus",
        name: "Wooting 60HE+",
        family: Family::Wooting,
        vendor_id: WOOTING_VENDOR_ID,
        product_ids: &[0x1320, 0x1321, 0x1322],
        more_usb_ids: &[],
        protocol: Protocol::WootingAnalog,
        rows: 6,
        cols: 14,
        keys: 84,
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: "wooting-60he-v2",
        name: "Wooting 60HE V2",
        family: Family::Wooting,
        vendor_id: WOOTING_VENDOR_ID,
        product_ids: &[0x1340, 0x1341, 0x1342],
        more_usb_ids: &[],
        protocol: Protocol::WootingAnalog,
        rows: 6,
        cols: 14,
        keys: 84,
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: "wooting-80he",
        name: "Wooting 80HE",
        family: Family::Wooting,
        vendor_id: WOOTING_VENDOR_ID,
        product_ids: &[0x1400, 0x1401, 0x1402],
        more_usb_ids: &[],
        protocol: Protocol::WootingAnalog,
        rows: 6,
        cols: 17,
        keys: 102,
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: launchpad::MINI_MK3_ID,
        name: "Novation Launchpad Mini MK3",
        family: Family::Novation,
        vendor_id: NOVATION_VENDOR_ID,
        product_ids: &[0x0113],
        more_usb_ids: &[],
        protocol: Protocol::NovationLaunchpadMidi,
        // The 8 x 8 pads (top-left first), the top row, then the right column
        // top to bottom (`launchpad::key_names`), placed by
        // `launchpad::key_cells`. The logo, top right, is the deck light.
        rows: 9,
        cols: 9,
        keys: 80,
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 1,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: launchpad::X_ID,
        name: "Novation Launchpad X",
        family: Family::Novation,
        vendor_id: NOVATION_VENDOR_ID,
        product_ids: &[0x0103],
        more_usb_ids: &[],
        protocol: Protocol::NovationLaunchpadMidi,
        // Laid out as the Mini MK3; its pads are velocity-sensitive.
        rows: 9,
        cols: 9,
        keys: 80,
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 1,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: launchpad::PRO_MK3_ID,
        name: "Novation Launchpad Pro MK3",
        family: Family::Novation,
        vendor_id: NOVATION_VENDOR_ID,
        product_ids: &[0x0123],
        more_usb_ids: &[],
        protocol: Protocol::NovationLaunchpadMidi,
        // The 8 x 8 pads, the top row, the left and right columns, then the
        // two rows under the pads: 64 pads and 41 buttons. Setup (bottom left)
        // is not reported in Programmer mode; the logo is the deck light.
        rows: 11,
        cols: 10,
        keys: 105,
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 1,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: launchpad_mk2::MK2_ID,
        name: "Novation Launchpad MK2",
        family: Family::Novation,
        vendor_id: NOVATION_VENDOR_ID,
        // 0069 for bootloader device id 1, up to 0078 for id 16.
        product_ids: &[
            0x0069, 0x006a, 0x006b, 0x006c, 0x006d, 0x006e, 0x006f, 0x0070, 0x0071, 0x0072, 0x0073,
            0x0074, 0x0075, 0x0076, 0x0077, 0x0078,
        ],
        more_usb_ids: &[],
        protocol: Protocol::LaunchpadMk2Midi,
        // The 8 x 8 pads (top-left first), the top row, then the right column
        // top to bottom (`launchpad_mk2::key_names`), placed by
        // `launchpad_mk2::key_cells`; the top-right corner has no key and the
        // MK2 has no logo light. Found by MIDI port name; the USB ids are
        // shown only.
        rows: 9,
        cols: 9,
        keys: 80,
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: launchpad_pro::PRO_ID,
        name: "Novation Launchpad Pro",
        family: Family::Novation,
        vendor_id: NOVATION_VENDOR_ID,
        product_ids: &[0x0051],
        more_usb_ids: &[],
        protocol: Protocol::LaunchpadProMidi,
        // The 8 x 8 pads (top-left first), the top row, the left column, the
        // right column, then the bottom row (`launchpad_pro::key_names`),
        // placed on a 10 x 10 grid by `launchpad_pro::key_cells`; the corners
        // are no keys, and the logo is the deck light. Found by MIDI port
        // name (the Standalone port); the USB id is shown only.
        rows: 10,
        cols: 10,
        keys: 96,
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 1,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: novation_dicer::DICER_ID,
        name: "Novation Dicer",
        family: Family::Novation,
        vendor_id: NOVATION_VENDOR_ID,
        product_ids: &[0x0015],
        more_usb_ids: &[],
        protocol: Protocol::NovationDicerMidi,
        // A chained pair on one port: the deck-A unit's buttons 1-5 (row 1),
        // then the deck-B unit's (row 2), named by `novation_dicer::key_names`.
        // Found by MIDI port name; the USB id is shown only.
        rows: 2,
        cols: 5,
        keys: 10,
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: launchpad_legacy::LAUNCHPAD_ID,
        name: "Novation Launchpad",
        family: Family::Novation,
        vendor_id: NOVATION_VENDOR_ID,
        product_ids: &[0x000e],
        more_usb_ids: &[],
        protocol: Protocol::LaunchpadLegacyMidi,
        // The 8 x 8 pads (top-left first), the scene keys top to bottom, then
        // the top row (`launchpad_legacy::key_names`), placed by
        // `launchpad_legacy::key_cells`; the top-right corner has no key.
        // Found by MIDI port name; the USB id is shown only.
        rows: 9,
        cols: 9,
        keys: 80,
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: launchpad_legacy::S_ID,
        name: "Novation Launchpad S",
        family: Family::Novation,
        vendor_id: NOVATION_VENDOR_ID,
        product_ids: &[],
        more_usb_ids: &[],
        protocol: Protocol::LaunchpadLegacyMidi,
        // Laid out as the original. Its USB id is in no reference read.
        rows: 9,
        cols: 9,
        keys: 80,
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: launchpad_legacy::MINI_ID,
        name: "Novation Launchpad Mini (MK1/MK2)",
        family: Family::Novation,
        vendor_id: NOVATION_VENDOR_ID,
        product_ids: &[],
        more_usb_ids: &[],
        protocol: Protocol::LaunchpadLegacyMidi,
        // MK1 and MK2 share a port name and a protocol. The top keys are
        // round (1 to 8) and the scene keys A to H. USB id unknown.
        rows: 9,
        cols: 9,
        keys: 80,
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: launch_control::XL_ID,
        name: "Novation Launch Control XL",
        family: Family::Novation,
        vendor_id: NOVATION_VENDOR_ID,
        product_ids: &[0x0061],
        more_usb_ids: &[],
        protocol: Protocol::NovationLaunchControlXl,
        // Track Focus 1-8 and Track Control 1-8 in rows 3 and 4, the arrows
        // top right and Device, Mute, Solo and Record Arm under them
        // (`launch_control::key_cells`). Dials 1-8, 9-16 and 17-24 are the
        // knob rows top to bottom (each with a light), 25-32 the faders.
        // The MK2 is taken to share the id; it is found by MIDI port name and
        // the USB id is shown only.
        rows: 4,
        cols: 10,
        keys: 24,
        dials: 32,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::PerDial,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: launch_control::LC_ID,
        name: "Novation Launch Control",
        family: Family::Novation,
        vendor_id: NOVATION_VENDOR_ID,
        product_ids: &[0x0034],
        more_usb_ids: &[],
        protocol: Protocol::NovationLaunchControl,
        // Pads 1-8 along the bottom, Up and Down above Left and Right at the
        // right; dials 1-8 the top knobs, 9-16 the bottom ones (no lights).
        // Found by MIDI port name; the USB id is shown only.
        rows: 2,
        cols: 10,
        keys: 12,
        dials: 16,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: launch_control3::XL3_ID,
        name: "Novation Launch Control XL 3",
        family: Family::Novation,
        vendor_id: NOVATION_VENDOR_ID,
        product_ids: &[
            0x0148, 0x0149, 0x014a, 0x014b, 0x014c, 0x014d, 0x014e, 0x014f,
        ],
        more_usb_ids: &[],
        protocol: Protocol::NovationLaunchControlXl3,
        // The two button rows (keys 1-16) in rows 3 and 4, columns 3-10,
        // Solo/Arm, the logo button and Mute/Select left of them, Page,
        // Record, Play and Shift on top and the Track arrows under them
        // (`launch_control3::key_cells`). Dials 1-8, 9-16 and 17-24 are the
        // encoder rows top to bottom (each with a light), 25-32 the faders.
        // The 128 x 64 OLED is the info screen (key 27). The ids are the
        // bootloader device ids 1-8 (Ableton's), shown only: it is found by
        // MIDI port name.
        rows: 4,
        cols: 10,
        keys: 26,
        dials: 32,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        // One bit per pixel; the driver repacks the rows into the deck's
        // 7-pixel bytes.
        info_image: Some(KeyImage {
            width: 128,
            height: 64,
            mode: ImageMode::MonoRows,
            rotation: Rotation::None,
            mirror: Mirror::None,
        }),
        dial_lights: DialLights::PerDial,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: launch_control3::LC3_ID,
        name: "Novation Launch Control 3",
        family: Family::Novation,
        vendor_id: NOVATION_VENDOR_ID,
        product_ids: &[
            0x0157, 0x0158, 0x0159, 0x015a, 0x015b, 0x015c, 0x015d, 0x015e,
        ],
        more_usb_ids: &[],
        protocol: Protocol::NovationLaunchControl3,
        // Shift, then buttons 1-8 in one row; its Page, Track, Function and
        // Mode buttons are not in Novation's map and are not keys. Dials 1-8
        // and 9-16 are the encoder rows (each with a light). The OLED is the
        // info screen (key 10). The ids (Ableton's, device ids 1-8) are shown
        // only: it is found by MIDI port name.
        rows: 1,
        cols: 9,
        keys: 9,
        dials: 16,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: Some(KeyImage {
            width: 128,
            height: 64,
            mode: ImageMode::MonoRows,
            rotation: Rotation::None,
            mirror: Mirror::None,
        }),
        dial_lights: DialLights::PerDial,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: launchkey::MINI_ID,
        name: "Novation Launchkey Mini MK3",
        family: Family::Novation,
        vendor_id: NOVATION_VENDOR_ID,
        product_ids: &[0x0102],
        more_usb_ids: &[],
        protocol: Protocol::LaunchkeyMk3Midi,
        // Pads 1-16 in two rows, then Scene Launch, Stop/Solo/Mute, Track
        // Left and Right, Shift, Play and Record (`launchkey::key_cells`);
        // the knobs are dials 1-8. The USB id is Ableton's controller id for
        // it, taken to be the product id but unconfirmed: shown only, as it
        // is found by MIDI port name.
        rows: 3,
        cols: 9,
        keys: 23,
        dials: 8,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: launchkey::K25_ID,
        name: "Novation Launchkey 25 MK3",
        family: Family::Novation,
        vendor_id: NOVATION_VENDOR_ID,
        product_ids: &[0x0134],
        more_usb_ids: &[],
        protocol: Protocol::LaunchkeyMk3Midi,
        // Pads 1-16 in rows 2 and 3, then the 17 buttons around them
        // (`launchkey::key_cells`); the knobs are dials 1-8. The 16 x 2
        // character display is not used. The USB id (Ableton's controller
        // id) is shown only: it is found by MIDI port name, and on Windows
        // every full-size Launchkey is taken to be this one.
        rows: 4,
        cols: 10,
        keys: 33,
        dials: 8,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: launchkey::K37_ID,
        name: "Novation Launchkey 37 MK3",
        family: Family::Novation,
        vendor_id: NOVATION_VENDOR_ID,
        product_ids: &[0x0135],
        more_usb_ids: &[],
        protocol: Protocol::LaunchkeyMk3Midi,
        // As the 25.
        rows: 4,
        cols: 10,
        keys: 33,
        dials: 8,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: launchkey::K49_ID,
        name: "Novation Launchkey 49 MK3",
        family: Family::Novation,
        vendor_id: NOVATION_VENDOR_ID,
        product_ids: &[0x0136],
        more_usb_ids: &[],
        protocol: Protocol::LaunchkeyMk3Midi,
        // As the 25, plus the fader buttons and Arm/Select in row 5; the
        // knobs are dials 1-8, the faders 9-16 and the master fader 17.
        rows: 5,
        cols: 10,
        keys: 42,
        dials: 17,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: launchkey::K61_ID,
        name: "Novation Launchkey 61 MK3",
        family: Family::Novation,
        vendor_id: NOVATION_VENDOR_ID,
        product_ids: &[0x0137],
        more_usb_ids: &[],
        protocol: Protocol::LaunchkeyMk3Midi,
        // As the 49.
        rows: 5,
        cols: 10,
        keys: 42,
        dials: 17,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: launchkey_mk4::MINI_25_ID,
        name: "Novation Launchkey Mini 25 MK4",
        family: Family::Novation,
        vendor_id: NOVATION_VENDOR_ID,
        product_ids: &[0x0141],
        more_usb_ids: &[],
        protocol: Protocol::LaunchkeyMk4Midi,
        // Pads 1-16 in rows 2 and 3 (columns 2-9), then Pad Up and Down,
        // Scene Launch, Function, Encoder Up and Down, Shift, Play and Record
        // around them (`launchkey_mk4::key_cells`); the knobs are dials 1-8.
        // The OLED is left to the keyboard. The USB id is Ableton's
        // controller id for it, shown only: it is found by MIDI port name.
        rows: 3,
        cols: 10,
        keys: 25,
        dials: 8,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: launchkey_mk4::MINI_37_ID,
        name: "Novation Launchkey Mini 37 MK4",
        family: Family::Novation,
        vendor_id: NOVATION_VENDOR_ID,
        product_ids: &[0x0142],
        more_usb_ids: &[],
        protocol: Protocol::LaunchkeyMk4Midi,
        // As the Mini 25.
        rows: 3,
        cols: 10,
        keys: 25,
        dials: 8,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: launchkey_mk4::K25_ID,
        name: "Novation Launchkey 25 MK4",
        family: Family::Novation,
        vendor_id: NOVATION_VENDOR_ID,
        product_ids: &[0x0143],
        more_usb_ids: &[],
        protocol: Protocol::LaunchkeyMk4Midi,
        // As the Minis, plus Track Left and Right and a bottom row of
        // Capture MIDI, Undo, Quantise, Metronome, Stop, Loop, Play and Record
        // (`launchkey_mk4::key_cells`); the knobs are dials 1-8. The OLED is
        // left to the keyboard. The USB id (Ableton's controller id) is shown
        // only: it is found by MIDI port name.
        rows: 4,
        cols: 10,
        keys: 33,
        dials: 8,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: launchkey_mk4::K37_ID,
        name: "Novation Launchkey 37 MK4",
        family: Family::Novation,
        vendor_id: NOVATION_VENDOR_ID,
        product_ids: &[0x0144],
        more_usb_ids: &[],
        protocol: Protocol::LaunchkeyMk4Midi,
        // As the 25.
        rows: 4,
        cols: 10,
        keys: 33,
        dials: 8,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: launchkey_mk4::K49_ID,
        name: "Novation Launchkey 49 MK4",
        family: Family::Novation,
        vendor_id: NOVATION_VENDOR_ID,
        product_ids: &[0x0145],
        more_usb_ids: &[],
        protocol: Protocol::LaunchkeyMk4Midi,
        // As the 25, plus the fader buttons and Arm/Select in row 5; the
        // knobs are dials 1-8, the faders 9-16 and the master fader 17.
        rows: 5,
        cols: 10,
        keys: 42,
        dials: 17,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: launchkey_mk4::K61_ID,
        name: "Novation Launchkey 61 MK4",
        family: Family::Novation,
        vendor_id: NOVATION_VENDOR_ID,
        product_ids: &[0x0146],
        more_usb_ids: &[],
        protocol: Protocol::LaunchkeyMk4Midi,
        // As the 49.
        rows: 5,
        cols: 10,
        keys: 42,
        dials: 17,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: launchkey_mk2::K25_ID,
        name: "Novation Launchkey 25 MK2",
        family: Family::Novation,
        vendor_id: NOVATION_VENDOR_ID,
        product_ids: &[0x7b7a, 0x007b],
        more_usb_ids: &[],
        protocol: Protocol::LaunchkeyMk2Midi,
        // Pads 1-16 in rows 1 and 2 with Scene Launch and Stop All Clips to
        // their right, then the track, scene and transport buttons in row 3
        // (`launchkey_mk2::key_cells`); the knobs are dials 1-8. The USB id
        // is shown only: it is found by MIDI port name, and a name without a
        // size (the usual case) is taken to be this one.
        rows: 3,
        cols: 9,
        keys: 26,
        dials: 8,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: launchkey_mk2::K49_ID,
        name: "Novation Launchkey 49 MK2",
        family: Family::Novation,
        vendor_id: NOVATION_VENDOR_ID,
        product_ids: &[0x7c7a, 0x007c],
        more_usb_ids: &[],
        protocol: Protocol::LaunchkeyMk2Midi,
        // As the 25, plus the fader buttons and the master button in row 4;
        // the knobs are dials 1-8, the faders 9-16 and the master fader 17.
        // Picked only when the port name says 49.
        rows: 4,
        cols: 9,
        keys: 35,
        dials: 17,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: launchkey_mk2::K61_ID,
        name: "Novation Launchkey 61 MK2",
        family: Family::Novation,
        vendor_id: NOVATION_VENDOR_ID,
        product_ids: &[0x7d7a, 0x007d],
        more_usb_ids: &[],
        protocol: Protocol::LaunchkeyMk2Midi,
        // As the 49; picked only when the port name says 61.
        rows: 4,
        cols: 9,
        keys: 35,
        dials: 17,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: launchkey_mini::LAUNCHKEY_MINI_ID,
        name: "Novation Launchkey Mini",
        family: Family::Novation,
        vendor_id: NOVATION_VENDOR_ID,
        // Ableton's Launchkey Mini script. Shown only: it is found by the
        // name of its InControl MIDI port.
        product_ids: &[0x0035],
        more_usb_ids: &[],
        protocol: Protocol::LaunchkeyMiniMidi,
        // Track Left and Right at the top left, pads 1-16 in two rows of
        // eight, the round Scene Launch and Stop All Clips buttons (red/green
        // lights, as the pads) and Scene Up and Down to their right
        // (`launchkey_mini::key_cells`); the arrows have no light. The knobs
        // are dials 1-8, with no light.
        rows: launchkey_mini::ROWS,
        cols: launchkey_mini::COLS,
        keys: launchkey_mini::KEYS,
        dials: launchkey_mini::DIALS,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: sl_mkiii::SL_MKIII_ID,
        name: "Novation SL MkIII (49 / 61)",
        family: Family::Novation,
        vendor_id: NOVATION_VENDOR_ID,
        product_ids: &[0x0101],
        more_usb_ids: &[],
        protocol: Protocol::NovationSlMk3Midi,
        // The 49 and the 61 share this USB id and their port names, and only
        // their pianos (never driven) differ. Placed by `sl_mkiii::key_cells`
        // on a 6 x 11 grid, folded to fit: the 16 pads (keys 0-15) in two
        // rows in the middle, the three soft button rows above them, the
        // other buttons around them and the transport along the bottom (keys
        // 16-60). The knobs are dials 1-8, lit as their screen column's knob
        // colour, and the faders dials 9-16, lit by their LEDs. Every key
        // but Shift has an RGB light. Found by its InControl MIDI port's
        // name; the USB id is shown only.
        rows: sl_mkiii::ROWS,
        cols: sl_mkiii::COLS,
        keys: sl_mkiii::KEYS,
        dials: sl_mkiii::DIALS,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::PerDial,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: akai_apc::APC_MINI_MK2_ID,
        name: "Akai APC mini MK2",
        family: Family::Akai,
        vendor_id: AKAI_VENDOR_ID,
        product_ids: &[0x004f],
        more_usb_ids: &[],
        protocol: Protocol::AkaiApcMiniMk2Midi,
        // The 8 x 8 pads top left (keys 0-63, reading order), the scene
        // column at the right (64-71), the track row underneath (72-79) and
        // Shift bottom right (80), placed by `akai_apc::key_cells`. The
        // faders are dials 1-8 and the master fader dial 9. Found by MIDI
        // port name; the USB id (Ableton's controller id) is shown only.
        rows: 9,
        cols: 9,
        keys: 81,
        dials: 9,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: akai_apc_mini::APC_MINI_ID,
        name: "Akai APC mini (MK1)",
        family: Family::Akai,
        vendor_id: AKAI_VENDOR_ID,
        product_ids: &[0x0028],
        more_usb_ids: &[],
        protocol: Protocol::AkaiApcMiniMidi,
        // Placed like the MK2 by `akai_apc::key_cells`: the 8 x 8 pads top
        // left (keys 0-63), the scene column at the right (64-71), the track
        // row underneath (72-79) and Shift bottom right (80). The faders are
        // dials 1-8 and the master fader dial 9. Found by MIDI port name; the
        // USB id is shown only.
        rows: 9,
        cols: 9,
        keys: 81,
        dials: 9,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: akai_midimix::MIDIMIX_ID,
        name: "Akai MIDImix",
        family: Family::Akai,
        vendor_id: AKAI_VENDOR_ID,
        product_ids: &[0x0031],
        more_usb_ids: &[],
        protocol: Protocol::AkaiMidimixMidi,
        // Three rows of nine: Mute 1-8 and Bank Left (keys 0-8), Solo 1-8
        // (Mute with SOLO held) and Bank Right (9-17), Rec Arm 1-8 and SOLO
        // (18-26). Dials 1-8 are the top knobs, 9-16 the middle knobs, 17-24
        // the bottom knobs, 25-32 the channel faders and 33 the master fader.
        // Found by MIDI port name; the USB id (Ableton's controller id) is
        // shown only.
        rows: 3,
        cols: 9,
        keys: akai_midimix::KEYS,
        dials: akai_midimix::DIALS,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: akai_lpd8::LPD8_ID,
        name: "Akai LPD8",
        family: Family::Akai,
        vendor_id: AKAI_VENDOR_ID,
        product_ids: &[0x0075],
        more_usb_ids: &[],
        protocol: Protocol::AkaiLpd8Midi,
        // The original LPD8 (MK1) only. Two rows of four pads: PAD5-8 on top
        // (keys 0-3), PAD1-4 below (4-7). Knobs K1-K8 are dials 1-8. Found by
        // MIDI port name; the USB id (Ableton's controller id) is shown only.
        rows: 2,
        cols: 4,
        keys: akai_lpd8::KEYS,
        dials: akai_lpd8::DIALS,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: akai_fire::FIRE_ID,
        name: "Akai Fire",
        family: Family::Akai,
        vendor_id: AKAI_VENDOR_ID,
        product_ids: &[0x0043],
        more_usb_ids: &[],
        protocol: Protocol::AkaiFireMidi,
        // The 4 x 16 pads (keys 0-63, reading order from the top left), then
        // the 20 buttons (64-83), placed by `akai_fire::key_cells` on a 6 x 17
        // grid: the Mute/Solo buttons left of the pad rows, the knob-mode and
        // pattern/browser/grid buttons above, the mode and transport buttons
        // below. The 128 x 64 OLED is the info screen (key 84). Volume, Pan,
        // Filter, Resonance and the Select encoder are dials 1-5. Found by
        // MIDI port name; the USB id is shown only.
        rows: akai_fire::GRID_ROWS,
        cols: akai_fire::GRID_COLS,
        keys: akai_fire::KEYS,
        dials: akai_fire::DIALS,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        // One bit per pixel; the driver repacks the rows into the deck's
        // 7-pixel bands.
        info_image: Some(KeyImage {
            width: 128,
            height: 64,
            mode: ImageMode::MonoRows,
            rotation: Rotation::None,
            mirror: Mirror::None,
        }),
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: akai_apc40::APC40_MK2_ID,
        name: "Akai APC40 MKII",
        family: Family::Akai,
        vendor_id: AKAI_VENDOR_ID,
        product_ids: &[0x0029],
        more_usb_ids: &[],
        protocol: Protocol::AkaiApc40Mk2Midi,
        // Placed by `akai_apc40::key_cells` on a 14 x 9 grid: the 5 x 8 clip
        // matrix top left (keys 0-39) with the scene column at its right
        // (40-44), then rows for clip stop and Stop All (45-53), track select
        // and Master (54-62), mute, solo, arm and crossfade assign (63-94),
        // the device buttons and Bank (95-102, 110), Pan to Session
        // (103-109), and the arrows, Shift, Tap Tempo, Nudge and the foot
        // pedal (111-119). Dials: track knobs 1-8, device knobs 9-16, Tempo
        // 17, Cue 18, channel faders 19-26, master 27, crossfader 28. Found
        // by MIDI port name; the USB id is shown only.
        rows: 14,
        cols: 9,
        keys: akai_apc40::KEYS,
        dials: akai_apc40::DIALS,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: akai_apc40_mk1::APC40_MK1_ID,
        name: "Akai APC40 (MK1)",
        family: Family::Akai,
        vendor_id: AKAI_VENDOR_ID,
        product_ids: &[0x0073],
        more_usb_ids: &[],
        protocol: Protocol::AkaiApc40Mk1Midi,
        // Placed by `akai_apc40_mk1::key_cells` on a 13 x 9 grid: the 5 x 8
        // clip matrix top left (keys 0-39) with the scene column at its right
        // (40-44), then rows for clip stop and Stop All (45-53), track select
        // and Master (54-62), activator, solo and arm (63-86), the device
        // buttons (87-94), Pan to Record and Tap Tempo (95-101, 107), and the
        // arrows, Shift, Nudge and the two footswitches (102-106, 108-111).
        // Dials: track knobs 1-8, device knobs 9-16, Cue 17, channel faders
        // 18-25, master 26, crossfader 27. Found by MIDI port name; the USB
        // id (from Ableton's script) is shown only.
        rows: 13,
        cols: 9,
        keys: akai_apc40_mk1::APC40_KEYS,
        dials: akai_apc40_mk1::APC40_DIALS,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: akai_apc40_mk1::APC20_ID,
        name: "Akai APC20",
        family: Family::Akai,
        vendor_id: AKAI_VENDOR_ID,
        product_ids: &[0x007b],
        more_usb_ids: &[],
        protocol: Protocol::AkaiApc20Midi,
        // Placed by `akai_apc40_mk1::key_cells` on a 10 x 9 grid: the 5 x 8
        // clip matrix top left (keys 0-39) with the scene column at its right
        // (40-44), then rows for clip stop and Shift (45-52, 86), track
        // select and Master (53-61), activator and the foot pedal (62-69,
        // 87), solo (70-77) and arm (78-85). Dials: channel faders 1-8,
        // master 9, Cue 10. Found by MIDI port name; the USB id (from
        // Ableton's script) is shown only.
        rows: 10,
        cols: 9,
        keys: akai_apc40_mk1::APC20_KEYS,
        dials: akai_apc40_mk1::APC20_DIALS,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: akai_apc_key25::APC_KEY_25_ID,
        name: "Akai APC Key 25",
        family: Family::Akai,
        vendor_id: AKAI_VENDOR_ID,
        product_ids: &[0x0027],
        more_usb_ids: &[],
        protocol: Protocol::AkaiApcKey25Midi,
        // Placed by `akai_apc_key25::key_cells` on a 7 x 9 grid: the 5 x 8
        // clip matrix top left (keys 0-39), the scene launch column at its
        // right (40-44), the track buttons underneath (45-52) with Stop All
        // Clips at the right (53), then Shift, Play and Record (56, 54, 55).
        // The eight knobs are dials 1-8. Found by MIDI port name (`APC Key
        // 25`, not `mk2`); the port also carries the piano keys, on another
        // channel. The USB id (Ableton's controller id) is shown only.
        rows: 7,
        cols: 9,
        keys: akai_apc_key25::KEYS,
        dials: akai_apc_key25::DIALS,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: akai_apc_key25::APC_KEY_25_MK2_ID,
        name: "Akai APC Key 25 MK2",
        family: Family::Akai,
        vendor_id: AKAI_VENDOR_ID,
        product_ids: &[0x004e],
        more_usb_ids: &[],
        protocol: Protocol::AkaiApcKey25Mk2Midi,
        // Placed like the MK1 by `akai_apc_key25::key_cells`. Found by its
        // second (`Control`) MIDI port, never the first (`Keys`); the exact
        // port names per host are unconfirmed on hardware. The USB id
        // (Ableton's controller id) is shown only.
        rows: 7,
        cols: 9,
        keys: akai_apc_key25::KEYS,
        dials: akai_apc_key25::DIALS,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: akai_mpk_mk4::MPK_MINI_IV_ID,
        name: "Akai MPK mini IV",
        family: Family::Akai,
        vendor_id: AKAI_VENDOR_ID,
        // A logical 7 x 4 grid in key order: the eight pads in the first
        // bank (upper row keys 0-3, lower row 4-7) and again in the second
        // (8-15), switched by the deck's own bank buttons; then Shift, -, +,
        // Tap Tempo, Undo, Loop, Play, Record and Overdub (16-24). Every key
        // but Shift has a light (pads by palette, buttons on or off). Dials
        // 0-7 are the knobs, 8 the main encoder (with its push), none with a
        // light. Found by its `DAW Port` name, never the keybed's `MIDI
        // Port`. The USB id (Ableton's controller id) is shown only.
        product_ids: &[0x005d],
        more_usb_ids: &[],
        protocol: Protocol::AkaiMpkMk4Midi,
        rows: akai_mpk_mk4::ROWS,
        cols: akai_mpk_mk4::COLS,
        keys: akai_mpk_mk4::KEYS,
        dials: akai_mpk_mk4::DIALS,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        // The colour screen takes only text lines; the driver writes fixed
        // status text.
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: akai_mpk_mk4::MPK_MINI_PLUS_II_ID,
        name: "Akai MPK mini Plus II",
        family: Family::Akai,
        vendor_id: AKAI_VENDOR_ID,
        // Laid out like the MPK mini IV, with its own SysEx product id. Its
        // USB product id is not known; found by its `DAW Port` name only.
        product_ids: &[],
        more_usb_ids: &[],
        protocol: Protocol::AkaiMpkMk4Midi,
        rows: akai_mpk_mk4::ROWS,
        cols: akai_mpk_mk4::COLS,
        keys: akai_mpk_mk4::KEYS,
        dials: akai_mpk_mk4::DIALS,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        // The colour screen takes only text lines; the driver writes fixed
        // status text.
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: akai_mpk_mini_plus::MPK_MINI_PLUS_ID,
        name: "Akai MPK mini Plus",
        family: Family::Akai,
        vendor_id: AKAI_VENDOR_ID,
        // The first generation only (never the MPK mini Plus II). One row of
        // the five transport buttons: Rewind, Fast Forward, Stop, Play and
        // Record (that order left to right is assumed), each with an on/off
        // light. The pads and knobs follow the active program and are not
        // keys or dials. Found by MIDI port name; the USB id (Ableton's
        // controller id) is shown only.
        product_ids: &[0x0054],
        more_usb_ids: &[],
        protocol: Protocol::AkaiMpkMiniPlusMidi,
        rows: 1,
        cols: 5,
        keys: akai_mpk_mini_plus::KEYS,
        dials: akai_mpk_mini_plus::DIALS,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: push1::PUSH_1_ID,
        name: "Ableton Push (1st gen)",
        family: Family::Ableton,
        vendor_id: AKAI_VENDOR_ID,
        product_ids: &[0x0015],
        more_usb_ids: &[],
        protocol: Protocol::AbletonPush1Midi,
        // Placed by `push1::key_cells` on a 13 x 14 grid: the 8 x 8 pads
        // (keys 0-63, row by row from the top left) in the middle, the 63
        // bindable buttons (keys 64-126, in controller order, User left out)
        // around them where they sit on the device, both 8-button rows under
        // the display. Dials: Tempo 1, Swing 2, the track encoders 3-10,
        // Master 11 and the touch strip 12. Every key has a light: 72 RGB by
        // palette, 16 red/amber/yellow/green and 39 single-colour. The 4 x 68
        // character display shows fixed text only. Found by its User MIDI
        // port's name; the USB id is shown only.
        rows: 13,
        cols: 14,
        keys: push1::KEYS,
        dials: push1::DIALS,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: push2::PUSH_2_ID,
        name: "Ableton Push 2",
        family: Family::Ableton,
        vendor_id: ABLETON_VENDOR_ID,
        product_ids: &[0x1967],
        more_usb_ids: &[],
        protocol: Protocol::AbletonPush2Midi,
        // Placed by `push2::key_cells` on a 13 x 14 grid: the 8 x 8 pads
        // (keys 0-63, row by row from the top left) in the middle, the 64
        // bindable buttons (keys 64-127, in controller order, User left out)
        // around them where they sit on the device. Dials: Tempo 1, Swing 2,
        // the track encoders 3-10, Master 11 and the touch strip 12. Every
        // key has a light: 94 RGB and 34 white. The 960 x 160 display is a
        // libusb bulk interface and is not driven. Found by its User MIDI
        // port's name; the USB id is shown only.
        rows: 13,
        cols: 14,
        keys: push2::KEYS,
        dials: push2::DIALS,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: xtouch_mini::XTOUCH_MINI_ID,
        name: "Behringer X-Touch Mini",
        family: Family::Behringer,
        vendor_id: BEHRINGER_VENDOR_ID,
        // Not confirmed by the references read; shown only.
        product_ids: &[0x00b3],
        more_usb_ids: &[],
        protocol: Protocol::BehringerXTouchMiniMidi,
        // Placed by `xtouch_mini::key_cells` on a 2 x 9 grid: the two rows
        // of eight buttons (keys 0-7 on top, 8-15 below, left to right), then
        // Layer A (16) and Layer B (17) in the right-hand column beside the
        // fader. The encoders are dials 1-8 and the fader dial 9; each
        // encoder's ring takes a dial colour, the fader has no light. Found
        // by MIDI port name; the USB id is shown only.
        rows: 2,
        cols: 9,
        keys: xtouch_mini::KEYS,
        dials: xtouch_mini::DIALS,
        extra_keys: 2,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::PerDial,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: xtouch_one::XTOUCH_ONE_ID,
        name: "Behringer X-Touch One",
        family: Family::Behringer,
        // Behringer's vendor id, shown only; no product id is known.
        vendor_id: BEHRINGER_VENDOR_ID,
        product_ids: &[],
        more_usb_ids: &[],
        protocol: Protocol::BehringerXTouchOneMidi,
        // A 6 x 6 grid in button-number order: the 35 buttons (keys 0-34,
        // each with an on/off light) and the fader touch (35, no light);
        // their places on the deck are not known. The encoder, jog wheel and
        // fader are dials 1-3, with no light. The deck light colours the LCD
        // backlight. Found by MIDI port name.
        rows: 6,
        cols: 6,
        keys: xtouch_one::KEYS,
        dials: xtouch_one::DIALS,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 1,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: behringer_bcontrol::BCR2000_ID,
        name: "Behringer BCR2000",
        family: Family::Behringer,
        // 1397:00bc as Ableton's script gives it, shown only: the BCF2000
        // reports the same id (usb.ids names it so), so the deck is found
        // by MIDI port name and the BCF2000 row lists no product id.
        vendor_id: BEHRINGER_VENDOR_ID,
        product_ids: &[0x00bc],
        more_usb_ids: &[],
        protocol: Protocol::BehringerBControlMidi,
        // Keys 0-15 are the two button rows, 16-19 the 2 x 2 box at the
        // right, each with an on/off light. Dials 0-7 are the push
        // encoders (any of their four groups; pushing presses the dial),
        // 8-31 the three rows of plain encoders. The rings only show values.
        rows: behringer_bcontrol::ROWS,
        cols: behringer_bcontrol::COLS,
        keys: behringer_bcontrol::KEYS,
        dials: behringer_bcontrol::BCR_DIALS,
        extra_keys: behringer_bcontrol::EXTRA_KEYS,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: behringer_bcontrol::BCF2000_ID,
        name: "Behringer BCF2000",
        family: Family::Behringer,
        // Reports 1397:00bc as the BCR2000 does; the id is listed on that
        // row only, and the deck is found by MIDI port name.
        vendor_id: BEHRINGER_VENDOR_ID,
        product_ids: &[],
        more_usb_ids: &[],
        protocol: Protocol::BehringerBControlMidi,
        // Keys as on the BCR2000. Dials 0-7 are the push encoders (any of
        // their four groups), 8-15 the motor faders, which Ryu never moves.
        rows: behringer_bcontrol::ROWS,
        cols: behringer_bcontrol::COLS,
        keys: behringer_bcontrol::KEYS,
        dials: behringer_bcontrol::BCF_DIALS,
        extra_keys: behringer_bcontrol::EXTRA_KEYS,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: korg::NANOKONTROL2_ID,
        name: "Korg nanoKONTROL2",
        family: Family::Korg,
        vendor_id: KORG_VENDOR_ID,
        product_ids: &[0x0117],
        more_usb_ids: &[],
        protocol: Protocol::KorgNanoKontrol2,
        // Placed by `korg::key_cells` on a 3 x 13 grid like the panel: Track
        // ◀ ▶ top left, Cycle and Marker Set ◀ ▶ below, the transport along
        // the bottom, and the eight strips' S, M and R buttons (keys 0-23) in
        // columns 5-12. Track and Marker have no LED (`korg::lit_keys`); the
        // others are on or off. The knobs are dials 0-7 and the sliders 8-15,
        // with no light. Found by MIDI port name; the USB id is shown only.
        rows: 3,
        cols: 13,
        keys: korg::NK2_KEYS,
        dials: korg::NK2_DIALS,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: korg::STUDIO_ID,
        name: "Korg nanoKONTROL Studio",
        family: Family::Korg,
        // KORG's vendor id, shown only. No reference read gives the USB
        // product id (`0137` is its SysEx family code, and 0944:010f is the
        // original nanoKONTROL), so the Studio is matched by port name only.
        vendor_id: KORG_VENDOR_ID,
        product_ids: &[],
        more_usb_ids: &[],
        protocol: Protocol::KorgNanoKontrolStudio,
        // Placed by `korg::key_cells` on a 4 x 14 grid: Scene, Track ◀ ▶ and
        // Marker Set ◀ ▶ top left, Cycle below, the transport (Return to
        // Zero to Rec) along the bottom, and the eight strips in columns
        // 6-13, Mute over Solo over Rec over Select (keys 0-7 Solo, 8-15
        // Mute, 16-23 Rec, 24-31 Select). Every button but Scene (key 44) has
        // an on/off LED. The knobs are dials 0-7, the sliders 8-15 and the
        // jog wheel 16, with no light. USB only, never Bluetooth MIDI.
        rows: 4,
        cols: 14,
        keys: korg::STUDIO_KEYS,
        dials: korg::STUDIO_DIALS,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: korg_padkontrol::PADKONTROL_ID,
        name: "Korg padKONTROL",
        family: Family::Korg,
        vendor_id: KORG_VENDOR_ID,
        // Ableton's padKONTROL script. Shown only: it is found by the name
        // of its Native Mode MIDI port (Port 2: PORT A in, CTRL out).
        product_ids: &[0x0105],
        more_usb_ids: &[],
        protocol: Protocol::KorgPadKontrol,
        // Placed by `korg_padkontrol::key_cells` on a 6 x 8 grid like the
        // panel (approximately): the 16 pads (keys 0-15, pad 1 top left) on
        // the right, the 19 buttons (keys 16-34, in switch order) to their
        // left and above, the X-Y pad's touch (35) and the pedal jack (36)
        // bottom left. Pads and buttons have an on/off LED; the touch and the
        // pedal have none. The encoder, Knob 1 and 2 and the X-Y pad's X and
        // Y are dials 0-4, with no light. The 7-segment display is not driven.
        rows: korg_padkontrol::ROWS,
        cols: korg_padkontrol::COLS,
        keys: korg_padkontrol::KEYS,
        dials: korg_padkontrol::DIALS,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: arturia_beatstep::BEATSTEP_ID,
        name: "Arturia BeatStep",
        family: Family::Arturia,
        vendor_id: ARTURIA_VENDOR_ID,
        // Ableton's controller id for it; shown only. The BeatStep Pro is
        // not listed: no reference read gives its port name or protocol.
        product_ids: &[0x0206],
        more_usb_ids: &[],
        protocol: Protocol::ArturiaBeatStepMidi,
        // Placed by `arturia_beatstep::key_cells` on a 2 x 10 grid: the 16
        // pads (keys 0-15, top row then bottom row, left to right) in
        // columns 2-9, Shift (16) top left, Stop (17) and Play (18) bottom
        // left. Only the pads have lights (red, blue or both). The 16
        // encoders are dials 0-15 (top row then bottom row) and the large
        // encoder dial 16, with no light. Found by MIDI port name.
        rows: arturia_beatstep::ROWS,
        cols: arturia_beatstep::COLS,
        keys: arturia_beatstep::KEYS,
        dials: arturia_beatstep::DIALS,
        extra_keys: arturia_beatstep::EXTRA_KEYS,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: arturia_minilab3::MINILAB3_ID,
        name: "Arturia MiniLab 3",
        family: Family::Arturia,
        vendor_id: ARTURIA_VENDOR_ID,
        // Ableton's controller id for it (product 8715); shown only.
        product_ids: &[0x220b],
        more_usb_ids: &[],
        protocol: Protocol::ArturiaMiniLab3Midi,
        // A logical 3 x 8 grid in key order: the eight pads as bank A (keys
        // 0-7) on top and as bank B (8-15) under them, switched by the deck's
        // own bank button; then Shift (16, no light) and Loop, Stop, Play,
        // Record and Tap (17-21). Every key but Shift has an RGB light. Dials
        // 0-7 are the encoders, 8 the main encoder and 9 the same knob with
        // Shift held, 10-13 the faders (no motors), none with a light. Found
        // by MIDI port name, in the DAW program.
        rows: arturia_minilab3::ROWS,
        cols: arturia_minilab3::COLS,
        keys: arturia_minilab3::KEYS,
        dials: arturia_minilab3::DIALS,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: arturia_keylab_essential3::KEYLAB_ESSENTIAL_49_MK3_ID,
        name: "Arturia KeyLab Essential 49 mk3",
        family: Family::Arturia,
        vendor_id: ARTURIA_VENDOR_ID,
        // Ableton's controller id for it (product 588); shown only.
        product_ids: &[0x024c],
        more_usb_ids: &[],
        protocol: Protocol::ArturiaKeyLabEssential3Midi,
        // A logical 5 x 8 grid in key order: bank A's pads (top row keys
        // 0-3, bottom row 4-7) and bank B's (8-11, 12-15), switched by the
        // deck's own Bank button; Context 1-4 and Save, Punch, Undo, Redo
        // (16-23); Loop, Rewind, Fast-forward, Metronome, Stop, Play, Record
        // and Tap (24-31); Part (32) and Bank (33, no light). Every key but
        // Bank has an RGB light. Dials 0-8 are the knobs, 9 the display
        // encoder, 10-18 the faders (no motors), none with a light. Found by
        // MIDI port name, in the DAW program.
        rows: arturia_keylab_essential3::ROWS,
        cols: arturia_keylab_essential3::COLS,
        keys: arturia_keylab_essential3::KEYS,
        dials: arturia_keylab_essential3::DIALS,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: arturia_keylab_essential3::KEYLAB_ESSENTIAL_61_MK3_ID,
        name: "Arturia KeyLab Essential 61 mk3",
        family: Family::Arturia,
        vendor_id: ARTURIA_VENDOR_ID,
        // Ableton's controller id for it (product 652); shown only.
        product_ids: &[0x028c],
        more_usb_ids: &[],
        protocol: Protocol::ArturiaKeyLabEssential3Midi,
        // The same deck as the 49 with a longer keybed.
        rows: arturia_keylab_essential3::ROWS,
        cols: arturia_keylab_essential3::COLS,
        keys: arturia_keylab_essential3::KEYS,
        dials: arturia_keylab_essential3::DIALS,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: arturia_keylab_essential3::KEYLAB_ESSENTIAL_88_MK3_ID,
        name: "Arturia KeyLab Essential 88 mk3",
        family: Family::Arturia,
        vendor_id: ARTURIA_VENDOR_ID,
        // Ableton's controller id for it (product 716); shown only.
        product_ids: &[0x02cc],
        more_usb_ids: &[],
        protocol: Protocol::ArturiaKeyLabEssential3Midi,
        // The same deck as the 49 with a longer keybed.
        rows: arturia_keylab_essential3::ROWS,
        cols: arturia_keylab_essential3::COLS,
        keys: arturia_keylab_essential3::KEYS,
        dials: arturia_keylab_essential3::DIALS,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: midi_fighter_twister::TWISTER_ID,
        name: "DJ TechTools Midi Fighter Twister",
        family: Family::DjTechTools,
        vendor_id: DJTT_VENDOR_ID,
        // The factory id and the three alternate-id firmware images; shown
        // only.
        product_ids: &[0x0007, 0x1007, 0x2007, 0x3007],
        more_usb_ids: &[],
        protocol: Protocol::MidiFighterTwisterMidi,
        // The four outer side buttons on a 2 x 2 grid (left top, right top,
        // left bottom, right bottom); the middle two switch banks on the deck
        // and are not keys. The 16 knobs are dials 0-15, row by row from the
        // top left, each with an RGB light (a palette colour) and a white
        // ring Ryu dims with it. Found by MIDI port name.
        rows: midi_fighter_twister::ROWS,
        cols: midi_fighter_twister::COLS,
        keys: midi_fighter_twister::KEYS,
        dials: midi_fighter_twister::DIALS,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::PerDial,
        deck_lights: 0,
        key_lights: false,
        usage_page: None,
    },
    DeviceModel {
        id: midi_fighter_spectra::SPECTRA_ID,
        name: "DJ TechTools Midi Fighter Spectra",
        family: Family::DjTechTools,
        vendor_id: DJTT_VENDOR_ID,
        // Shown only.
        product_ids: &[0x0006],
        more_usb_ids: &[],
        protocol: Protocol::MidiFighterSpectraMidi,
        // The 16 arcade buttons, row by row from the top left, each with an
        // RGB light (a palette colour); the six side buttons switch the
        // deck's banks and are not keys. The four white ground-effect lights
        // are the deck lights, at one level. Found by MIDI port name.
        rows: midi_fighter_spectra::ROWS,
        cols: midi_fighter_spectra::COLS,
        keys: midi_fighter_spectra::KEYS,
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: midi_fighter_spectra::DECK_LIGHTS,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: midi_fighter_64::MF64_ID,
        name: "DJ TechTools Midi Fighter 64",
        family: Family::DjTechTools,
        vendor_id: DJTT_VENDOR_ID,
        // No reference gives its product id: found by MIDI port name, never
        // by USB id.
        product_ids: &[],
        more_usb_ids: &[],
        protocol: Protocol::MidiFighter64Midi,
        // The 64 arcade buttons, row by row from the top left, each with a
        // palette-colour light and a steady brightness mode.
        rows: midi_fighter_64::ROWS,
        cols: midi_fighter_64::COLS,
        keys: midi_fighter_64::KEYS,
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: intech_grid::GRID_ID,
        name: "Intech Studio Grid",
        family: Family::Intech,
        vendor_id: ATMEL_VENDOR_ID,
        // Every module type (EN16, PO16, BU16, EF44, PBF4, TEK2) shares these
        // ids, its port name and no identity reply, so one generic row serves
        // them all. 03eb:ecad is the D51 firmware's and 03eb:ecac Grid
        // Editor's older module id; 2e8a:1145 is also the RP2350 bootloader's,
        // which has no MIDI port. Shown only.
        product_ids: &[0xecad, 0xecac],
        more_usb_ids: &[INTECH_GRID_ESP32_USB_ID, INTECH_GRID_RP2350_USB_ID],
        protocol: Protocol::IntechGridMidi,
        // Element i of the module plugged into USB is key i (its button or
        // push) and dial i (its turn, potentiometer or fader), placed row by
        // row from the top left whatever the module's own layout. No light
        // from Ryu. Found by MIDI port name.
        rows: intech_grid::ROWS,
        cols: intech_grid::COLS,
        keys: intech_grid::KEYS,
        dials: intech_grid::DIALS,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: false,
        usage_page: None,
    },
    DeviceModel {
        id: presonus_atom::ATOM_ID,
        name: "PreSonus ATOM",
        family: Family::Presonus,
        vendor_id: PRESONUS_VENDOR_ID,
        product_ids: &[0x0206],
        more_usb_ids: &[],
        protocol: Protocol::PresonusAtomMidi,
        // The 16 RGB pads in reading order (keys 0-15), then the 20 buttons
        // (keys 16-35: Shift, Zoom, Note Repeat, Full Level, Bank, Preset,
        // Show/Hide, Nudge, Editor, Set Loop, Setup, Up, Down, Left, Right,
        // Select, Click, Record, Play, Stop), four to a row below the pads
        // rather than at their places on the panel. The four encoders are
        // dials 1-4. Found by MIDI port name; the USB id (Ableton's) is shown
        // only.
        rows: 9,
        cols: 4,
        keys: presonus_atom::KEYS,
        dials: presonus_atom::DIALS,
        extra_keys: presonus_atom::BUTTONS,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: presonus_atom_sq::ATOM_SQ_ID,
        name: "PreSonus ATOM SQ",
        family: Family::Presonus,
        vendor_id: PRESONUS_VENDOR_ID,
        product_ids: &[0x020a],
        more_usb_ids: &[],
        protocol: Protocol::PresonusAtomSqMidi,
        // Row 1 the 16 upper pads and row 2 the 16 lower pads (keys 0-31, RGB),
        // then the 31 buttons (keys 32-62: Bank A-H, Display 1-6, Shift, Song,
        // Instrument, Editor, User, Display Left and Right, Up, Down, Left,
        // Right, Click, Record, Play, Stop, Plus, Minus) on rows 3 and 4 rather
        // than at their places on the panel. The eight encoders, the display
        // encoder and the touch strip are dials 1-10. Its text display shows
        // Ryu and the display buttons' labels. Found by MIDI port name; the
        // USB id is shown only.
        rows: presonus_atom_sq::ROWS,
        cols: presonus_atom_sq::COLS,
        keys: presonus_atom_sq::KEYS,
        dials: presonus_atom_sq::DIALS,
        extra_keys: presonus_atom_sq::BUTTONS,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: faderport::FADERPORT_ID,
        name: "PreSonus FaderPort",
        family: Family::Presonus,
        // PreSonus' vendor id, shown only; the classic FaderPort's product id
        // is not known.
        vendor_id: PRESONUS_VENDOR_ID,
        product_ids: &[],
        more_usb_ids: &[],
        protocol: Protocol::PresonusFaderPortMidi,
        // A 4 x 7 grid in the deck's button-id order: the 24 buttons (keys
        // 0-23, each with an on/off light), the footswitch (24) and the fader
        // touch (25), neither lit; their places on the panel are not known.
        // The encoder and the motor fader are dials 1 and 2. Found by MIDI
        // port name.
        rows: faderport::ROWS,
        cols: faderport::COLS,
        keys: faderport::KEYS,
        dials: faderport::DIALS,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: faderport8::FADERPORT2_ID,
        name: "PreSonus FaderPort 2 (2018)",
        family: Family::Presonus,
        vendor_id: PRESONUS_VENDOR_ID,
        product_ids: &[0x0205],
        more_usb_ids: &[],
        protocol: Protocol::PresonusFaderPort8Midi,
        // A 4 x 7 grid in note order rather than at the buttons' places: 28
        // keys, of which the encoder push, the footswitch and the fader touch
        // have no light; nine buttons take a colour. The encoder and the fader
        // are dials 1 and 2. Native mode only. Found by MIDI port name; the
        // USB id (Ableton's) is shown only.
        rows: faderport8::FP2_ROWS,
        cols: faderport8::FP2_COLS,
        keys: faderport8::FP2_KEYS,
        dials: faderport8::FP2_DIALS,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: faderport8::FADERPORT8_ID,
        name: "PreSonus FaderPort 8",
        family: Family::Presonus,
        vendor_id: PRESONUS_VENDOR_ID,
        // 0x0203 is Ableton's; 0x0202 is in a comment in Ardour's code shared
        // by the three models, so it is not confirmed as the FaderPort 8's.
        product_ids: &[0x0203, 0x0202],
        more_usb_ids: &[],
        protocol: Protocol::PresonusFaderPort8Midi,
        // Rows 1-4 the eight strips' Select (RGB), Mute, Solo and fader touch
        // (no light), then the 42 global buttons on rows 5-10 rather than at
        // their places on the panel. The two encoders and the eight faders
        // are dials 1-10. Native mode only. The scribble strips are cleared.
        // Found by MIDI port name; the USB ids are shown only.
        rows: faderport8::FP8_ROWS,
        cols: faderport8::FP8_COLS,
        keys: faderport8::FP8_KEYS,
        dials: faderport8::FP8_DIALS,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: faderport8::FADERPORT16_ID,
        name: "PreSonus FaderPort 16",
        family: Family::Presonus,
        vendor_id: PRESONUS_VENDOR_ID,
        product_ids: &[0x0204],
        more_usb_ids: &[],
        protocol: Protocol::PresonusFaderPort8Midi,
        // Rows 1-4 the 16 strips' Select (RGB), Mute, Solo and fader touch (no
        // light), then the 42 global buttons on rows 5-7. The two encoders and
        // the 16 faders are dials 1-18. Native mode only; kept alive each
        // second. The scribble strips are cleared. Found by MIDI port name;
        // the USB id (Ableton's) is shown only.
        rows: faderport8::FP16_ROWS,
        cols: faderport8::FP16_COLS,
        keys: faderport8::FP16_KEYS,
        dials: faderport8::FP16_DIALS,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: faderfox_ec4::FADERFOX_EC4_ID,
        name: "Faderfox EC4",
        family: Family::Faderfox,
        // No reference read gives Faderfox's USB vendor or product id, so
        // none is shown (vendor 0000, unlike the X-Touch One, which shows
        // Behringer's).
        vendor_id: 0,
        product_ids: &[],
        more_usb_ids: &[],
        protocol: Protocol::FaderfoxEc4Midi,
        // Rows 1-4 are the 16 encoder pushes as they sit on the deck (keys
        // 0-15), row 5 User 1-4 (FUNC held with encoder 1, 5, 9 or 13) and
        // row 6 Shift alone; none has a light Ryu sets. Each push key's label
        // is written to its encoder's name on the OLED. The encoders are
        // dials 1-16, in the same order. Found by MIDI port name.
        rows: faderfox_ec4::ROWS,
        cols: faderfox_ec4::COLS,
        keys: faderfox_ec4::KEYS,
        dials: faderfox_ec4::DIALS,
        extra_keys: faderfox_ec4::EXTRA_KEYS,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: false,
        usage_page: None,
    },
    DeviceModel {
        id: livid_base::BASE_ID,
        name: "Livid Instruments Base",
        family: Family::Livid,
        // No reference read gives Livid's USB vendor or product id (the
        // 2536 / 115 in aumhaa's scripts is a placeholder every Livid script
        // shares), so none is shown, as for the Faderfox EC4.
        vendor_id: 0,
        product_ids: &[],
        more_usb_ids: &[],
        protocol: Protocol::LividBaseMidi,
        // Row 1 is the eight touch buttons, rows 2-5 the 32 pads as they sit
        // and row 6 the eight function buttons (a layout choice), each with a
        // palette light. The nine touch faders are dials 1-9, the master
        // last; their lights are not driven. Found by MIDI port name
        // (`Base8`).
        rows: livid_base::ROWS,
        cols: livid_base::COLS,
        keys: livid_base::KEYS,
        dials: livid_base::DIALS,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: livid_base::BASE_II_ID,
        name: "Livid Instruments Base II",
        family: Family::Livid,
        // No reference read gives Livid's USB vendor or product id (the
        // 2536 / 115 in aumhaa's scripts is a placeholder every Livid script
        // shares), so none is shown, as for the Faderfox EC4.
        vendor_id: 0,
        product_ids: &[],
        more_usb_ids: &[],
        protocol: Protocol::LividBaseMidi,
        // Row 1 is the eight touch buttons, rows 2-5 the 32 pads as they sit
        // and row 6 the eight function buttons (a layout choice), each with a
        // palette light. The nine touch faders are dials 1-9, the master
        // last; their lights are not driven. Found by MIDI port name
        // (`Base2`).
        rows: livid_base::ROWS,
        cols: livid_base::COLS,
        keys: livid_base::KEYS,
        dials: livid_base::DIALS,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: xone_k2::XONE_K2_ID,
        name: "Allen & Heath Xone:K2",
        family: Family::AllenHeath,
        // Allen & Heath's vendor id (Mixxx's udev rules), shown only; no
        // reference read gives the Xone:K2's product id.
        vendor_id: ALLEN_HEATH_VENDOR_ID,
        product_ids: &[],
        more_usb_ids: &[],
        protocol: Protocol::AllenHeathXoneK2Midi,
        // Row 1 is the top encoder pushes, rows 2-4 the three button rows
        // under the knobs, rows 5-8 the bottom 4 x 4 matrix, each button
        // with a red, amber or green light; Layer (red, amber or green) and
        // Exit Setup (on or off) sit at the ends of row 9
        // (`xone_k2::key_cells`). Dials 1-4 are the top encoders, 5-16 the
        // knobs, 17-20 the faders (never moved: not motorised) and 21-22 the
        // bottom encoders, whose pushes press them; no dial has a light.
        // Found by MIDI port name (`XONE:K2`).
        rows: xone_k2::ROWS,
        cols: xone_k2::COLS,
        keys: xone_k2::KEYS,
        dials: xone_k2::DIALS,
        extra_keys: xone_k2::EXTRA_KEYS,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: exquis::EXQUIS_ID,
        name: "Intuitive Instruments Exquis",
        family: Family::IntuitiveInstruments,
        // No reference gives a USB vendor or product id: found by MIDI port
        // name only (`exquis`, never the exquis-midi library's own port).
        vendor_id: 0,
        product_ids: &[],
        more_usb_ids: &[],
        protocol: Protocol::ExquisMidi,
        // The 61 hexagonal pads, top hex row first (the half-pad offset of
        // the five-pad rows is not shown), then Record, Loop, Clips,
        // Play/Stop, Down and Up, then Undo and Redo, then the six slider
        // portions, each with an RGB LED (`exquis::key_cells`; the extra
        // rows are a logical arrangement). Dials 1-4 are the encoders, each
        // lit (its encoder and encoder-button LEDs). Settings and Sound stay
        // the device's own.
        rows: exquis::ROWS,
        cols: exquis::COLS,
        keys: exquis::KEYS,
        dials: exquis::DIALS,
        extra_keys: exquis::EXTRA_KEYS,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::PerDial,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: maudio_oxygen_pro::OXYGEN_PRO_25_ID,
        name: "M-Audio Oxygen Pro 25",
        family: Family::MAudio,
        vendor_id: MAUDIO_VENDOR_ID,
        // A logical 6 x 8 grid: the 16 pads (top row, then bottom row) and
        // the 8 track buttons, all lit (pads by palette, track buttons on or
        // off), then 24 unlit buttons: transport, Back, banks, Preset, DAW,
        // Scene 1 and 2, Shift and the nine mode buttons. Found by its DAW
        // (`Mackie/HUI`, Windows interface 3) port name, never the keybed's;
        // the USB id (Ableton's controller id) is shown only.
        // Dials 0-7 are the knobs, 8 the master fader (the only fader), 9
        // the main encoder (with its push), none with a light.
        product_ids: &[0x0038],
        more_usb_ids: &[],
        protocol: Protocol::MAudioOxygenProMidi,
        rows: maudio_oxygen_pro::ROWS,
        cols: maudio_oxygen_pro::COLS,
        keys: maudio_oxygen_pro::KEYS,
        dials: maudio_oxygen_pro::DIALS_25,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: maudio_oxygen_pro::OXYGEN_PRO_49_ID,
        name: "M-Audio Oxygen Pro 49",
        family: Family::MAudio,
        vendor_id: MAUDIO_VENDOR_ID,
        // A logical 6 x 8 grid: the 16 pads (top row, then bottom row) and
        // the 8 track buttons, all lit (pads by palette, track buttons on or
        // off), then 24 unlit buttons: transport, Back, banks, Preset, DAW,
        // Scene 1 and 2, Shift and the nine mode buttons. Found by its DAW
        // (`Mackie/HUI`, Windows interface 3) port name, never the keybed's;
        // the USB id (Ableton's controller id) is shown only.
        // Dials 0-7 are the knobs, 8-15 the faders (not motorised), 16 the
        // master fader, 17 the main encoder (with its push), none lit.
        product_ids: &[0x0039],
        more_usb_ids: &[],
        protocol: Protocol::MAudioOxygenProMidi,
        rows: maudio_oxygen_pro::ROWS,
        cols: maudio_oxygen_pro::COLS,
        keys: maudio_oxygen_pro::KEYS,
        dials: maudio_oxygen_pro::DIALS_FADERS,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: maudio_oxygen_pro::OXYGEN_PRO_61_ID,
        name: "M-Audio Oxygen Pro 61",
        family: Family::MAudio,
        vendor_id: MAUDIO_VENDOR_ID,
        // A logical 6 x 8 grid: the 16 pads (top row, then bottom row) and
        // the 8 track buttons, all lit (pads by palette, track buttons on or
        // off), then 24 unlit buttons: transport, Back, banks, Preset, DAW,
        // Scene 1 and 2, Shift and the nine mode buttons. Found by its DAW
        // (`Mackie/HUI`, Windows interface 3) port name, never the keybed's;
        // the USB id (Ableton's controller id) is shown only.
        // Dials 0-7 are the knobs, 8-15 the faders (not motorised), 16 the
        // master fader, 17 the main encoder (with its push), none lit.
        product_ids: &[0x003a],
        more_usb_ids: &[],
        protocol: Protocol::MAudioOxygenProMidi,
        rows: maudio_oxygen_pro::ROWS,
        cols: maudio_oxygen_pro::COLS,
        keys: maudio_oxygen_pro::KEYS,
        dials: maudio_oxygen_pro::DIALS_FADERS,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: maudio_oxygen_pro::HAMMER_88_PRO_ID,
        name: "M-Audio Hammer 88 Pro",
        family: Family::MAudio,
        vendor_id: MAUDIO_VENDOR_ID,
        // A logical 6 x 8 grid: the 16 pads (top row, then bottom row) and
        // the 8 track buttons, all lit (pads by palette, track buttons on or
        // off), then 24 unlit buttons: transport, Back, banks, Preset, DAW,
        // Scene 1 and 2, Shift and the nine mode buttons. Laid out like the Oxygen Pro 61. Found by its DAW
        // (`Mackie/HUI`, Windows interface 3) port name, never the keybed's;
        // the USB id (Ableton's controller id) is shown only.
        // Dials 0-7 are the knobs, 8-15 the faders (not motorised), 16 the
        // master fader, 17 the main encoder (with its push), none lit.
        product_ids: &[0x003c],
        more_usb_ids: &[],
        protocol: Protocol::MAudioOxygenProMidi,
        rows: maudio_oxygen_pro::ROWS,
        cols: maudio_oxygen_pro::COLS,
        keys: maudio_oxygen_pro::KEYS,
        dials: maudio_oxygen_pro::DIALS_FADERS,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: maudio_oxygen_pro::OXYGEN_PRO_MINI_ID,
        name: "M-Audio Oxygen Pro Mini",
        family: Family::MAudio,
        vendor_id: MAUDIO_VENDOR_ID,
        // A logical 9 x 4 grid: the 8 pads (top row, then bottom row) and
        // the 4 track buttons, all lit, then 22 unlit buttons (no Scene
        // buttons, per Bitwig). Dials 0-3 are the knobs, 4 the main encoder;
        // the fader is not a dial. Found by its DAW port name only; the USB
        // id (Ableton's controller id) is shown only.
        product_ids: &[0x103b],
        more_usb_ids: &[],
        protocol: Protocol::MAudioOxygenProMidi,
        rows: maudio_oxygen_pro::MINI_ROWS,
        cols: maudio_oxygen_pro::MINI_COLS,
        keys: maudio_oxygen_pro::MINI_KEYS,
        dials: maudio_oxygen_pro::MINI_DIALS,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: icon_platform::PLATFORM_M_PLUS_ID,
        name: "iCON Platform M+",
        family: Family::Icon,
        // No reference read gives iCON's USB vendor or product id, so none is
        // shown (vendor 0000).
        vendor_id: 0,
        product_ids: &[],
        more_usb_ids: &[],
        protocol: Protocol::IconPlatformMMidi,
        // Columns 1-8 are the strips, top down Select, Mute, Solo, Rec and
        // the fader touch; column 9 is Mixer, Read, Write, Cycle and the
        // master fader touch; the bottom row is Channel < >, Bank < > and the
        // transport. Every key but the touches has an on/off light. Dials 1-8
        // are the V-Pots (their pushes press them), 9 the jog wheel and 10-18
        // the motor faders, with no light; Ryu never moves a fader. Mackie
        // Control mode only. Found by MIDI port name.
        rows: icon_platform::ROWS,
        cols: icon_platform::COLS,
        keys: icon_platform::KEYS,
        dials: icon_platform::DIALS,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: icon_platform::PLATFORM_M_ID,
        name: "iCON Platform M",
        family: Family::Icon,
        // No reference read gives iCON's USB vendor or product id, so none is
        // shown (vendor 0000).
        vendor_id: 0,
        product_ids: &[],
        more_usb_ids: &[],
        protocol: Protocol::IconPlatformMMidi,
        // Columns 1-8 are the strips, top down Select, Mute, Solo, Rec and
        // the fader touch; column 9 is Mixer, Read, Write, Cycle and the
        // master fader touch; the bottom row is Channel < >, Bank < > and the
        // transport. Every key but the touches has an on/off light. Dials 1-8
        // are the V-Pots (their pushes press them), 9 the jog wheel and 10-18
        // the motor faders, with no light; Ryu never moves a fader. Mackie
        // Control mode only. Found by MIDI port name.
        // The layout is inferred from the M+'s.
        rows: icon_platform::ROWS,
        cols: icon_platform::COLS,
        keys: icon_platform::KEYS,
        dials: icon_platform::DIALS,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: icon_platform_x::PLATFORM_X_PLUS_ID,
        name: "iCON Platform X+",
        family: Family::Icon,
        // No reference read gives iCON's USB vendor or product id, so none is
        // shown (vendor 0000).
        vendor_id: 0,
        product_ids: &[],
        more_usb_ids: &[],
        protocol: Protocol::IconPlatformXPlusMidi,
        // The Mackie Control extender: a column per strip, top down the
        // V-Pot push (no light), Select, Mute, Solo, Rec (on/off lights) and
        // the fader touch (no light). Dials 1-8 are the V-Pots and 9-16 the
        // motor faders, with no light; Ryu never moves a fader. Mackie
        // Control mode only. Found by MIDI port name (`Platform X+<unit>`);
        // each unit is its own deck.
        rows: icon_platform_x::ROWS,
        cols: icon_platform_x::COLS,
        keys: icon_platform_x::KEYS,
        dials: icon_platform_x::DIALS,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: icon_qcon::PRO_X_ID,
        name: "iCON QCon Pro X",
        family: Family::Icon,
        // No reference read gives iCON's USB vendor or product id, so none is
        // shown (vendor 0000).
        vendor_id: 0,
        product_ids: &[],
        more_usb_ids: &[],
        protocol: Protocol::IconQconProMidi,
        // A key's index is its Mackie note, eight to a row: Rec, Solo, Mute,
        // Select and V-Pot push by strip, then the 64 global buttons in note
        // order (Track to User B), the fader touches and the master touch.
        // Where the globals sit and what iCON prints on them is not in any
        // reference, so they carry the Mackie names. Rec to Select and Track
        // to Scrub have on/off lights. Dials 1-8 are the V-Pots, 9 the jog
        // wheel and 10-18 the motor faders (master last), with no light; Ryu
        // never moves a fader. The main LCD shows the Select and V-Pot push
        // labels. Mackie Control mode only. Found by MIDI port name.
        rows: icon_qcon::PRO_X_ROWS,
        cols: icon_qcon::PRO_X_COLS,
        keys: icon_qcon::PRO_X_KEYS,
        dials: icon_qcon::PRO_X_DIALS,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: icon_qcon::PRO_XS1_ID,
        name: "iCON QCon Pro XS1",
        family: Family::Icon,
        // No reference read gives iCON's USB vendor or product id, so none is
        // shown (vendor 0000).
        vendor_id: 0,
        product_ids: &[],
        more_usb_ids: &[],
        protocol: Protocol::IconQconProMidi,
        // The extender, its own deck: rows 1-5 are Rec, Solo, Mute, Select
        // and V-Pot push by strip (notes 0x00-0x27), row 6 the fader touches.
        // Rec to Select have on/off lights. Dials 1-8 are the V-Pots and 9-16
        // the motor faders, with no light; Ryu never moves a fader. The LCD
        // shows the Select and V-Pot push labels. Mackie Control mode only.
        // Found by MIDI port name.
        rows: icon_qcon::XS1_ROWS,
        cols: icon_qcon::XS1_COLS,
        keys: icon_qcon::XS1_KEYS,
        dials: icon_qcon::XS1_DIALS,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: icon_qcon::PRO_G2_ID,
        name: "iCON QCon Pro G2",
        family: Family::Icon,
        // No reference read gives iCON's USB vendor or product id, so none is
        // shown (vendor 0000).
        vendor_id: 0,
        product_ids: &[],
        more_usb_ids: &[],
        protocol: Protocol::IconQconG2Midi,
        // Keys 1-102 are the Mackie notes 0x00-0x65, eight to a row: Rec,
        // Solo, Mute, Select and V-Pot push by strip, then the global buttons
        // in note order (Track to Scrub); keys 103-111 the fader touches and
        // master touch. iCON prints other legends on many globals (and the
        // references disagree on them), so the keys carry the Mackie names.
        // Rec to Select and Track to Scrub have on/off lights. Dials 1-8 are
        // the V-Pots (ring centre dot on or off), 9 the jog wheel and 10-18
        // the motor faders (master last), with no light; Ryu never moves a
        // fader. The LCD shows the Select and V-Pot push labels. Mackie
        // Control mode only. Found by MIDI port name.
        rows: icon_qcon::PRO_G2_ROWS,
        cols: icon_qcon::PRO_G2_COLS,
        keys: icon_qcon::PRO_G2_KEYS,
        dials: icon_qcon::PRO_G2_DIALS,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::PerDial,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: icon_qcon::EX_G2_ID,
        name: "iCON QCon EX G2",
        family: Family::Icon,
        // No reference read gives iCON's USB vendor or product id, so none is
        // shown (vendor 0000).
        vendor_id: 0,
        product_ids: &[],
        more_usb_ids: &[],
        protocol: Protocol::IconQconG2Midi,
        // The extender, its own deck: rows 1-5 are Rec, Solo, Mute, Select
        // and V-Pot push by strip (notes 0x00-0x27), row 6 the fader touches.
        // Rec to Select have on/off lights. Dials 1-8 are the V-Pots (ring
        // centre dot on or off) and 9-16 the motor faders, with no light; Ryu
        // never moves a fader. The LCD shows the Select and V-Pot push
        // labels. Mackie Control mode only. Found by MIDI port name.
        rows: icon_qcon::EX_G2_ROWS,
        cols: icon_qcon::EX_G2_COLS,
        keys: icon_qcon::EX_G2_KEYS,
        dials: icon_qcon::EX_G2_DIALS,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::PerDial,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: icon_p1::NANO_ID,
        name: "iCON P1-Nano",
        family: Family::Icon,
        // iCON's vendor id and the P1-Nano's product id as Mixxx's mapping
        // gives them, shown only.
        vendor_id: ICON_VENDOR_ID,
        product_ids: &[icon_p1::NANO_PRODUCT_ID],
        more_usb_ids: &[],
        protocol: Protocol::IconP1Midi,
        // A key's index is its Mackie note, eight to a row: Rec, Solo, Mute,
        // Select and V-Pot push by strip, then the global buttons in note
        // order (Track to User B), the fader touches and the master touch.
        // Which globals the P1-Nano has is not in any reference, so they
        // carry the Mackie names and an absent one never fires. Rec to Select
        // and Track to Scrub have on/off lights. Dials 1-8 are the V-Pots,
        // each lit by the RGB backlight of the scribble strip above it, 9 the
        // jog wheel and 10 the motor fader, with no light; Ryu never moves
        // the fader. The LCD shows the Select and V-Pot push labels. Mackie
        // Control mode only. Found by MIDI port name.
        rows: icon_p1::ROWS,
        cols: icon_p1::COLS,
        keys: icon_p1::KEYS,
        dials: icon_p1::DIALS,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::PerDial,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: zoom::ZOOM_R16_R24_ID,
        name: "Zoom R16 / R24",
        family: Family::Zoom,
        // The R16 and R24 share this id ("ZOOM R16/24 in audio interface
        // mode") and their MIDI port name, so one row stands for both. Shown
        // only: found by MIDI port name.
        vendor_id: ZOOM_VENDOR_ID,
        product_ids: &[0x00dd],
        more_usb_ids: &[],
        protocol: Protocol::ZoomR16Midi,
        // Placed by `zoom::key_cells` on a 3 x 9 grid: the eight track status
        // keys on top (their lights, on or off, are unconfirmed), then F1-F5
        // (Auto Punch I/O to Mark) and the 1-8Tr and 9-16Tr bank keys, then
        // the transport and the cursor keys, which have no light. Dials 1-8
        // are the track faders, 9 the master fader (no motors) and 10 the jog
        // dial, none with a light. Audio interface mode, Mackie Control.
        rows: zoom::ROWS,
        cols: zoom::COLS,
        keys: zoom::KEYS,
        dials: zoom::DIALS,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: mackie_mcu::MCU_PRO_ID,
        name: "Mackie MCU Pro",
        family: Family::Mackie,
        // Mackie Designs' vendor id and the MCU Pro's product id as Ableton
        // Live's script gives them, shown only: found by MIDI port name.
        vendor_id: MACKIE_VENDOR_ID,
        product_ids: &[mackie_mcu::MCU_PRO_PRODUCT_ID],
        more_usb_ids: &[],
        protocol: Protocol::MackieMcuProMidi,
        // A key's index is its Mackie note, eight to a row: Rec, Solo, Mute,
        // Select and V-Pot push by strip, then the global buttons in note
        // order (Track to User Switch B), the fader touches and the master
        // touch. Rec to Select and Track to Scrub have on/off lights. Dials
        // 1-8 are the V-Pots (their LED rings are the dial lights), 9 the
        // jog wheel and 10-18 the motor faders (master last), with no light;
        // Ryu never moves a fader. The LCD shows "Ryu". Mackie Control (or
        // Logic Control) mode only; the first of its four MIDI ports.
        rows: mackie_mcu::MCU_ROWS,
        cols: mackie_mcu::MCU_COLS,
        keys: mackie_mcu::MCU_KEYS,
        dials: mackie_mcu::MCU_DIALS,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::PerDial,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
    DeviceModel {
        id: mackie_mcu::XT_PRO_ID,
        name: "Mackie XT Pro",
        family: Family::Mackie,
        // No reference read gives the XT Pro's own USB id or port name, so
        // none is shown (vendor 0000): it is found only on an MCU Pro's
        // extra MIDI ports, chained to the MCU Pro.
        vendor_id: 0,
        product_ids: &[],
        more_usb_ids: &[],
        protocol: Protocol::MackieXtProMidi,
        // The extender, its own deck: rows 1-5 are Rec, Solo, Mute, Select
        // and V-Pot push by strip (notes 0x00-0x27), row 6 the fader touches.
        // Rec to Select have on/off lights. Dials 1-8 are the V-Pots (LED
        // rings as dial lights) and 9-16 the motor faders, with no light;
        // Ryu never moves a fader. The LCD shows "Ryu".
        rows: mackie_mcu::XT_ROWS,
        cols: mackie_mcu::XT_COLS,
        keys: mackie_mcu::XT_KEYS,
        dials: mackie_mcu::XT_DIALS,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::PerDial,
        deck_lights: 0,
        key_lights: true,
        usage_page: None,
    },
];

impl DeviceModel {
    /// How many dial colours this model keeps: one per lit dial, or one
    /// shared. The X-Touch Mini's fader, its last dial, has no light, nor do
    /// the faders after the knobs of a Launch Control XL (MK1/MK2 or 3).
    #[must_use]
    pub fn dial_light_colors(&self) -> u8 {
        let lit = match self.protocol {
            Protocol::BehringerXTouchMiniMidi => self.dials.min(xtouch_mini::ENCODERS),
            Protocol::IconQconG2Midi => self.dials.min(icon_qcon::G2_RINGS),
            Protocol::IconP1Midi => self.dials.min(icon_p1::STRIP_LIGHTS),
            Protocol::MackieMcuProMidi | Protocol::MackieXtProMidi => {
                self.dials.min(mackie_mcu::RINGS)
            }
            Protocol::NovationLaunchControlXl => self
                .dials
                .min(u8::try_from(launch_control::XL_DIAL_LIGHTS).unwrap_or(u8::MAX)),
            Protocol::NovationLaunchControlXl3 => {
                self.dials.saturating_sub(launch_control3::XL3_FADERS)
            }
            _ => self.dials,
        };
        self.dial_lights.colors(lit)
    }

    /// The HID interface this model's deck is driven through, where its
    /// protocol names one: the wired G915 lists HID++ on interface 2, not on
    /// interface 1 like the other `PerKeyLighting` keyboards.
    #[must_use]
    pub fn hid_interface(&self) -> Option<HidInterface> {
        if self.id == "logitech-g915" {
            return Some(LOGITECH_G915_HIDPP);
        }
        self.protocol.hid_interface()
    }

    /// Whether this model answers to the given USB ids, under any of its brands.
    #[must_use]
    pub fn matches_usb(&self, vendor_id: u16, product_id: u16) -> bool {
        (self.vendor_id == vendor_id && self.product_ids.contains(&product_id))
            || self.more_usb_ids.contains(&(vendor_id, product_id))
    }

    /// Keys numbered from here on are extra keys, with no screen unless
    /// `extra_image` gives them one.
    #[must_use]
    pub const fn screen_keys(&self) -> u8 {
        if self.image.is_some() {
            self.keys - self.extra_keys
        } else {
            0
        }
    }

    /// Every bindable key: the grid, the strip segments, the wheel, then the info
    /// screen.
    #[must_use]
    pub const fn total_keys(&self) -> u8 {
        self.keys
            + self.strip_keys
            + if self.wheel_image.is_some() { 1 } else { 0 }
            + if self.info_image.is_some() { 1 } else { 0 }
    }

    /// The info screen's key, on a deck that has one.
    #[must_use]
    pub const fn info_key(&self) -> Option<u8> {
        if self.info_image.is_some() {
            Some(self.total_keys() - 1)
        } else {
            None
        }
    }

    /// Whether `key` is an extra key with a screen of its own.
    #[must_use]
    pub const fn extra_has_screen(&self, key: u8) -> bool {
        self.extra_image.is_some() && key >= self.keys - self.extra_keys && key < self.keys
    }

    /// The wheel screen's key, on a deck that has one.
    #[must_use]
    pub const fn wheel_key(&self) -> Option<u8> {
        if self.wheel_image.is_some() {
            Some(self.keys + self.strip_keys)
        } else {
            None
        }
    }

    /// The baud rate this deck's serial port is opened at.
    #[must_use]
    pub fn serial_baud(&self) -> u32 {
        if self.id == monome::ARDUINOME_ID {
            monome::ARDUINOME_BAUD
        } else {
            self.protocol.serial_baud()
        }
    }

    /// How this MIDI deck's port is found by name.
    #[must_use]
    pub fn midi_port(&self) -> Option<midi::PortRule> {
        launchpad::port_rule(self)
    }

    /// Which strip segment a key is, if it is one.
    #[must_use]
    pub fn strip_segment(&self, key: u8) -> Option<u8> {
        key.checked_sub(self.keys)
            .filter(|segment| *segment < self.strip_keys)
    }
}

/// The model a HID collection belongs to, if this app can drive it.
#[must_use]
pub fn identify(vendor_id: u16, product_id: u16, usage_page: u16) -> Option<&'static DeviceModel> {
    MODELS.iter().find(|model| {
        !model.protocol.is_serial()
            && !model.protocol.is_midi()
            && model.matches_usb(vendor_id, product_id)
            && model.usage_page.is_none_or(|page| page == usage_page)
            && model.protocol.claims_usage_page(usage_page)
            && (!model.protocol.needs_vendor_collection() || usage_page >= VENDOR_USAGE_PAGES)
    })
}

/// Whether a listed deck on these USB ids is driven through collections
/// paired by usage page (a Wooting keyboard), so none of its other
/// collections (its Dynamic Lighting one) is claimed as a deck of its own.
#[must_use]
pub fn driven_by_paired_collections(vendor_id: u16, product_id: u16) -> bool {
    MODELS.iter().any(|model| {
        model.protocol.paired_collections().is_some() && model.matches_usb(vendor_id, product_id)
    })
}

/// The model a USB serial port belongs to, if this app can drive it. monome
/// grids and arcs are found by their USB strings instead ([`monome::identify`]).
#[must_use]
pub fn identify_serial(vendor_id: u16, product_id: u16) -> Option<&'static DeviceModel> {
    MODELS
        .iter()
        .find(|model| model.protocol.is_serial() && model.matches_usb(vendor_id, product_id))
}

/// The model a Linux input event node belongs to, for a deck the kernel keeps
/// from hidraw ([`Protocol::linux_evdev`]).
#[must_use]
pub fn identify_evdev(vendor_id: u16, product_id: u16) -> Option<&'static DeviceModel> {
    MODELS
        .iter()
        .find(|model| model.protocol.linux_evdev() && model.matches_usb(vendor_id, product_id))
}

/// The model a MIDI port belongs to, by its normalised name. `windows` says
/// the name follows Windows' scheme, where the interface number decides.
#[must_use]
pub fn identify_midi(name: &midi::PortName, windows: bool) -> Option<&'static DeviceModel> {
    MODELS.iter().find(|model| {
        model.protocol.is_midi()
            && (model
                .midi_port()
                .is_some_and(|rule| midi::matches(rule, name, windows))
                || launchpad_legacy::port_matches(model, name)
                || launchpad_mk2::port_matches(model, name)
                || launchpad_pro::port_matches(model, name)
                || novation_dicer::port_matches(model, name)
                || launch_control::port_matches(model, name)
                || launch_control3::port_matches(model, name)
                || launchkey::port_matches(model, name)
                || launchkey_mk4::port_matches(model, name)
                || launchkey_mk2::port_matches(model, name)
                || launchkey_mini::port_matches(model, name)
                || sl_mkiii::port_matches(model, name)
                || akai_apc::port_matches(model, name)
                || akai_apc_mini::port_matches(model, name)
                || akai_midimix::port_matches(model, name)
                || akai_lpd8::port_matches(model, name)
                || akai_fire::port_matches(model, name)
                || akai_mpk_mk4::port_matches(model, name)
                || akai_mpk_mini_plus::port_matches(model, name)
                || akai_apc40::port_matches(model, name)
                || akai_apc40_mk1::port_matches(model, name)
                || akai_apc_key25::port_matches(model, name)
                || push1::port_matches(model, name)
                || push2::port_matches(model, name)
                || xtouch_mini::port_matches(model, name)
                || xtouch_one::port_matches(model, name)
                || korg::port_matches(model, name)
                || korg_padkontrol::port_matches(model, name)
                || arturia_beatstep::port_matches(model, name)
                || arturia_minilab3::port_matches(model, name)
                || arturia_keylab_essential3::port_matches(model, name)
                || midi_fighter_twister::port_matches(model, name)
                || midi_fighter_spectra::port_matches(model, name)
                || midi_fighter_64::port_matches(model, name)
                || intech_grid::port_matches(model, name)
                || presonus_atom::port_matches(model, name)
                || presonus_atom_sq::port_matches(model, name)
                || faderport::port_matches(model, name)
                || faderport8::port_matches(model, name)
                || faderfox_ec4::port_matches(model, name)
                || livid_base::port_matches(model, name)
                || xone_k2::port_matches(model, name)
                || exquis::port_matches(model, name)
                || maudio_oxygen_pro::port_matches(model, name)
                || icon_platform::port_matches(model, name)
                || icon_platform_x::port_matches(model, name)
                || icon_qcon::port_matches(model, name)
                || icon_p1::port_matches(model, name)
                || zoom::port_matches(model, name)
                || mackie_mcu::port_matches(model, name)
                || behringer_bcontrol::port_matches(model, name))
    })
}

/// The listed model with this id, or a Vial board's or LampArray's model
/// built this run.
#[must_use]
pub fn by_id(id: &str) -> Option<&'static DeviceModel> {
    MODELS.iter().find(|model| model.id == id).or_else(|| {
        vial::models()
            .into_iter()
            .chain(lamparray::models())
            .rev()
            .find(|model| model.id == id)
    })
}

/// The id a device is known by in the API and the store: the model id, plus the
/// serial number when the device reports one, so two identical decks keep separate
/// layouts. Only `[a-z0-9-]` survives, which keeps it a single URL path segment.
#[must_use]
pub fn device_id(model: &DeviceModel, serial: Option<&str>) -> String {
    let serial: String = serial
        .unwrap_or_default()
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .take(48)
        .collect();
    if serial.is_empty() {
        model.id.to_owned()
    } else {
        format!("{}-{serial}", model.id)
    }
}

/// The serial number a deck is told apart by: `None` when the model's serial
/// says nothing (all zeros on the Galleon 100 SD, the same on every SurfaceCore
/// board), so two such decks fall back to their paths.
#[must_use]
pub fn usable_serial<'a>(model: &DeviceModel, serial: Option<&'a str>) -> Option<&'a str> {
    serial.filter(|serial| {
        !model.protocol.serial_is_shared()
            && !(model.protocol.zero_serial_is_none()
                && !serial.is_empty()
                && serial.bytes().all(|byte| byte == b'0'))
    })
}

/// The model a stored device id belongs to. Model ids never contain a serial, and
/// the longest matching prefix wins, so `elgato-mini` never claims an
/// `elgato-mini-...` device that belongs to a longer id.
/// A Vial board's or LampArray's model is known only once the device has been
/// seen this run; the latest one built wins a tie.
#[must_use]
pub fn model_for_device_id(device_id: &str) -> Option<&'static DeviceModel> {
    MODELS
        .iter()
        .chain(vial::models())
        .chain(lamparray::models())
        .filter(|model| {
            device_id == model.id
                || device_id
                    .strip_prefix(model.id)
                    .is_some_and(|rest| rest.starts_with('-'))
        })
        .max_by_key(|model| model.id.len())
}

#[derive(Debug, Serialize)]
pub struct ModelView {
    pub id: &'static str,
    pub name: &'static str,
    pub family: Family,
    pub vendor_id: String,
    pub product_ids: Vec<String>,
    /// Every `vvvv:pppp` pair the model answers to, rebrands included.
    pub usb_ids: Vec<String>,
    pub rows: u8,
    pub cols: u8,
    pub keys: u8,
    pub dials: u8,
    pub extra_keys: u8,
    pub key_screens: bool,
    pub key_lights: bool,
    pub face_size: Option<u32>,
    /// Width and height of a grid key's face (not square on the Studio).
    pub key_face: Option<[u32; 2]>,
    pub strip_keys: u8,
    /// Width and height of one strip segment's face.
    pub strip_face: Option<[u32; 2]>,
    pub strip_placement: StripPlacement,
    /// Side of the round wheel screen's face, on a deck that has one.
    pub wheel_face: Option<u32>,
    /// Width and height of an extra key's own screen, on decks whose extra keys
    /// have one.
    pub extra_face: Option<[u32; 2]>,
    /// Width and height of the info screen's face, on a deck that has one.
    pub info_face: Option<[u32; 2]>,
    /// What a key light can show: any colour, or only blue, red or both (the
    /// legacy X-keys backlights), on or off, eight colours, or the nearest of
    /// a palette of ten (the Yamaha CC121MK2), or off, green, red or yellow
    /// (`green_red_yellow`: the first APC mini's pads, whose scene and track
    /// buttons are only on or off), or off, dim or bright green or orange
    /// (`green_orange`: the Novation Dicer).
    pub key_light_colors: &'static str,
    /// The keys that have a light, when only some do (`None`: every key that
    /// has no screen, on a deck with key lights).
    pub lit_keys: Option<Vec<u8>>,
    /// The legend on each key, on decks whose keys are printed.
    pub key_names: Option<Vec<&'static str>>,
    /// Each key's cell in the `rows` x `cols` grid, on decks whose keys do not
    /// simply fill it in reading order.
    pub key_cells: Option<Vec<u16>>,
    pub dial_lights: DialLights,
    pub deck_lights: u8,
    /// Whether the Companion offers a brightness control.
    pub brightness: bool,
}

impl From<&DeviceModel> for ModelView {
    fn from(model: &DeviceModel) -> Self {
        Self {
            id: model.id,
            name: model.name,
            family: model.family,
            vendor_id: format!("{:04x}", model.vendor_id),
            product_ids: model
                .product_ids
                .iter()
                .map(|pid| format!("{pid:04x}"))
                .collect(),
            usb_ids: model
                .product_ids
                .iter()
                .map(|pid| (model.vendor_id, *pid))
                .chain(model.more_usb_ids.iter().copied())
                .map(|(vid, pid)| format!("{vid:04x}:{pid:04x}"))
                .collect(),
            rows: model.rows,
            cols: model.cols,
            keys: model.keys,
            dials: model.dials,
            extra_keys: model.extra_keys,
            key_screens: model.image.is_some(),
            key_lights: model.key_lights,
            face_size: model.image.map(|image| image.width),
            key_face: model.image.map(|image| [image.width, image.height]),
            strip_keys: model.strip_keys,
            strip_face: model.strip_image.map(|image| [image.width, image.height]),
            strip_placement: model.strip_placement,
            wheel_face: model.wheel_image.map(|image| image.width),
            extra_face: model.extra_image.map(|image| [image.width, image.height]),
            info_face: model.info_image.map(|image| [image.width, image.height]),
            dial_lights: model.dial_lights,
            deck_lights: model.deck_lights,
            brightness: (model.image.is_some()
                || model.key_lights
                || model.protocol.shows_text_labels()
                || model.protocol.dims_its_lights()
                    && (model.dial_lights != DialLights::None || model.deck_lights > 0))
                && model.protocol.has_brightness(),
            key_light_colors: match model.id {
                _ if matches!(
                    model.protocol,
                    Protocol::XKeysLegacy | Protocol::ArturiaBeatStepMidi
                ) =>
                {
                    "blue_red"
                }
                "blackmagic-atem-micro-panel" => "basic_rgb",
                _ if model.protocol == Protocol::LividBaseMidi => "basic_rgb",
                _ if model.protocol == Protocol::BlackmagicPanel => "on_off",
                _ if model.protocol == Protocol::LogitechG13 => "on_off",
                _ if model.protocol == Protocol::LogitechG15 => "on_off",
                _ if model.protocol == Protocol::YamahaCc1Serial => "palette",
                _ if model.protocol == Protocol::MonomeMext => "levels",
                _ if matches!(model.protocol, Protocol::MonomeSeries | Protocol::Monome40h) => {
                    "on_off"
                }
                _ if model.protocol == Protocol::SonyBuzz => "on_off",
                _ if matches!(
                    model.protocol,
                    Protocol::AkaiMidimixMidi
                        | Protocol::AkaiLpd8Midi
                        | Protocol::AkaiMpkMiniPlusMidi
                        | Protocol::BehringerXTouchMiniMidi
                        | Protocol::BehringerXTouchOneMidi
                        | Protocol::IconPlatformMMidi
                        | Protocol::IconPlatformXPlusMidi
                        | Protocol::IconQconProMidi
                        | Protocol::IconQconG2Midi
                        | Protocol::IconP1Midi
                        | Protocol::ZoomR16Midi
                        | Protocol::MackieMcuProMidi
                        | Protocol::MackieXtProMidi
                        | Protocol::PresonusFaderPortMidi
                        | Protocol::KorgNanoKontrol2
                        | Protocol::KorgNanoKontrolStudio
                        | Protocol::KorgPadKontrol
                ) =>
                {
                    "on_off"
                }
                _ if matches!(
                    model.protocol,
                    Protocol::AkaiApcMiniMidi
                        | Protocol::AkaiApcKey25Midi
                        | Protocol::AkaiApc40Mk1Midi
                        | Protocol::AkaiApc20Midi
                        | Protocol::AllenHeathXoneK2Midi
                ) =>
                {
                    "green_red_yellow"
                }
                _ if model.protocol == Protocol::NovationDicerMidi => "green_orange",
                _ if matches!(
                    model.protocol,
                    Protocol::LaunchpadLegacyMidi
                        | Protocol::LaunchkeyMiniMidi
                        | Protocol::NovationLaunchControlXl
                        | Protocol::NovationLaunchControl
                ) =>
                {
                    "red_green"
                }
                _ => "rgb",
            },
            lit_keys: blackmagic::panel(model.id)
                .and_then(|panel| {
                    let lit: Vec<u8> = (0..model.keys)
                        .zip(panel.keys)
                        .filter(|(_, key)| key.led != blackmagic::Led::None)
                        .map(|(index, _)| index)
                        .collect();
                    (lit.len() < usize::from(model.keys)).then_some(lit)
                })
                .or_else(|| {
                    (model.protocol == Protocol::LogitechG13).then(|| g13::LIT_KEYS.to_vec())
                })
                .or_else(|| {
                    (model.protocol == Protocol::YamahaCc1Serial).then(|| yamaha::LIT_KEYS.to_vec())
                })
                .or_else(|| {
                    (model.protocol == Protocol::SurfaceCoreHid)
                        .then(|| surfacecore::LIT_KEYS.to_vec())
                })
                .or_else(|| (model.protocol == Protocol::SonyBuzz).then(|| buzz::LIT_KEYS.to_vec()))
                .or_else(|| g15::lit_keys(model))
                .or_else(|| maschine::lit_keys(model))
                .or_else(|| launchkey::lit_keys(model))
                .or_else(|| launchkey_mk4::lit_keys(model))
                .or_else(|| launchkey_mk2::lit_keys(model))
                .or_else(|| launchkey_mini::lit_keys(model))
                .or_else(|| launch_control3::lit_keys(model))
                .or_else(|| sl_mkiii::lit_keys(model))
                .or_else(|| akai_apc::lit_keys(model))
                .or_else(|| akai_apc40::lit_keys(model))
                .or_else(|| akai_apc40_mk1::lit_keys(model))
                .or_else(|| akai_apc_key25::lit_keys(model))
                .or_else(|| akai_midimix::lit_keys(model))
                .or_else(|| akai_fire::lit_keys(model))
                .or_else(|| xtouch_one::lit_keys(model))
                .or_else(|| icon_platform::lit_keys(model))
                .or_else(|| icon_platform_x::lit_keys(model))
                .or_else(|| icon_qcon::lit_keys(model))
                .or_else(|| icon_p1::lit_keys(model))
                .or_else(|| zoom::lit_keys(model))
                .or_else(|| mackie_mcu::lit_keys(model))
                .or_else(|| korg::lit_keys(model))
                .or_else(|| korg_padkontrol::lit_keys(model))
                .or_else(|| maudio_oxygen_pro::lit_keys(model))
                .or_else(|| arturia_beatstep::lit_keys(model))
                .or_else(|| arturia_minilab3::lit_keys(model))
                .or_else(|| akai_mpk_mk4::lit_keys(model))
                .or_else(|| arturia_keylab_essential3::lit_keys(model))
                .or_else(|| faderport::lit_keys(model))
                .or_else(|| faderport8::lit_keys(model))
                .or_else(|| vial::lit_keys(model)),
            key_names: blackmagic::panel(model.id)
                .map(|panel| panel.keys.iter().map(|key| key.name).collect())
                .or_else(|| {
                    (model.protocol == Protocol::TourBoxSerial).then(|| tourbox::KEY_NAMES.to_vec())
                })
                .or_else(|| {
                    (model.protocol == Protocol::SpaceMouseHid && model.keys > 0)
                        .then(|| spacemouse::key_names(model.id).to_vec())
                })
                .or_else(|| {
                    (model.protocol == Protocol::LogitechG13).then(|| g13::KEY_NAMES.to_vec())
                })
                .or_else(|| {
                    (model.protocol == Protocol::YamahaCc1Serial)
                        .then(|| yamaha::KEY_NAMES.to_vec())
                })
                .or_else(|| (model.protocol == Protocol::XboxGip).then(|| xbox::KEY_NAMES.to_vec()))
                .or_else(|| playstation::key_names(model))
                .or_else(|| {
                    (model.protocol == Protocol::GameSirConfig).then(|| gamesir::KEY_NAMES.to_vec())
                })
                .or_else(|| {
                    (model.protocol == Protocol::SonyBuzz).then(|| buzz::KEY_NAMES.to_vec())
                })
                .or_else(|| (model.protocol == Protocol::DjiDuml).then(|| dji::KEY_NAMES.to_vec()))
                .or_else(|| {
                    (model.protocol == Protocol::SurfaceCoreHid)
                        .then(|| surfacecore::KEY_NAMES.to_vec())
                })
                .or_else(|| {
                    (model.protocol == Protocol::PrehPosKey).then(|| prehkeytec::key_names(model))
                })
                .or_else(|| g15::key_names(model))
                .or_else(|| logitech_rgb::key_names(model))
                .or_else(|| maschine::key_names(model))
                .or_else(|| launchpad::key_names(model))
                .or_else(|| launchpad_legacy::key_names(model))
                .or_else(|| launchpad_mk2::key_names(model))
                .or_else(|| launchpad_pro::key_names(model))
                .or_else(|| novation_dicer::key_names(model))
                .or_else(|| launch_control::key_names(model))
                .or_else(|| launch_control3::key_names(model))
                .or_else(|| launchkey::key_names(model))
                .or_else(|| launchkey_mk4::key_names(model))
                .or_else(|| launchkey_mk2::key_names(model))
                .or_else(|| launchkey_mini::key_names(model))
                .or_else(|| sl_mkiii::key_names(model))
                .or_else(|| akai_apc::key_names(model))
                .or_else(|| akai_apc_mini::key_names(model))
                .or_else(|| akai_midimix::key_names(model))
                .or_else(|| akai_lpd8::key_names(model))
                .or_else(|| akai_fire::key_names(model))
                .or_else(|| akai_apc40::key_names(model))
                .or_else(|| akai_apc40_mk1::key_names(model))
                .or_else(|| akai_apc_key25::key_names(model))
                .or_else(|| push1::key_names(model))
                .or_else(|| push2::key_names(model))
                .or_else(|| xtouch_mini::key_names(model))
                .or_else(|| xtouch_one::key_names(model))
                .or_else(|| icon_platform::key_names(model))
                .or_else(|| icon_platform_x::key_names(model))
                .or_else(|| icon_qcon::key_names(model))
                .or_else(|| icon_p1::key_names(model))
                .or_else(|| zoom::key_names(model))
                .or_else(|| mackie_mcu::key_names(model))
                .or_else(|| korg::key_names(model))
                .or_else(|| korg_padkontrol::key_names(model))
                .or_else(|| arturia_beatstep::key_names(model))
                .or_else(|| arturia_minilab3::key_names(model))
                .or_else(|| akai_mpk_mk4::key_names(model))
                .or_else(|| akai_mpk_mini_plus::key_names(model))
                .or_else(|| arturia_keylab_essential3::key_names(model))
                .or_else(|| midi_fighter_twister::key_names(model))
                .or_else(|| presonus_atom::key_names(model))
                .or_else(|| presonus_atom_sq::key_names(model))
                .or_else(|| faderport::key_names(model))
                .or_else(|| faderport8::key_names(model))
                .or_else(|| faderfox_ec4::key_names(model))
                .or_else(|| livid_base::key_names(model))
                .or_else(|| xone_k2::key_names(model))
                .or_else(|| exquis::key_names(model))
                .or_else(|| maudio_oxygen_pro::key_names(model))
                .or_else(|| behringer_bcontrol::key_names(model))
                .or_else(|| {
                    (model.protocol == Protocol::TraktorF1Hid).then(|| traktor::KEY_NAMES.to_vec())
                })
                .or_else(|| {
                    (model.protocol == Protocol::KompleteKontrolHid)
                        .then(|| komplete::KEY_NAMES.to_vec())
                })
                .or_else(|| vial::key_names(model))
                .or_else(|| lamparray::key_names(model))
                .or_else(|| wooting::key_names(model)),
            key_cells: blackmagic::panel(model.id)
                .map(|panel| {
                    panel
                        .keys
                        .iter()
                        .map(|key| u16::from(key.row) * u16::from(model.cols) + u16::from(key.col))
                        .collect()
                })
                .or_else(|| {
                    (model.protocol == Protocol::LogitechG13).then(|| {
                        g13::KEY_PLACES
                            .iter()
                            .map(|(row, col)| {
                                u16::from(*row) * u16::from(model.cols) + u16::from(*col)
                            })
                            .collect()
                    })
                })
                .or_else(|| {
                    (model.protocol == Protocol::SurfaceCoreHid).then(|| {
                        surfacecore::KEY_PLACES
                            .iter()
                            .map(|(row, col)| {
                                u16::from(*row) * u16::from(model.cols) + u16::from(*col)
                            })
                            .collect()
                    })
                })
                .or_else(|| prehkeytec::key_cells(model))
                .or_else(|| g15::key_cells(model))
                .or_else(|| maschine::key_cells(model))
                .or_else(|| launchpad::key_cells(model))
                .or_else(|| launchpad_legacy::key_cells(model))
                .or_else(|| launchpad_mk2::key_cells(model))
                .or_else(|| launchpad_pro::key_cells(model))
                .or_else(|| launch_control::key_cells(model))
                .or_else(|| launch_control3::key_cells(model))
                .or_else(|| launchkey::key_cells(model))
                .or_else(|| launchkey_mk4::key_cells(model))
                .or_else(|| launchkey_mk2::key_cells(model))
                .or_else(|| launchkey_mini::key_cells(model))
                .or_else(|| sl_mkiii::key_cells(model))
                .or_else(|| akai_apc::key_cells(model))
                .or_else(|| akai_apc40::key_cells(model))
                .or_else(|| akai_apc40_mk1::key_cells(model))
                .or_else(|| akai_apc_key25::key_cells(model))
                .or_else(|| akai_fire::key_cells(model))
                .or_else(|| push1::key_cells(model))
                .or_else(|| push2::key_cells(model))
                .or_else(|| xtouch_mini::key_cells(model))
                .or_else(|| zoom::key_cells(model))
                .or_else(|| korg::key_cells(model))
                .or_else(|| korg_padkontrol::key_cells(model))
                .or_else(|| arturia_beatstep::key_cells(model))
                .or_else(|| behringer_bcontrol::key_cells(model))
                .or_else(|| xone_k2::key_cells(model))
                .or_else(|| exquis::key_cells(model))
                .or_else(|| {
                    (model.protocol == Protocol::KompleteKontrolHid).then(|| {
                        komplete::KEY_PLACES
                            .iter()
                            .map(|(row, col)| {
                                u16::from(*row) * u16::from(model.cols) + u16::from(*col)
                            })
                            .collect()
                    })
                })
                .or_else(|| {
                    (model.protocol == Protocol::TraktorF1Hid).then(|| {
                        traktor::KEY_PLACES
                            .iter()
                            .map(|(row, col)| {
                                u16::from(*row) * u16::from(model.cols) + u16::from(*col)
                            })
                            .collect()
                    })
                })
                .or_else(|| vial::key_cells(model))
                .or_else(|| lamparray::key_cells(model)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_usb_id_names_one_model() {
        let mut seen = std::collections::HashSet::new();
        for model in MODELS {
            // A MasterHub module may be only dials, or only a light.
            assert!(
                model.total_keys() > 0 || model.dials > 0 || model.deck_lights > 0,
                "{} has nothing to bind or light",
                model.id
            );
            assert!(
                u16::from(model.rows) * u16::from(model.cols) >= u16::from(model.keys),
                "{} layout cannot hold its keys",
                model.id
            );
            assert!(model.extra_keys <= model.keys, "{}", model.id);
            assert_eq!(
                model.strip_keys == 0,
                model.strip_image.is_none(),
                "{} strip keys need a strip image",
                model.id
            );
            // A strip sits by the dials, unless the strip is the whole deck.
            assert!(
                model.keys == 0 || model.strip_keys <= model.dials,
                "{} has more strip segments than dials",
                model.id
            );
            assert!(
                model.strip_placement == StripPlacement::Below || model.strip_keys % 2 == 0,
                "{} splits its side strips unevenly",
                model.id
            );
            let ids = model
                .product_ids
                .iter()
                .map(|pid| (model.vendor_id, *pid))
                .chain(model.more_usb_ids.iter().copied());
            for (vid, pid) in ids {
                assert!(
                    seen.insert((vid, pid, model.usage_page, model.protocol.is_serial())),
                    "duplicate id {vid:04x}:{pid:04x}"
                );
            }
        }
        let ids: std::collections::HashSet<_> = MODELS.iter().map(|m| m.id).collect();
        assert_eq!(ids.len(), MODELS.len(), "model ids must be unique");
    }

    #[test]
    fn identify_respects_the_rpc_collection() {
        assert_eq!(identify(0x0fd9, 0x0080, 0x000c).unwrap().id, "elgato-mk2");
        assert_eq!(
            identify(0x0fd9, 0x00aa, 0x000c).unwrap().id,
            "elgato-studio"
        );
        assert_eq!(identify(0x05f3, 0x04c1, 0x000c).unwrap().id, "xkeys-xk80");
        assert_eq!(
            identify(0x05f3, 0x05dc, 0x000c).unwrap().id,
            "xkeys-xbk-16x6"
        );
        assert_eq!(
            identify(0x0b33, 0x0020, 0x000c).unwrap().id,
            "contour-shuttlexpress"
        );
        assert_eq!(
            identify(0x05f3, 0x0240, 0x000c).unwrap().id,
            "contour-shuttlepro-v1"
        );
        // The MX Creative Console answers only on its vendor (HID++) collection.
        assert_eq!(
            identify(0x046d, 0xc354, 0xff43).unwrap().id,
            "logitech-mx-keypad"
        );
        assert!(identify(0x046d, 0xc354, 0x0001).is_none());
        assert_eq!(
            identify(0x046d, 0xbc00, 0xff00).unwrap().id,
            "logitech-mx-dialpad"
        );
        // The DisplayPad is found on its control interface; faces go to interface 1.
        let pad = identify(0x3282, 0x0009, 0xff00).unwrap();
        assert_eq!((pad.id, pad.keys), ("mountain-displaypad", 12));
        assert_eq!(
            pad.protocol.paired_interfaces(),
            Some(PairedInterfaces {
                control: 3,
                display: 1
            })
        );
        assert!(!ModelView::from(pad).brightness);
        // The Quick Keys answers only on its vendor collection (interface 2).
        let quick_keys = identify(0x28bd, 0x5202, 0xff0a).unwrap();
        assert_eq!(quick_keys.id, "xencelabs-quick-keys");
        assert!(identify(0x28bd, 0x5202, 0x0001).is_none());
        assert_eq!(
            identify(0x28bd, 0x5203, 0xff0a).unwrap().id,
            "xencelabs-quick-keys-wireless"
        );
        assert_eq!(
            model_for_device_id("xencelabs-quick-keys-wireless")
                .unwrap()
                .id,
            "xencelabs-quick-keys-wireless"
        );
        let view = ModelView::from(quick_keys);
        assert!(view.brightness && !view.key_screens);
        assert_eq!(
            (view.dial_lights, view.extra_keys),
            (DialLights::PerDial, 1)
        );
        assert!(ModelView::from(by_id("elgato-mk2").unwrap()).brightness);
        let keypad = by_id("logitech-mx-keypad").unwrap();
        assert_eq!((keypad.screen_keys(), keypad.total_keys()), (9, 11));
        let neo = by_id("elgato-neo").unwrap();
        assert_eq!((neo.info_key(), neo.total_keys()), (Some(10), 11));
        assert_eq!(
            identify(0x0fd9, 0x00c6, 0x000c).unwrap().id,
            "elgato-plus-xl"
        );
        assert_eq!(identify(0x2207, 0x0019, 0x0001).unwrap().id, "ulanzi-d200");
        // The Codex Micro's keyboard collection is not the RPC channel.
        assert!(identify(0x303a, 0x8360, 0x0001).is_none());
        assert_eq!(
            identify(0x303a, 0x8360, 0xff00).unwrap().id,
            "openai-codex-micro"
        );
        assert!(identify(0x046d, 0xc52b, 0x0001).is_none());
        // Rebrands resolve through `more_usb_ids`, on the command collection only.
        assert_eq!(identify(0x0b00, 0x1000, 0xffa0).unwrap().id, "ajazz-akp153");
        assert_eq!(
            identify(0x6603, 0x1003, 0xffa0).unwrap().id,
            "ajazz-akp03-v3"
        );
        assert!(identify(0x6603, 0x1003, 0x0001).is_none());
        assert_eq!(identify(0x5548, 0x6672, 0xffa0).unwrap().id, "ajazz-akp815");
        assert_eq!(identify(0x5548, 0x1008, 0xffa0).unwrap().id, "ajazz-akp05");
        assert_eq!(
            identify(0x6603, 0x1006, 0xffa0).unwrap().id,
            "mirabox-293-v3"
        );
        assert_eq!(identify(0x5548, 0x1000, 0xffa0).unwrap().id, "mirabox-m18");
        assert_eq!(
            identify(0x5548, 0x1031, 0xffa0).unwrap().id,
            "mirabox-stream-dock-xl"
        );
        assert_eq!(identify(0x6603, 0x1000, 0xffa0).unwrap().id, "mirabox-n1");
        assert_eq!(
            identify(0x0200, 0x1000, 0xffa0).unwrap().id,
            "redragon-ss550"
        );
        assert_eq!(
            identify(0x0200, 0x3000, 0xffa0).unwrap().id,
            "redragon-ss550-v3"
        );
        assert_eq!(identify(0x0600, 0x3001, 0xffa0).unwrap().id, "womier-duo87");
        assert_eq!(identify(0x5548, 0x1020, 0xffa0).unwrap().id, "magtran-m3");
        let n1 = by_id("mirabox-n1").unwrap();
        assert!(n1.extra_has_screen(15) && n1.extra_has_screen(16));
        assert!(!n1.extra_has_screen(14));
        assert_eq!(n1.wheel_key(), Some(17));
        // Serial decks never match a HID collection, and the other way round.
        assert!(identify(0x2ec2, 0x0004, 0x0000).is_none());
        assert_eq!(
            identify_serial(0x1532, 0x0d06).unwrap().id,
            "loupedeck-live"
        );
        assert!(identify_serial(0x0fd9, 0x0080).is_none());
        let ct = by_id("loupedeck-ct").unwrap();
        assert_eq!((ct.wheel_key(), ct.total_keys()), (Some(38), 39));
        assert_eq!(by_id("loupedeck-live").unwrap().wheel_key(), None);
        // Blackmagic panels match on their ids alone, USB only.
        let speed = identify(0x1edb, 0xda0e, 0x0001).unwrap();
        assert_eq!(
            (speed.id, speed.keys, speed.dials),
            ("blackmagic-speed-editor", 43, 1)
        );
        assert!(speed.protocol.wired_only());
        assert_eq!(
            identify(0x1edb, 0xda11, 0xff00).unwrap().id,
            "blackmagic-replay-editor"
        );
        let atem = ModelView::from(identify(0x1edb, 0xbef0, 0x0001).unwrap());
        assert_eq!(atem.key_light_colors, "basic_rgb");
        assert!(atem.lit_keys.is_none() && !atem.brightness);
        assert_eq!(atem.key_names.as_ref().unwrap()[0], "Macro");
        // MACRO sits at row 0, column 5 of 17.
        assert_eq!(atem.key_cells.as_ref().unwrap()[0], 5);
        let speed = ModelView::from(speed);
        assert_eq!(speed.key_light_colors, "on_off");
        assert_eq!(speed.lit_keys.as_ref().map(Vec::len), Some(21));
        assert!(ModelView::from(by_id("elgato-mk2").unwrap())
            .key_names
            .is_none());
        // The Mystrix answers to 0x1040 plus its device id, on its vendor
        // collection only.
        for pid in [0x1040, 0x105a, 0x107f] {
            let mystrix = identify(0x0203, pid, 0xff00).unwrap();
            assert_eq!(
                (mystrix.id, mystrix.keys, mystrix.key_lights),
                ("203systems-mystrix", 64, true)
            );
        }
        assert!(identify(0x0203, 0x1040, 0x0001).is_none());
        assert!(identify(0x0203, 0x1080, 0xff00).is_none());
        assert!(by_id("203systems-mystrix")
            .unwrap()
            .protocol
            .open_probe()
            .is_some_and(|probe| probe.len() == 64 && probe[0] == 0xff));
        // The Framework macropad is driven only on the Companion firmware's
        // raw-HID collection, never stock VIA or its keyboard collections.
        let macropad = identify(0x32ac, 0x0013, 0xffdd).unwrap();
        assert_eq!(
            (macropad.id, macropad.keys, macropad.key_lights),
            ("framework-16-macropad", 24, true)
        );
        assert!(macropad.image.is_none() && macropad.protocol.has_brightness());
        assert!(identify(0x32ac, 0x0013, 0xff60).is_none());
        assert!(identify(0x32ac, 0x0013, 0x0001).is_none());
        assert!(identify(0x32ac, 0x0012, 0xffdd).is_none());
        // The Adafruit MacroPad is claimed only on its Consumer collections;
        // stock CircuitPython's keyboard and mouse never are.
        let adafruit = identify(0x239a, 0x8108, 0x000c).unwrap();
        assert_eq!(
            (
                adafruit.id,
                adafruit.family,
                adafruit.keys,
                adafruit.rows,
                adafruit.cols
            ),
            ("adafruit-macropad-rp2040", Family::Adafruit, 12, 4, 3)
        );
        assert!(adafruit.key_lights && adafruit.image.is_none() && adafruit.dials == 0);
        assert!(adafruit.protocol.has_brightness());
        assert!(adafruit.protocol.keeps_alternate_paths());
        assert!(adafruit.protocol.accepts_collection(0x000c, 0x01));
        assert!(!adafruit.protocol.accepts_collection(0x000c, 0x06));
        assert!(identify(0x239a, 0x8108, 0x0001).is_none());
        assert!(identify(0x239a, 0x8107, 0x000c).is_none());
        assert_eq!(
            serde_json::to_value(Family::Adafruit).unwrap(),
            serde_json::json!("adafruit")
        );
        // The VEC footpedal shares the X-keys vendor id but has its own
        // product id; it has nothing to light or dim.
        let pedal = identify(0x05f3, 0x00ff, 0x000c).unwrap();
        assert_eq!(
            (pedal.id, pedal.family, pedal.keys, pedal.dials),
            ("vec-footpedal", Family::Vec, 3, 0)
        );
        let pedal = ModelView::from(pedal);
        assert!(!pedal.brightness && !pedal.key_lights && !pedal.key_screens);
        // TourBoxes are serial decks; the generic Neo ids are probed first.
        let elite = identify_serial(0xc251, 0x2005).unwrap();
        assert_eq!(
            (elite.id, elite.family, elite.keys, elite.dials),
            ("tourbox-elite", Family::TourBox, 11, 3)
        );
        assert!(!elite.protocol.needs_serial_probe(0xc251, 0x2005));
        assert_eq!(elite.protocol.serial_baud(), 115_200);
        assert!(identify(0xc251, 0x2005, 0x0001).is_none());
        let neo = identify_serial(0x2e3c, 0x5740).unwrap();
        assert_eq!(neo.id, "tourbox-neo");
        assert!(neo.protocol.needs_serial_probe(0x2e3c, 0x5740));
        assert_eq!(
            identify_serial(0xcafe, 0x4001).unwrap().id,
            "tourbox-neo-unconfirmed"
        );
        assert_eq!(
            model_for_device_id("tourbox-neo-unconfirmed-ab12")
                .unwrap()
                .id,
            "tourbox-neo-unconfirmed"
        );
        let neo = ModelView::from(neo);
        assert!(!neo.brightness && !neo.key_lights && !neo.key_screens);
        assert_eq!(neo.key_names.as_ref().unwrap()[6], "Tour");
        assert!(!Protocol::Loupedeck.needs_serial_probe(0x2e3c, 0x5740));
        assert_eq!(Protocol::Loupedeck.serial_baud(), 256_000);
        // The Galleon 100 SD's panel is interface 0 of a keyboard; the
        // keyboard's other interfaces on the same ids are not the deck.
        let galleon = identify(0x1b1c, 0x2b18, 0x000c).unwrap();
        assert_eq!(
            (galleon.id, galleon.family, galleon.keys, galleon.dials),
            ("corsair-galleon-100-sd", Family::Corsair, 12, 2)
        );
        let panel = galleon.protocol.hid_interface().unwrap();
        assert!(panel.accepts(0, 0x01) && panel.accepts(0, 0));
        assert!(!panel.accepts(1, 0x01) && !panel.accepts(0, 0x06));
        assert!(Protocol::ElgatoPlus.hid_interface().is_none());
        assert!(galleon.protocol.zero_serial_is_none());
        assert_eq!(
            (galleon.strip_segment(12), galleon.total_keys()),
            (Some(0), 14)
        );
        let view = ModelView::from(galleon);
        assert!(view.brightness && view.key_screens);
        assert_eq!(view.strip_face, Some([360, 384]));
        assert_eq!(view.strip_placement, StripPlacement::Above);
        // Each MasterHub module is a deck of its own on Cooler Master's id.
        let keypad = identify(0x2516, 0x01fe, 0xff00).unwrap();
        assert_eq!(
            (keypad.id, keypad.family, keypad.keys, keypad.dials),
            ("coolermaster-masterhub-keypad", Family::CoolerMaster, 15, 0)
        );
        assert_eq!(keypad.image.map(|image| image.width), Some(72));
        assert!(ModelView::from(keypad).brightness);
        let knob = identify(0x2516, 0x0202, 0x0001).unwrap();
        assert_eq!((knob.id, knob.dials), ("coolermaster-masterhub-knob", 3));
        assert_eq!(knob.dial_lights, DialLights::PerDial);
        assert!(ModelView::from(knob).brightness);
        let roller = ModelView::from(identify(0x2516, 0x0204, 0x0001).unwrap());
        assert!(!roller.brightness && roller.dial_lights == DialLights::None);
        assert_eq!(identify(0x2516, 0x0200, 0).unwrap().dials, 5);
        let dial = identify(0x2516, 0x0206, 0).unwrap();
        assert_eq!((dial.wheel_key(), dial.total_keys()), (Some(0), 1));
        assert!(!ModelView::from(dial).brightness);
        let base = ModelView::from(identify(0x2516, 0x01f6, 0).unwrap());
        assert_eq!((base.deck_lights, base.keys, base.dials), (1, 0, 0));
        assert!(base.brightness);
        assert!(identify(0x2516, 0x0208, 0).is_none());
        // 3Dconnexion mice match on their multi-axis (Generic Desktop)
        // collection; the older ones sit on Logitech's vendor id.
        let navigator = identify(0x046d, 0xc626, 0x0001).unwrap();
        assert_eq!(
            (
                navigator.id,
                navigator.family,
                navigator.keys,
                navigator.dials
            ),
            ("3dconnexion-spacenavigator", Family::ThreeDconnexion, 2, 6)
        );
        assert!(identify(0x046d, 0xc626, 0xff00).is_none());
        assert_eq!(
            identify(0x256f, 0xc638, 0x0001).unwrap().id,
            "3dconnexion-spacemouse-pro-wireless"
        );
        assert_eq!(
            identify(0x256f, 0xc633, 0x0001).unwrap().id,
            "3dconnexion-spacemouse-enterprise"
        );
        assert!(identify(0x256f, 0xc664, 0x0001).is_none(), "Keyboard Pro");
        assert!(
            identify(0x256f, 0xc65a, 0x0001).is_none(),
            "Universal Receiver 2"
        );
        let pro = ModelView::from(identify(0x256f, 0xc652, 0x0001).unwrap());
        assert_eq!(pro.key_names.as_ref().unwrap()[0], "Menu");
        assert_eq!(pro.deck_lights, 1);
        assert!(!pro.brightness && !pro.key_screens && !pro.key_lights);
        let usb = ModelView::from(identify(0x256f, 0xc641, 0x0001).unwrap());
        assert_eq!((usb.keys, usb.dials, usb.deck_lights), (0, 6, 0));
        assert!(usb.key_names.is_none());
        assert_eq!(
            ModelView::from(by_id("3dconnexion-spacepilot").unwrap()).deck_lights,
            0
        );
        // The G13 has one HID interface, whatever usage the host reports.
        let g13 = identify(0x046d, 0xc21c, 0x0001).unwrap();
        assert_eq!(
            (
                g13.id,
                g13.family,
                g13.keys,
                g13.total_keys(),
                g13.info_key()
            ),
            ("logitech-g13", Family::Logitech, 38, 39, Some(38))
        );
        assert_eq!(identify(0x046d, 0xc21c, 0).unwrap().id, "logitech-g13");
        let g13 = ModelView::from(g13);
        assert_eq!(g13.key_light_colors, "on_off");
        assert_eq!(g13.lit_keys, Some(vec![22, 23, 24, 25]));
        assert_eq!(g13.info_face, Some([160, 43]));
        assert!(g13.brightness && g13.key_lights && !g13.key_screens);
        assert_eq!(g13.deck_lights, 1);
        let names = g13.key_names.unwrap();
        assert_eq!(
            (names[0], names[26], names[37]),
            ("G1", "Round button", "Stick right")
        );
        let cells = g13.key_cells.unwrap();
        assert_eq!(cells.len(), 38);
        let distinct: std::collections::HashSet<_> = cells.iter().collect();
        assert_eq!(distinct.len(), 38, "two G13 keys share a cell");
        assert!(cells.iter().all(|cell| *cell < 49));
        // The Touch Bar is never found by USB id.
        assert!(MODELS
            .iter()
            .filter(|model| model.protocol == Protocol::TouchBar)
            .all(|model| model.product_ids.is_empty() && model.more_usb_ids.is_empty()));
    }

    #[test]
    fn the_yamaha_cc121mk2_is_a_serial_deck_with_palette_lights() {
        let cc1 = identify_serial(0x0499, 0x140f).unwrap();
        assert_eq!(
            (cc1.id, cc1.family, cc1.keys, cc1.extra_keys, cc1.dials),
            ("yamaha-cc121mk2", Family::Yamaha, 29, 17, 7)
        );
        assert!(identify(0x0499, 0x140f, 0x0001).is_none());
        assert!(!cc1.protocol.needs_serial_probe(0x0499, 0x140f));
        assert!(cc1.protocol.holds_dtr() && cc1.protocol.serial_is_invented());
        assert_eq!(cc1.protocol.serial_baud(), 115_200);
        let image = cc1.image.unwrap();
        assert_eq!((image.width, image.height), (72, 72));
        assert_eq!(image.mode, ImageMode::Rgb565);
        assert_eq!(cc1.screen_keys(), 12);
        let view = ModelView::from(cc1);
        assert_eq!(view.key_light_colors, "palette");
        assert_eq!(view.lit_keys, Some((14..29).collect::<Vec<u8>>()));
        assert!(view.brightness && view.key_screens);
        let names = view.key_names.unwrap();
        assert_eq!(names.len(), 29);
        assert_eq!(
            (names[12], names[14], names[28]),
            ("Button 1", "Button 15", "Button 29")
        );
        assert_eq!(device_id(cc1, None), "yamaha-cc121mk2");
        assert_eq!(model_for_device_id("yamaha-cc121mk2").unwrap().id, cc1.id);
    }

    #[test]
    fn the_dji_rc_n1_is_a_probed_serial_remote_without_outputs() {
        let remote = identify_serial(0x2ca3, 0x1020).unwrap();
        assert_eq!(
            (
                remote.id,
                remote.family,
                remote.keys,
                remote.dials,
                remote.extra_keys
            ),
            ("dji-rc-n1", Family::Dji, 10, 5, 0)
        );
        assert!(identify(0x2ca3, 0x1020, 0).is_none(), "not a HID device");
        let protocol = remote.protocol;
        assert!(protocol.is_serial() && protocol.needs_serial_probe(0x2ca3, 0x1020));
        assert!(protocol.reprobes_silent_ports());
        assert!(!Protocol::TourBoxSerial.reprobes_silent_ports());
        assert_eq!(protocol.serial_baud(), 115_200);
        assert!(!protocol.has_brightness());
        assert_eq!(
            serde_json::to_value(Family::Dji).unwrap(),
            serde_json::json!("dji")
        );
        let view = ModelView::from(remote);
        assert!(!view.brightness && !view.key_screens && !view.key_lights);
        assert_eq!((view.deck_lights, view.dial_lights), (0, DialLights::None));
        let names = view.key_names.unwrap();
        assert_eq!(
            (names[0], names[7], names[9]),
            ("LS up", "RS right", "Wheel down")
        );
        assert_eq!(device_id(remote, Some("3NZCJ1234")), "dji-rc-n1-3nzcj1234");
    }

    #[test]
    fn the_haute42_b16_is_found_only_on_surfacecore_s_vendor_collection() {
        let b16 = identify(0xcafe, 0x2043, 0xff00).unwrap();
        assert_eq!(
            (b16.id, b16.family, b16.keys, b16.dials, b16.rows, b16.cols),
            ("haute42-b16", Family::Haute42, 21, 0, 4, 15)
        );
        assert!(
            identify(0xcafe, 0x2043, 0x0001).is_none(),
            "not its collection"
        );
        assert!(
            identify_serial(0xcafe, 0x2043).is_none(),
            "never the CDC port"
        );
        assert_eq!(
            identify_serial(0xcafe, 0x4001).unwrap().family,
            Family::TourBox
        );
        assert!(!b16.protocol.is_serial() && b16.protocol.has_brightness());
        assert_eq!(
            serde_json::to_value(Family::Haute42).unwrap(),
            serde_json::json!("haute42")
        );
        // Every unit reports serial "0002", so it is not part of the id.
        assert_eq!(usable_serial(b16, Some("0002")), None);
        assert_eq!(
            device_id(b16, usable_serial(b16, Some("0002"))),
            "haute42-b16"
        );
        let view = ModelView::from(b16);
        assert!(view.key_lights && view.brightness && !view.key_screens);
        assert_eq!(view.lit_keys, Some((0..16).collect::<Vec<u8>>()));
        assert_eq!(
            view.key_cells,
            Some(vec![
                0, 4, 5, 6, 7, 8, 15, 16, 17, 19, 20, 21, 22, 36, 50, 51, 10, 11, 12, 13, 14
            ])
        );
        let names = view.key_names.unwrap();
        assert_eq!((names[0], names[15], names[20]), ("B01", "B16", "Fn5"));
    }

    #[test]
    fn maschine_controllers_are_hid_decks_with_mono_displays() {
        let mk2 = identify(0x17cc, 0x1140, 0).unwrap();
        assert_eq!(
            (mk2.id, mk2.family, mk2.keys, mk2.dials, mk2.total_keys()),
            ("ni-maschine-mk2", Family::NativeInstruments, 64, 9, 65)
        );
        assert_eq!(mk2.info_image.unwrap().mode, ImageMode::MonoRows);
        let mikro = identify(0x17cc, 0x1200, 0xff00).unwrap();
        assert_eq!(
            (mikro.id, mikro.keys, mikro.dials, mikro.info_key()),
            ("ni-maschine-mikro-mk2", 44, 1, Some(44))
        );
        assert_eq!(mikro.info_image.unwrap().mode, ImageMode::MonoColumns);
        let jam = identify(0x17cc, 0x1500, 0).unwrap();
        assert_eq!((jam.id, jam.keys, jam.dials), ("ni-maschine-jam", 115, 9));
        assert!(jam.info_key().is_none());
        // The MK1 is vendor-class USB, not HID.
        assert!(identify(0x17cc, 0x0808, 0).is_none());
        assert_eq!(
            serde_json::to_value(Family::NativeInstruments).unwrap(),
            "native_instruments"
        );
        for model in [mk2, mikro, jam] {
            let view = ModelView::from(model);
            assert!(view.brightness && view.key_lights && !view.key_screens);
            let names = view.key_names.unwrap();
            assert_eq!(names.len(), usize::from(model.keys));
            let cells = view.key_cells.unwrap();
            let distinct: std::collections::HashSet<_> = cells.iter().collect();
            assert_eq!(
                distinct.len(),
                cells.len(),
                "{} keys share a cell",
                model.id
            );
            assert!(cells
                .iter()
                .all(|cell| *cell < u16::from(model.rows) * u16::from(model.cols)));
        }
        let mk2 = ModelView::from(mk2);
        // Every key but Main (key 40) has a light.
        let lit = mk2.lit_keys.unwrap();
        assert_eq!(lit.len(), 63);
        assert!(!lit.contains(&39));
        assert_eq!(mk2.info_face, Some([512, 64]));
        assert_eq!(ModelView::from(mikro).lit_keys.unwrap().len(), 44);
        assert_eq!(ModelView::from(jam).lit_keys.unwrap().len(), 115);
    }

    #[test]
    fn traktor_kontrol_f1_is_a_hid_deck_on_interface_one() {
        let f1 = identify(0x17cc, 0x1120, 0xff01).unwrap();
        assert_eq!(
            (f1.id, f1.family, f1.keys, f1.dials, f1.total_keys()),
            (
                "ni-traktor-kontrol-f1",
                Family::NativeInstruments,
                28,
                9,
                28
            )
        );
        let interface = f1.protocol.hid_interface().unwrap();
        assert!(interface.accepts(1, 0) && interface.accepts(1, 1));
        assert!(!interface.accepts(0, 1));
        let view = ModelView::from(f1);
        assert!(view.brightness && view.key_lights && !view.key_screens);
        assert_eq!(view.key_light_colors, "rgb");
        // Every key has a light.
        assert!(view.lit_keys.is_none());
        let names = view.key_names.unwrap();
        assert_eq!(
            (names.len(), names[0], names[16], names[27]),
            (28, "Pad 1", "Browse", "Stop 4")
        );
        let cells = view.key_cells.unwrap();
        let distinct: std::collections::HashSet<_> = cells.iter().collect();
        assert_eq!(distinct.len(), cells.len());
        assert!(cells
            .iter()
            .all(|cell| *cell < u16::from(f1.rows) * u16::from(f1.cols)));
        // Pad 1 sits below the eight buttons, Stop 1 below the pads.
        assert_eq!((cells[0], cells[24]), (8, 24));
    }

    #[test]
    fn komplete_kontrol_s_mk1_is_its_control_section() {
        for (pid, id, guide) in [
            (0x1340, "ni-komplete-kontrol-s25", 25),
            (0x1350, "ni-komplete-kontrol-s49", 49),
            (0x1360, "ni-komplete-kontrol-s61", 61),
            (0x1410, "ni-komplete-kontrol-s88", 88),
        ] {
            let kk = identify(0x17cc, pid, 0xff01).unwrap();
            assert_eq!(
                (kk.id, kk.family, kk.keys, kk.dials, kk.total_keys()),
                (id, Family::NativeInstruments, 23, 9, 23)
            );
            assert_eq!(usize::from(kk.deck_lights), guide);
            assert_eq!(komplete::guide_keys(kk), guide);
            assert!(kk.protocol.hid_interface().is_none());
            let view = ModelView::from(kk);
            assert!(view.brightness && view.key_lights && !view.key_screens);
            assert!(view.lit_keys.is_none());
            let names = view.key_names.unwrap();
            assert_eq!((names.len(), names[0], names[22]), (23, "Preset Up", "Ffw"));
            let cells = view.key_cells.unwrap();
            let distinct: std::collections::HashSet<_> = cells.iter().collect();
            assert_eq!(distinct.len(), 23);
            assert!(cells
                .iter()
                .all(|cell| *cell < u16::from(kk.rows) * u16::from(kk.cols)));
            // Shift top-left, Navigate Down at the bottom of the cross.
            assert_eq!((cells[15], cells[11]), (0, 4 * 7 + 5));
        }
    }

    #[test]
    fn logitech_rgb_keyboards_are_found_on_the_hidpp_collection_hidapi_lists() {
        for (pid, id, keys, protocol) in [
            (
                0xc33e,
                "logitech-g915",
                5,
                Protocol::LogitechPerKeyLighting2,
            ),
            (
                0xc33f,
                "logitech-g815",
                5,
                Protocol::LogitechPerKeyLighting2,
            ),
            (0xc32b, "logitech-g910", 9, Protocol::LogitechPerKeyLighting),
            (0xc335, "logitech-g910", 9, Protocol::LogitechPerKeyLighting),
            (0xc33c, "logitech-g513", 0, Protocol::LogitechPerKeyLighting),
            (
                0xc339,
                "logitech-g-pro-keyboard",
                0,
                Protocol::LogitechPerKeyLighting,
            ),
        ] {
            // What hidapi lists for the HID++ collection (OpenRGB's detector):
            // the wired G915 on interface 2, page 0xFF00, usage 2; the others
            // on interface 1, page 0xFF43, usages 0x0602 and 0x0604.
            let (page, number, usage, sibling) = if id == "logitech-g915" {
                (0xff00, 2, 0x0002, 0x0001)
            } else {
                (0xff43, 1, 0x0602, 0x0604)
            };
            let model = identify(0x046d, pid, page).unwrap();
            assert_eq!(
                (model.id, model.family, model.keys, model.protocol),
                (id, Family::Logitech, keys, protocol)
            );
            assert!(
                identify(0x046d, pid, 0x0001).is_none(),
                "{id}: the keyboard collection is not the deck"
            );
            let interface = model.hid_interface().unwrap();
            assert!(
                interface.accepts(number, usage),
                "{id}: its HID++ collection"
            );
            assert!(interface.accepts(number, 0), "a host that reports no usage");
            assert!(
                !interface.accepts(number, sibling),
                "{id}: not a second deck"
            );
            assert!(!interface.accepts(0, usage), "{id}: the keyboard interface");
            assert!(
                !interface.accepts(3 - number, usage),
                "{id}: the other interface"
            );
            let view = ModelView::from(model);
            assert_eq!(view.deck_lights, 1);
            assert!(view.brightness, "{id} scales its colours");
            assert_eq!(view.key_lights, keys > 0);
            assert_eq!(view.key_light_colors, "rgb");
            assert_eq!(
                view.key_names.map(|names| names.len()),
                Some(usize::from(keys))
            );
        }
    }

    #[test]
    fn logitech_g15_family_is_found_only_on_its_vendor_collection() {
        for (product_id, id, keys, info) in [
            (0xc222, "logitech-g15", 28, Some(28)),
            (0xc227, "logitech-g15-v2", 16, Some(16)),
            (0xc225, "logitech-g11", 23, None),
            (0xc22d, "logitech-g510", 31, Some(31)),
            (0xc22e, "logitech-g510", 31, Some(31)),
            (0xc22b, "logitech-g110", 17, None),
        ] {
            let model = identify(LOGITECH_VENDOR_ID, product_id, 0xff00).unwrap();
            assert_eq!(
                (model.id, model.family, model.keys, model.info_key()),
                (id, Family::Logitech, keys, info)
            );
            assert_eq!(model.protocol, Protocol::LogitechG15);
            // The G510's boot keyboard on interface 0, and any page the host
            // does not report, are never taken.
            assert!(identify(LOGITECH_VENDOR_ID, product_id, 0x0001).is_none());
            assert!(identify(LOGITECH_VENDOR_ID, product_id, 0x000c).is_none());
            assert!(identify(LOGITECH_VENDOR_ID, product_id, 0).is_none());
            let view = ModelView::from(model);
            assert_eq!(view.key_light_colors, "on_off");
            assert!(view.brightness && view.key_lights && !view.key_screens);
            assert_eq!(view.lit_keys.as_ref().unwrap().len(), 4);
            assert_eq!(view.key_names.as_ref().unwrap().len(), usize::from(keys));
            assert_eq!(view.key_cells.as_ref().unwrap().len(), usize::from(keys));
            assert_eq!(view.info_face, info.map(|_| [160, 43]));
            assert_eq!(view.deck_lights, u8::from(id == "logitech-g510"));
        }
        let g15 = ModelView::from(by_id("logitech-g15").unwrap());
        assert_eq!(g15.lit_keys, Some(vec![18, 19, 20, 21]));
        let v2 = ModelView::from(by_id("logitech-g15-v2").unwrap());
        assert_eq!(v2.lit_keys, Some(vec![6, 7, 8, 9]));
        let g110 = ModelView::from(by_id("logitech-g110").unwrap());
        assert_eq!(g110.lit_keys, Some(vec![12, 13, 14, 15]));
    }

    #[test]
    fn monome_devices_are_serial_decks_found_by_serial_number_not_usb_id() {
        let grid = monome::identify(Some("m1000123"), None, None).unwrap();
        assert_eq!(grid.protocol, Protocol::MonomeMext);
        let series = monome::identify(Some("m64-0042"), None, None).unwrap();
        assert_eq!(
            (
                series.id,
                series.family,
                series.rows,
                series.cols,
                series.keys
            ),
            ("monome-series-64", Family::Monome, 8, 8, 64)
        );
        // The FTDI bridge's id is never a match key.
        assert!(identify_serial(FTDI_VENDOR_ID, 0x6001).is_none());
        assert!(identify(FTDI_VENDOR_ID, 0x6001, 0).is_none());
        for model in MODELS.iter().filter(|model| model.family == Family::Monome) {
            assert!(model.protocol.is_serial(), "{}", model.id);
            assert!(model.product_ids.is_empty() && model.more_usb_ids.is_empty());
            assert!(model.image.is_none() && model.deck_lights == 0);
            let view = ModelView::from(model);
            assert!(view.brightness, "{}", model.id);
            assert!(view.usb_ids.is_empty());
        }
        let grid = by_id("monome-grid-128").unwrap();
        assert_eq!((grid.rows, grid.cols, grid.keys), (8, 16, 128));
        assert!(grid.protocol.needs_serial_probe(FTDI_VENDOR_ID, 0x6001));
        assert!(grid.protocol.reprobes_silent_ports());
        assert_eq!(ModelView::from(grid).key_light_colors, "levels");
        assert_eq!(
            ModelView::from(by_id("monome-40h").unwrap()).key_light_colors,
            "on_off"
        );
        let arc = by_id("monome-arc-4").unwrap();
        assert_eq!(
            (arc.keys, arc.dials, arc.dial_lights),
            (0, 4, DialLights::PerDial)
        );
        assert!(!ModelView::from(arc).key_lights);
        assert_eq!(by_id("monome-arduinome").unwrap().serial_baud(), 57_600);
        assert_eq!(arc.serial_baud(), 115_200);
        assert_eq!(
            serde_json::to_value(Family::Monome).unwrap(),
            serde_json::json!("monome")
        );
        assert_eq!(
            device_id(grid, Some("m1000123")),
            "monome-grid-128-m1000123"
        );
    }

    #[test]
    fn a_wooting_is_claimed_by_its_analog_collection_only() {
        // Every gamepad mode of a 31E3 board, on the V2 or V1 analog page.
        for pid in [0x1220, 0x1221, 0x1222] {
            assert_eq!(identify(0x31e3, pid, 0xff53).unwrap().id, "wooting-two-he");
        }
        assert_eq!(identify(0x31e3, 0x1100, 0xff54).unwrap().id, "wooting-one");
        let sixty = identify(0x31e3, 0x1340, 0xff53).unwrap();
        assert_eq!(
            (sixty.id, sixty.rows, sixty.cols),
            ("wooting-60he-v2", 6, 14)
        );
        assert_eq!(identify(0x31e3, 0x1402, 0xff53).unwrap().cols, 17);
        // Its keyboard and lighting collections are not decks of their own.
        for page in [0x0001, 0x000c, 0x1337, 0xff55] {
            assert!(identify(0x31e3, 0x1220, page).is_none(), "{page:04x}");
        }
        // The legacy boards sit on Atmel's vendor id, claimed only on 0xFF54.
        let two = identify(0x03eb, 0xff02, 0xff54).unwrap();
        assert_eq!(
            (two.id, two.protocol, two.keys),
            ("wooting-two-legacy", Protocol::WootingLegacy, 126)
        );
        assert!(identify(0x03eb, 0xff01, 0x0001).is_none());
        assert!(identify(0x03eb, 0x2ff4, 0xff54).is_none());
        // Its Dynamic Lighting collection is not lit as a LampArray too.
        assert!(driven_by_paired_collections(0x31e3, 0x1342));
        assert!(!driven_by_paired_collections(0x0414, 0x8104));
        assert_eq!(
            Protocol::WootingAnalog.paired_collections(),
            Some(PairedCollections {
                control: &[0xff53, 0xff54],
                display: &[0x1337, 0xff55],
            })
        );
        assert!(Protocol::WootingAnalog.has_brightness());
        let view = ModelView::from(by_id("wooting-two").unwrap());
        assert_eq!(view.key_names.as_ref().unwrap()[0], "Esc");
        assert!(ModelView::from(by_id("wooting-60he").unwrap())
            .key_names
            .is_none());
    }

    #[test]
    fn launchpads_are_found_by_midi_port_name_alone() {
        let x = identify_midi(
            &midi::normalise("Launchpad X:Launchpad X LPX MIDI 24:1"),
            false,
        )
        .unwrap();
        assert_eq!((x.id, x.family), (launchpad::X_ID, Family::Novation));
        assert!(identify_midi(&midi::normalise("LPX DAW"), false).is_none());
        assert_eq!(
            identify_midi(&midi::normalise("MIDIIN2 (LPMiniMK3 MIDI)"), true)
                .unwrap()
                .id,
            launchpad::MINI_MK3_ID
        );
        assert_eq!(
            identify_midi(&midi::normalise("LPProMK3 MIDI"), true)
                .unwrap()
                .id,
            launchpad::PRO_MK3_ID
        );
        // Their USB ids are shown, never claimed by the HID, serial or evdev scans.
        for pid in [0x0113, 0x0103, 0x0123] {
            assert!(identify(NOVATION_VENDOR_ID, pid, 0).is_none());
            assert!(identify(NOVATION_VENDOR_ID, pid, 0xff00).is_none());
            assert!(identify_serial(NOVATION_VENDOR_ID, pid).is_none());
            assert!(identify_evdev(NOVATION_VENDOR_ID, pid).is_none());
        }
        let pro = by_id(launchpad::PRO_MK3_ID).unwrap();
        assert!(pro.protocol.is_midi() && !pro.protocol.is_serial());
        assert!(pro.protocol.has_brightness());
        let view = ModelView::from(pro);
        assert_eq!(view.usb_ids, vec!["1235:0123".to_owned()]);
        assert!(view.brightness && view.key_lights);
        assert_eq!(view.key_light_colors, "rgb");
        assert!(view.lit_keys.is_none());
        assert_eq!(view.deck_lights, 1);
        assert_eq!(
            serde_json::to_value(Family::Novation).unwrap(),
            serde_json::json!("novation")
        );
    }

    #[test]
    fn the_apc_mini_mk2_is_found_by_its_control_port_alone() {
        let apc = identify_midi(&midi::normalise("APC mini mk2 Control"), false).unwrap();
        assert_eq!(
            (apc.id, apc.family, apc.protocol),
            (
                akai_apc::APC_MINI_MK2_ID,
                Family::Akai,
                Protocol::AkaiApcMiniMk2Midi
            )
        );
        assert!(identify_midi(&midi::normalise("APC mini mk2 Notes"), false).is_none());
        assert_eq!(
            identify_midi(&midi::normalise("APC mini mk2"), true)
                .unwrap()
                .id,
            akai_apc::APC_MINI_MK2_ID
        );
        // Shown only: the USB id never identifies it.
        assert!(apc.matches_usb(AKAI_VENDOR_ID, 0x004f));
        assert!(identify(AKAI_VENDOR_ID, 0x004f, 0).is_none());
        assert!(identify_serial(AKAI_VENDOR_ID, 0x004f).is_none());
        assert!(identify_evdev(AKAI_VENDOR_ID, 0x004f).is_none());
        assert_eq!(ModelView::from(apc).usb_ids, vec!["09e8:004f".to_owned()]);
    }

    #[test]
    fn the_first_apc_mini_is_found_by_its_port_name() {
        let apc = identify_midi(&midi::normalise("APC MINI:APC MINI MIDI 1 24:0"), false).unwrap();
        assert_eq!(
            (apc.id, apc.family, apc.protocol),
            (
                akai_apc_mini::APC_MINI_ID,
                Family::Akai,
                Protocol::AkaiApcMiniMidi
            )
        );
        assert!(!apc.protocol.has_brightness());
        // The MK2 keeps its own ports.
        assert_eq!(
            identify_midi(&midi::normalise("APC mini mk2 Control"), false)
                .unwrap()
                .id,
            akai_apc::APC_MINI_MK2_ID
        );
        // Shown only: the USB id never identifies it.
        assert!(apc.matches_usb(AKAI_VENDOR_ID, 0x0028));
        assert!(identify(AKAI_VENDOR_ID, 0x0028, 0).is_none());
        assert!(identify_serial(AKAI_VENDOR_ID, 0x0028).is_none());
        assert_eq!(ModelView::from(apc).usb_ids, vec!["09e8:0028".to_owned()]);
    }

    #[test]
    fn the_midimix_is_found_by_its_port_name() {
        let mix = identify_midi(&midi::normalise("MIDI Mix"), false).unwrap();
        assert_eq!(
            (mix.id, mix.family, mix.protocol),
            (
                akai_midimix::MIDIMIX_ID,
                Family::Akai,
                Protocol::AkaiMidimixMidi
            )
        );
        assert!(!mix.protocol.has_brightness());
        // Shown only: the USB id never identifies it.
        assert!(mix.matches_usb(AKAI_VENDOR_ID, 0x0031));
        assert!(identify(AKAI_VENDOR_ID, 0x0031, 0).is_none());
        assert!(identify_serial(AKAI_VENDOR_ID, 0x0031).is_none());
        assert_eq!(ModelView::from(mix).usb_ids, vec!["09e8:0031".to_owned()]);
    }

    #[test]
    fn the_lpd8_is_found_by_its_port_name() {
        let lpd8 = identify_midi(&midi::normalise("LPD8"), false).unwrap();
        assert_eq!(
            (lpd8.id, lpd8.family, lpd8.protocol),
            (akai_lpd8::LPD8_ID, Family::Akai, Protocol::AkaiLpd8Midi)
        );
        assert!(!lpd8.protocol.has_brightness());
        // Shown only: the USB id never identifies it.
        assert!(lpd8.matches_usb(AKAI_VENDOR_ID, 0x0075));
        assert!(identify(AKAI_VENDOR_ID, 0x0075, 0).is_none());
        assert!(identify_serial(AKAI_VENDOR_ID, 0x0075).is_none());
        assert_eq!(ModelView::from(lpd8).usb_ids, vec!["09e8:0075".to_owned()]);
    }

    #[test]
    fn the_fire_is_found_by_its_port_name() {
        let fire = identify_midi(&midi::normalise("FL STUDIO FIRE"), false).unwrap();
        assert_eq!(
            (fire.id, fire.family, fire.protocol),
            (akai_fire::FIRE_ID, Family::Akai, Protocol::AkaiFireMidi)
        );
        // Brightness is applied in software.
        assert!(fire.protocol.has_brightness());
        assert_eq!(fire.info_key(), Some(84));
        // Shown only: the USB id never identifies it.
        assert!(fire.matches_usb(AKAI_VENDOR_ID, 0x0043));
        assert!(identify(AKAI_VENDOR_ID, 0x0043, 0).is_none());
        assert!(identify_serial(AKAI_VENDOR_ID, 0x0043).is_none());
        assert_eq!(ModelView::from(fire).usb_ids, vec!["09e8:0043".to_owned()]);
        assert_eq!(ModelView::from(fire).key_light_colors, "rgb");
    }

    #[test]
    fn the_apc40_mk1_and_apc20_are_found_by_their_port_names() {
        for (port, id, protocol, product) in [
            (
                "Akai APC40",
                akai_apc40_mk1::APC40_MK1_ID,
                Protocol::AkaiApc40Mk1Midi,
                0x0073,
            ),
            (
                "Akai APC20",
                akai_apc40_mk1::APC20_ID,
                Protocol::AkaiApc20Midi,
                0x007b,
            ),
        ] {
            let apc = identify_midi(&midi::normalise(port), false).unwrap();
            assert_eq!(
                (apc.id, apc.family, apc.protocol),
                (id, Family::Akai, protocol)
            );
            assert!(apc.protocol.is_midi() && !apc.protocol.has_brightness());
            // Shown only: the USB id never identifies it.
            assert!(apc.matches_usb(AKAI_VENDOR_ID, product));
            assert!(identify(AKAI_VENDOR_ID, product, 0).is_none());
            assert!(identify_serial(AKAI_VENDOR_ID, product).is_none());
        }
    }

    #[test]
    fn the_apc40_mk2_is_found_by_its_port_name() {
        let apc = identify_midi(&midi::normalise("APC40 mkII"), false).unwrap();
        assert_eq!(
            (apc.id, apc.family, apc.protocol),
            (
                akai_apc40::APC40_MK2_ID,
                Family::Akai,
                Protocol::AkaiApc40Mk2Midi
            )
        );
        assert!(apc.protocol.is_midi() && !apc.protocol.is_serial());
        assert!(!apc.protocol.has_brightness());
        // Shown only: the USB id never identifies it.
        assert!(apc.matches_usb(AKAI_VENDOR_ID, 0x0029));
        assert!(identify(AKAI_VENDOR_ID, 0x0029, 0).is_none());
        assert!(identify_serial(AKAI_VENDOR_ID, 0x0029).is_none());
        assert!(identify_evdev(AKAI_VENDOR_ID, 0x0029).is_none());
        assert_eq!(ModelView::from(apc).usb_ids, vec!["09e8:0029".to_owned()]);
    }

    #[test]
    fn the_apc_key_25s_are_found_by_their_port_names() {
        let mk1 = identify_midi(&midi::normalise("APC Key 25"), false).unwrap();
        assert_eq!(
            (mk1.id, mk1.family, mk1.protocol),
            (
                akai_apc_key25::APC_KEY_25_ID,
                Family::Akai,
                Protocol::AkaiApcKey25Midi
            )
        );
        assert!(mk1.protocol.is_midi() && !mk1.protocol.is_serial());
        assert!(!mk1.protocol.has_brightness());
        let mk2 = identify_midi(&midi::normalise("APC Key 25 mk2 Control"), false).unwrap();
        assert_eq!(
            (mk2.id, mk2.protocol),
            (
                akai_apc_key25::APC_KEY_25_MK2_ID,
                Protocol::AkaiApcKey25Mk2Midi
            )
        );
        assert!(mk2.protocol.has_brightness());
        assert!(identify_midi(&midi::normalise("APC Key 25 mk2 Keys"), false).is_none());
        // Shown only: the USB ids never identify them.
        for (model, pid) in [(mk1, 0x0027), (mk2, 0x004e)] {
            assert!(model.matches_usb(AKAI_VENDOR_ID, pid));
            assert!(identify(AKAI_VENDOR_ID, pid, 0).is_none());
            assert!(identify_serial(AKAI_VENDOR_ID, pid).is_none());
            assert!(identify_evdev(AKAI_VENDOR_ID, pid).is_none());
        }
        assert_eq!(ModelView::from(mk2).usb_ids, vec!["09e8:004e".to_owned()]);
    }

    #[test]
    fn the_sl_mkiii_is_found_by_its_incontrol_port_alone() {
        let sl = identify_midi(
            &midi::normalise("Novation SL MkIII SL MkIII InControl"),
            false,
        )
        .unwrap();
        assert_eq!(
            (sl.id, sl.family, sl.protocol),
            (
                sl_mkiii::SL_MKIII_ID,
                Family::Novation,
                Protocol::NovationSlMk3Midi
            )
        );
        assert!(sl.protocol.is_midi() && !sl.protocol.is_serial());
        assert!(sl.protocol.has_brightness());
        assert!(sl.midi_port().is_none());
        assert!(identify_midi(&midi::normalise("SL MkIII MIDI 1"), false).is_none());
        assert_eq!(
            identify_midi(&midi::normalise("MIDIIN2 (Novation SL MkIII)"), true)
                .unwrap()
                .id,
            sl_mkiii::SL_MKIII_ID
        );
        // Shown only: the USB id never identifies it.
        assert!(sl.matches_usb(NOVATION_VENDOR_ID, 0x0101));
        assert!(identify(NOVATION_VENDOR_ID, 0x0101, 0).is_none());
        assert!(identify_serial(NOVATION_VENDOR_ID, 0x0101).is_none());
        assert!(identify_evdev(NOVATION_VENDOR_ID, 0x0101).is_none());
        assert_eq!(ModelView::from(sl).usb_ids, vec!["1235:0101".to_owned()]);
    }

    #[test]
    fn the_first_push_is_found_by_its_user_port_alone() {
        let push = identify_midi(&midi::normalise("Ableton Push User Port"), false).unwrap();
        assert_eq!(
            (push.id, push.family, push.protocol),
            (
                push1::PUSH_1_ID,
                Family::Ableton,
                Protocol::AbletonPush1Midi
            )
        );
        assert!(push.protocol.is_midi() && !push.protocol.is_serial());
        assert!(!push.protocol.has_brightness());
        assert!(identify_midi(&midi::normalise("Ableton Push Live Port"), false).is_none());
        assert_eq!(
            identify_midi(&midi::normalise("MIDIIN2 (Ableton Push)"), true)
                .unwrap()
                .id,
            push1::PUSH_1_ID
        );
        // Shown only: the USB id never identifies it.
        assert!(push.matches_usb(AKAI_VENDOR_ID, 0x0015));
        assert!(identify(AKAI_VENDOR_ID, 0x0015, 0).is_none());
        assert!(identify_serial(AKAI_VENDOR_ID, 0x0015).is_none());
        assert!(identify_evdev(AKAI_VENDOR_ID, 0x0015).is_none());
        assert_eq!(ModelView::from(push).usb_ids, vec!["09e8:0015".to_owned()]);
    }

    #[test]
    fn the_push_2_is_found_by_its_user_port_alone() {
        let push = identify_midi(&midi::normalise("Ableton Push 2 User Port"), false).unwrap();
        assert_eq!(
            (push.id, push.family, push.protocol),
            (
                push2::PUSH_2_ID,
                Family::Ableton,
                Protocol::AbletonPush2Midi
            )
        );
        assert!(push.protocol.is_midi() && !push.protocol.is_serial());
        assert!(push.protocol.has_brightness());
        assert!(identify_midi(&midi::normalise("Ableton Push 2 Live Port"), false).is_none());
        assert!(identify_midi(
            &midi::normalise("Ableton Push 2:Ableton Push 2 MIDI 1 20:0"),
            false
        )
        .is_none());
        assert_eq!(
            identify_midi(&midi::normalise("MIDIIN2 (Ableton Push 2)"), true)
                .unwrap()
                .id,
            push2::PUSH_2_ID
        );
        // Shown only: the USB id never identifies it.
        assert!(push.matches_usb(ABLETON_VENDOR_ID, 0x1967));
        assert!(identify(ABLETON_VENDOR_ID, 0x1967, 0).is_none());
        assert!(identify_serial(ABLETON_VENDOR_ID, 0x1967).is_none());
        assert!(identify_evdev(ABLETON_VENDOR_ID, 0x1967).is_none());
        assert_eq!(ModelView::from(push).usb_ids, vec!["2982:1967".to_owned()]);
        assert_eq!(
            serde_json::to_value(Family::Ableton).unwrap(),
            serde_json::json!("ableton")
        );
    }

    #[test]
    fn the_xtouch_mini_is_found_by_its_port_name() {
        let mini = identify_midi(
            &midi::normalise("X-TOUCH MINI:X-TOUCH MINI MIDI 1 24:0"),
            false,
        )
        .unwrap();
        assert_eq!(
            (mini.id, mini.family, mini.protocol),
            (
                xtouch_mini::XTOUCH_MINI_ID,
                Family::Behringer,
                Protocol::BehringerXTouchMiniMidi
            )
        );
        assert!(mini.protocol.is_midi() && !mini.protocol.is_serial());
        assert!(!mini.protocol.has_brightness());
        assert_eq!(
            identify_midi(&midi::normalise("X-TOUCH MINI"), true)
                .unwrap()
                .id,
            xtouch_mini::XTOUCH_MINI_ID
        );
        // Shown only: the USB id never identifies it.
        assert!(mini.matches_usb(BEHRINGER_VENDOR_ID, 0x00b3));
        assert!(identify(BEHRINGER_VENDOR_ID, 0x00b3, 0).is_none());
        assert!(identify_serial(BEHRINGER_VENDOR_ID, 0x00b3).is_none());
        assert!(identify_evdev(BEHRINGER_VENDOR_ID, 0x00b3).is_none());
        let view = ModelView::from(mini);
        assert_eq!(view.usb_ids, vec!["1397:00b3".to_owned()]);
        assert_eq!(view.key_light_colors, "on_off");
        assert_eq!(view.dial_lights, DialLights::PerDial);
        assert!(view.key_lights && !view.brightness);
        assert_eq!(
            serde_json::to_value(Family::Behringer).unwrap(),
            serde_json::json!("behringer")
        );
    }

    #[test]
    fn nanokontrols_are_found_by_their_port_names() {
        let nk2 = identify_midi(
            &midi::normalise("nanoKONTROL2:nanoKONTROL2 MIDI 1 24:0"),
            false,
        )
        .unwrap();
        assert_eq!(
            (nk2.id, nk2.family, nk2.protocol),
            (
                korg::NANOKONTROL2_ID,
                Family::Korg,
                Protocol::KorgNanoKontrol2
            )
        );
        let studio = identify_midi(&midi::normalise("nanoKONTROL Studio"), true).unwrap();
        assert_eq!(studio.protocol, Protocol::KorgNanoKontrolStudio);
        assert!(identify_midi(&midi::normalise("nanoKONTROL"), true).is_none());
        // Shown only: the USB id never identifies it.
        assert!(nk2.matches_usb(KORG_VENDOR_ID, 0x0117));
        assert!(identify(KORG_VENDOR_ID, 0x0117, 0).is_none());
        assert!(identify_serial(KORG_VENDOR_ID, 0x0117).is_none());
        assert_eq!(ModelView::from(nk2).usb_ids, vec!["0944:0117".to_owned()]);
        assert_eq!(
            serde_json::to_value(Family::Korg).unwrap(),
            serde_json::json!("korg")
        );
    }

    #[test]
    fn the_padkontrol_is_found_by_its_native_mode_port() {
        let pad = identify_midi(&midi::normalise("padKONTROL Port 2"), false).unwrap();
        assert_eq!(
            (pad.id, pad.family, pad.protocol, pad.keys, pad.dials),
            (
                korg_padkontrol::PADKONTROL_ID,
                Family::Korg,
                Protocol::KorgPadKontrol,
                37,
                5
            )
        );
        assert!(pad.protocol.is_midi() && !pad.protocol.has_brightness());
        assert_eq!(pad.dial_light_colors(), 0);
        // Shown only: the USB id never identifies it.
        assert!(pad.matches_usb(KORG_VENDOR_ID, 0x0105));
        assert!(identify(KORG_VENDOR_ID, 0x0105, 0).is_none());
        assert!(identify_serial(KORG_VENDOR_ID, 0x0105).is_none());
        assert_eq!(ModelView::from(pad).usb_ids, vec!["0944:0105".to_owned()]);
        assert!(identify_midi(&midi::normalise("padKONTROL Port 1"), false).is_none());
        assert!(identify_midi(&midi::normalise("padKONTROL"), true).is_none());
    }

    #[test]
    fn the_midi_fighter_twister_is_found_by_its_port_name() {
        let twister = identify_midi(
            &midi::normalise("Midi Fighter Twister:Midi Fighter Twister MIDI 1 24:0"),
            false,
        )
        .unwrap();
        assert_eq!(
            (twister.id, twister.family, twister.protocol),
            (
                midi_fighter_twister::TWISTER_ID,
                Family::DjTechTools,
                Protocol::MidiFighterTwisterMidi
            )
        );
        assert!(twister.protocol.is_midi() && twister.protocol.has_brightness());
        let windows = identify_midi(&midi::normalise("Midi Fighter Twister 2"), true).unwrap();
        assert_eq!(windows.id, midi_fighter_twister::TWISTER_ID);
        // Shown only: the USB id never identifies it.
        assert!(twister.matches_usb(DJTT_VENDOR_ID, 0x0007));
        assert!(identify(DJTT_VENDOR_ID, 0x0007, 0).is_none());
        assert!(identify_serial(DJTT_VENDOR_ID, 0x0007).is_none());
        assert_eq!(
            ModelView::from(twister).usb_ids,
            vec![
                "2580:0007".to_owned(),
                "2580:1007".to_owned(),
                "2580:2007".to_owned(),
                "2580:3007".to_owned()
            ]
        );
        assert_eq!(
            serde_json::to_value(Family::DjTechTools).unwrap(),
            serde_json::json!("djtechtools")
        );
    }

    #[test]
    fn the_midi_fighter_spectra_is_found_by_its_port_name() {
        let spectra = identify_midi(
            &midi::normalise("Midi Fighter Spectra:Midi Fighter Spectra MIDI 1 24:0"),
            false,
        )
        .unwrap();
        assert_eq!(
            (spectra.id, spectra.family, spectra.protocol),
            (
                midi_fighter_spectra::SPECTRA_ID,
                Family::DjTechTools,
                Protocol::MidiFighterSpectraMidi
            )
        );
        assert!(spectra.protocol.is_midi() && spectra.protocol.has_brightness());
        assert_eq!(spectra.dial_light_colors(), 0);
        let twister = identify_midi(&midi::normalise("Midi Fighter Twister"), false).unwrap();
        assert_eq!(twister.id, midi_fighter_twister::TWISTER_ID);
        assert!(identify_midi(&midi::normalise("Midi Fighter 3D"), false).is_none());
        // Shown only: the USB id never identifies it.
        assert!(spectra.matches_usb(DJTT_VENDOR_ID, 0x0006));
        assert!(identify(DJTT_VENDOR_ID, 0x0006, 0).is_none());
        assert_eq!(
            ModelView::from(spectra).usb_ids,
            vec!["2580:0006".to_owned()]
        );
    }

    #[test]
    fn the_midi_fighter_64_is_found_by_its_port_name() {
        let mf64 = identify_midi(
            &midi::normalise("Midi Fighter 64:Midi Fighter 64 MIDI 1 24:0"),
            false,
        )
        .unwrap();
        assert_eq!(
            (mf64.id, mf64.family, mf64.protocol),
            (
                midi_fighter_64::MF64_ID,
                Family::DjTechTools,
                Protocol::MidiFighter64Midi
            )
        );
        assert!(mf64.protocol.is_midi() && mf64.protocol.has_brightness());
        assert_eq!(mf64.dial_light_colors(), 0);
        let spectra = identify_midi(&midi::normalise("Midi Fighter Spectra"), false).unwrap();
        assert_eq!(spectra.id, midi_fighter_spectra::SPECTRA_ID);
        // No product id is known: never found by USB id.
        assert!(ModelView::from(mf64).usb_ids.is_empty());
    }

    #[test]
    fn the_grid_is_found_by_its_port_name() {
        let grid = identify_midi(&midi::normalise("Grid:Grid MIDI 1 24:0"), false).unwrap();
        assert_eq!(
            (grid.id, grid.family, grid.protocol),
            (
                intech_grid::GRID_ID,
                Family::Intech,
                Protocol::IntechGridMidi
            )
        );
        assert!(grid.protocol.is_midi() && !grid.protocol.has_brightness());
        assert!(identify_midi(&midi::normalise("Grid MIDI"), false).is_none());
        // Shown only: no USB id identifies it, nor does it shadow a Wooting.
        for (vid, pid) in [
            (ATMEL_VENDOR_ID, 0xecad),
            (ATMEL_VENDOR_ID, 0xecac),
            INTECH_GRID_ESP32_USB_ID,
            INTECH_GRID_RP2350_USB_ID,
        ] {
            assert!(grid.matches_usb(vid, pid));
            assert!(identify(vid, pid, 0).is_none());
            assert!(identify_serial(vid, pid).is_none());
        }
        assert_eq!(
            ModelView::from(grid).usb_ids,
            vec![
                "03eb:ecad".to_owned(),
                "03eb:ecac".to_owned(),
                "303a:8123".to_owned(),
                "2e8a:1145".to_owned()
            ]
        );
        assert_eq!(
            serde_json::to_value(Family::Intech).unwrap(),
            serde_json::json!("intech")
        );
    }

    #[test]
    fn the_atom_is_found_by_its_port_name() {
        let atom = identify_midi(&midi::normalise("ATOM"), false).unwrap();
        assert_eq!(
            (atom.id, atom.family, atom.protocol),
            (
                presonus_atom::ATOM_ID,
                Family::Presonus,
                Protocol::PresonusAtomMidi
            )
        );
        assert!(atom.protocol.is_midi() && atom.protocol.has_brightness());
        // Shown only: the USB id never identifies it.
        assert!(atom.matches_usb(PRESONUS_VENDOR_ID, 0x0206));
        assert!(identify(PRESONUS_VENDOR_ID, 0x0206, 0).is_none());
        assert!(identify_serial(PRESONUS_VENDOR_ID, 0x0206).is_none());
        assert_eq!(ModelView::from(atom).usb_ids, vec!["194f:0206".to_owned()]);
        assert_eq!(
            serde_json::to_value(Family::Presonus).unwrap(),
            serde_json::json!("presonus")
        );
    }

    #[test]
    fn the_faderports_are_found_by_their_port_names() {
        let classic = identify_midi(&midi::normalise("FaderPort"), false).unwrap();
        assert_eq!(
            (classic.id, classic.family, classic.protocol),
            (
                faderport::FADERPORT_ID,
                Family::Presonus,
                Protocol::PresonusFaderPortMidi
            )
        );
        assert!(classic.protocol.is_midi() && !classic.protocol.has_brightness());
        assert!(ModelView::from(classic).usb_ids.is_empty());
        let fp8 = identify_midi(&midi::normalise("PreSonus FP8 Port 1"), false).unwrap();
        assert_eq!(
            (fp8.id, fp8.family, fp8.protocol),
            (
                faderport8::FADERPORT8_ID,
                Family::Presonus,
                Protocol::PresonusFaderPort8Midi
            )
        );
        assert!(fp8.protocol.is_midi() && !fp8.protocol.has_brightness());
        // Shown only: the USB ids never identify them.
        for (product, id) in [
            (0x0205, faderport8::FADERPORT2_ID),
            (0x0203, faderport8::FADERPORT8_ID),
            (0x0202, faderport8::FADERPORT8_ID),
            (0x0204, faderport8::FADERPORT16_ID),
        ] {
            assert!(by_id(id).unwrap().matches_usb(PRESONUS_VENDOR_ID, product));
            assert!(identify(PRESONUS_VENDOR_ID, product, 0).is_none());
            assert!(identify_serial(PRESONUS_VENDOR_ID, product).is_none());
        }
        assert_eq!(
            ModelView::from(fp8).usb_ids,
            vec!["194f:0203".to_owned(), "194f:0202".to_owned()]
        );
    }

    #[test]
    fn the_atom_sq_is_found_by_its_port_name() {
        let sq = identify_midi(&midi::normalise("ATM SQ"), false).unwrap();
        assert_eq!(
            (sq.id, sq.family, sq.protocol),
            (
                presonus_atom_sq::ATOM_SQ_ID,
                Family::Presonus,
                Protocol::PresonusAtomSqMidi
            )
        );
        assert!(sq.protocol.is_midi() && sq.protocol.has_brightness());
        // Shown only: the USB id never identifies it.
        assert!(sq.matches_usb(PRESONUS_VENDOR_ID, 0x020a));
        assert!(identify(PRESONUS_VENDOR_ID, 0x020a, 0).is_none());
        assert!(identify_serial(PRESONUS_VENDOR_ID, 0x020a).is_none());
        assert_eq!(ModelView::from(sq).usb_ids, vec!["194f:020a".to_owned()]);
        // The plain ATOM is another deck.
        assert_eq!(
            identify_midi(&midi::normalise("ATOM"), false).map(|found| found.id),
            Some(presonus_atom::ATOM_ID)
        );
    }

    #[test]
    fn the_beatstep_is_found_by_its_port_name() {
        let beatstep = identify_midi(
            &midi::normalise("Arturia BeatStep:Arturia BeatStep MIDI 1 20:0"),
            false,
        )
        .unwrap();
        assert_eq!(
            (beatstep.id, beatstep.family, beatstep.protocol),
            (
                arturia_beatstep::BEATSTEP_ID,
                Family::Arturia,
                Protocol::ArturiaBeatStepMidi
            )
        );
        assert!(beatstep.protocol.is_midi() && !beatstep.protocol.has_brightness());
        assert!(identify_midi(&midi::normalise("Arturia BeatStep Pro"), false).is_none());
        // Shown only: the USB id never identifies it.
        assert!(beatstep.matches_usb(ARTURIA_VENDOR_ID, 0x0206));
        assert!(identify(ARTURIA_VENDOR_ID, 0x0206, 0).is_none());
        assert!(identify_serial(ARTURIA_VENDOR_ID, 0x0206).is_none());
        assert_eq!(
            ModelView::from(beatstep).usb_ids,
            vec!["1c75:0206".to_owned()]
        );
        assert_eq!(
            serde_json::to_value(Family::Arturia).unwrap(),
            serde_json::json!("arturia")
        );
    }

    #[test]
    fn the_keylab_essential_mk3s_are_found_by_their_port_names() {
        let keylab =
            identify_midi(&midi::normalise("KeyLab Essential 61 mk3 MIDI"), false).unwrap();
        assert_eq!(
            (keylab.id, keylab.family, keylab.protocol),
            (
                arturia_keylab_essential3::KEYLAB_ESSENTIAL_61_MK3_ID,
                Family::Arturia,
                Protocol::ArturiaKeyLabEssential3Midi
            )
        );
        assert!(keylab.protocol.is_midi() && keylab.protocol.has_brightness());
        assert_eq!(keylab.dial_light_colors(), 0);
        let windows = identify_midi(&midi::normalise("KL Essential 88 mk3 MIDI"), true).unwrap();
        assert_eq!(
            windows.id,
            arturia_keylab_essential3::KEYLAB_ESSENTIAL_88_MK3_ID
        );
        assert!(
            identify_midi(&midi::normalise("MIDIIN2 (KL Essential 49 mk3 MIDI)"), true).is_none()
        );
        assert!(
            identify_midi(&midi::normalise("KeyLab Essential 49 mk3 DIN THRU"), false).is_none()
        );
        // Shown only: the USB ids never identify them.
        for product in [0x024c, 0x028c, 0x02cc] {
            assert!(identify(ARTURIA_VENDOR_ID, product, 0).is_none());
            assert!(identify_serial(ARTURIA_VENDOR_ID, product).is_none());
        }
        assert!(keylab.matches_usb(ARTURIA_VENDOR_ID, 0x028c));
        assert_eq!(
            ModelView::from(keylab).usb_ids,
            vec!["1c75:028c".to_owned()]
        );
    }

    #[test]
    fn the_mpk_minis_are_found_by_their_daw_ports() {
        let iv = identify_midi(&midi::normalise("MPK mini IV DAW Port"), false).unwrap();
        assert_eq!(
            (iv.id, iv.family, iv.protocol),
            (
                akai_mpk_mk4::MPK_MINI_IV_ID,
                Family::Akai,
                Protocol::AkaiMpkMk4Midi
            )
        );
        assert!(iv.protocol.is_midi() && iv.protocol.has_brightness());
        assert_eq!(iv.dial_light_colors(), 0);
        let plus = identify_midi(&midi::normalise("MIDIIN2 (MPK mini Plus II)"), true).unwrap();
        assert_eq!(plus.id, akai_mpk_mk4::MPK_MINI_PLUS_II_ID);
        assert!(identify_midi(&midi::normalise("MPK mini IV MIDI Port"), false).is_none());
        assert!(identify_midi(&midi::normalise("MPK mini Plus II"), true).is_none());
        // Shown only: the USB id never identifies it.
        assert!(identify(AKAI_VENDOR_ID, 0x005d, 0).is_none());
        assert!(identify_serial(AKAI_VENDOR_ID, 0x005d).is_none());
        assert!(iv.matches_usb(AKAI_VENDOR_ID, 0x005d));
        assert_eq!(ModelView::from(iv).usb_ids, vec!["09e8:005d".to_owned()]);
    }

    #[test]
    fn the_first_mpk_mini_plus_is_found_by_its_port_name() {
        let plus = identify_midi(&midi::normalise("MPK mini Plus Port 1"), false).unwrap();
        assert_eq!(
            (plus.id, plus.family, plus.protocol),
            (
                akai_mpk_mini_plus::MPK_MINI_PLUS_ID,
                Family::Akai,
                Protocol::AkaiMpkMiniPlusMidi
            )
        );
        assert!(plus.protocol.is_midi() && !plus.protocol.has_brightness());
        assert_eq!(plus.dial_light_colors(), 0);
        let linux = identify_midi(
            &midi::normalise("MPK mini Plus:MPK mini Plus MIDI 1 20:0"),
            false,
        )
        .unwrap();
        assert_eq!(linux.id, akai_mpk_mini_plus::MPK_MINI_PLUS_ID);
        // The second generation is never taken for it.
        assert!(identify_midi(&midi::normalise("MPK mini Plus II"), true).is_none());
        // Shown only: the USB id never identifies it.
        assert!(identify(AKAI_VENDOR_ID, 0x0054, 0).is_none());
        assert!(identify_serial(AKAI_VENDOR_ID, 0x0054).is_none());
        assert!(plus.matches_usb(AKAI_VENDOR_ID, 0x0054));
        assert_eq!(ModelView::from(plus).usb_ids, vec!["09e8:0054".to_owned()]);
    }

    #[test]
    fn the_minilab_3_is_found_by_its_port_name() {
        // The port names are assumed: no reference lists them.
        let minilab = identify_midi(&midi::normalise("Minilab3 MIDI"), false).unwrap();
        assert_eq!(
            (minilab.id, minilab.family, minilab.protocol),
            (
                arturia_minilab3::MINILAB3_ID,
                Family::Arturia,
                Protocol::ArturiaMiniLab3Midi
            )
        );
        assert!(minilab.protocol.is_midi() && minilab.protocol.has_brightness());
        assert_eq!(minilab.dial_light_colors(), 0);
        for other in ["Minilab3 MCU/HUI", "Minilab3 DIN THRU", "Minilab3 ALV"] {
            assert!(identify_midi(&midi::normalise(other), false).is_none());
        }
        assert!(identify_midi(&midi::normalise("MIDIIN2 (Minilab3)"), true).is_none());
        // Shown only: the USB id never identifies it.
        assert!(minilab.matches_usb(ARTURIA_VENDOR_ID, 0x220b));
        assert!(identify(ARTURIA_VENDOR_ID, 0x220b, 0).is_none());
        assert!(identify_serial(ARTURIA_VENDOR_ID, 0x220b).is_none());
        assert_eq!(
            ModelView::from(minilab).usb_ids,
            vec!["1c75:220b".to_owned()]
        );
    }

    #[test]
    fn the_icon_platforms_are_found_by_their_port_names() {
        let plus = identify_midi(&midi::normalise("Platform M+ V2.15"), false).unwrap();
        let m = identify_midi(&midi::normalise("Platform M V1.14"), false).unwrap();
        assert_eq!(
            (plus.id, m.id),
            (
                icon_platform::PLATFORM_M_PLUS_ID,
                icon_platform::PLATFORM_M_ID
            )
        );
        for model in [plus, m] {
            assert_eq!(
                (model.family, model.protocol),
                (Family::Icon, Protocol::IconPlatformMMidi)
            );
            assert!(model.protocol.is_midi() && !model.protocol.has_brightness());
            assert!(model.product_ids.is_empty() && model.more_usb_ids.is_empty());
            assert_eq!(ModelView::from(model).vendor_id, "0000");
        }
        assert!(identify_midi(&midi::normalise("Platform X1 V2.15"), false).is_none());
        let x_plus = identify_midi(&midi::normalise("Platform X+1 V2.13"), false).unwrap();
        assert_eq!(
            (x_plus.id, x_plus.family, x_plus.protocol),
            (
                icon_platform_x::PLATFORM_X_PLUS_ID,
                Family::Icon,
                Protocol::IconPlatformXPlusMidi
            )
        );
        assert!(x_plus.protocol.is_midi() && !x_plus.protocol.has_brightness());
        assert!(x_plus.product_ids.is_empty() && x_plus.more_usb_ids.is_empty());
        assert_eq!(
            serde_json::to_value(Family::Icon).unwrap(),
            serde_json::json!("icon")
        );
    }

    #[test]
    fn the_zoom_r16_r24_is_found_by_its_port_name() {
        let zoom = identify_midi(&midi::normalise("ZOOM R16_R24"), false).unwrap();
        assert_eq!(zoom.id, zoom::ZOOM_R16_R24_ID);
        assert_eq!(
            (zoom.family, zoom.protocol),
            (Family::Zoom, Protocol::ZoomR16Midi)
        );
        assert!(zoom.protocol.is_midi() && !zoom.protocol.has_brightness());
        // Its USB id is shown only, never matched.
        assert!(identify(ZOOM_VENDOR_ID, 0x00dd, 0).is_none());
        let view = ModelView::from(zoom);
        assert_eq!(
            (view.vendor_id.as_str(), view.key_light_colors),
            ("1686", "on_off")
        );
        assert_eq!(
            serde_json::to_value(Family::Zoom).unwrap(),
            serde_json::json!("zoom")
        );
    }

    #[test]
    fn the_mackie_mcu_pro_and_xt_pro_are_found_by_their_port_names() {
        let mcu = identify_midi(&midi::normalise("MCU Pro USB v3.1"), true).unwrap();
        let xt = identify_midi(&midi::normalise("MIDIIN2 (MCU Pro USB v3.1)"), true).unwrap();
        assert_eq!(
            (mcu.id, xt.id),
            (mackie_mcu::MCU_PRO_ID, mackie_mcu::XT_PRO_ID)
        );
        for model in [mcu, xt] {
            assert_eq!(model.family, Family::Mackie);
            assert!(model.protocol.is_midi() && !model.protocol.has_brightness());
        }
        assert_eq!(
            (mcu.protocol, xt.protocol),
            (Protocol::MackieMcuProMidi, Protocol::MackieXtProMidi)
        );
        // The USB id is shown only: identify() never takes a MIDI deck.
        assert!(mcu.matches_usb(MACKIE_VENDOR_ID, 0x0006));
        assert!(identify(MACKIE_VENDOR_ID, 0x0006, 0).is_none());
        assert_eq!(
            serde_json::to_value(Family::Mackie).unwrap(),
            serde_json::json!("mackie")
        );
    }

    #[test]
    fn the_icon_qcons_are_found_by_their_port_names() {
        let pro_x = identify_midi(&midi::normalise("iCON QCON Pro X V1.15"), false).unwrap();
        let xs1 = identify_midi(&midi::normalise("iCON QCON Pro XS1 V1.08"), false).unwrap();
        assert_eq!(
            (pro_x.id, xs1.id),
            (icon_qcon::PRO_X_ID, icon_qcon::PRO_XS1_ID)
        );
        for model in [pro_x, xs1] {
            assert_eq!(
                (model.family, model.protocol),
                (Family::Icon, Protocol::IconQconProMidi)
            );
            assert!(model.protocol.is_midi() && !model.protocol.has_brightness());
            assert!(model.product_ids.is_empty() && model.more_usb_ids.is_empty());
            assert_eq!(ModelView::from(model).vendor_id, "0000");
        }
    }

    #[test]
    fn the_icon_p1_nano_is_found_by_its_port_name() {
        let model = identify_midi(&midi::normalise("Icon P1-Nano MIDI 1 24:0"), false).unwrap();
        assert_eq!(model.id, icon_p1::NANO_ID);
        assert_eq!(
            (model.family, model.protocol),
            (Family::Icon, Protocol::IconP1Midi)
        );
        assert!(model.protocol.is_midi() && !model.protocol.has_brightness());
        // Shown only: a MIDI deck is never matched by USB id.
        assert!(identify(0x1d03, 0x0806, 0).is_none());
        let view = ModelView::from(model);
        assert_eq!(
            (view.vendor_id.as_str(), view.product_ids.as_slice()),
            ("1d03", &["0806".to_owned()][..])
        );
        assert_eq!(model.dial_lights, DialLights::PerDial);
        assert_eq!(model.dial_light_colors(), 8);
        assert!(identify_midi(&midi::normalise("iCON P1-M V1.00"), false).is_none());
    }

    #[test]
    fn the_icon_qcon_g2s_are_found_by_their_port_names() {
        let main = identify_midi(&midi::normalise("iCON QCON Pro G2 V1.13"), false).unwrap();
        let extender = identify_midi(&midi::normalise("iCON QCON EX1 G2 V1.13"), false).unwrap();
        assert_eq!(
            (main.id, extender.id),
            (icon_qcon::PRO_G2_ID, icon_qcon::EX_G2_ID)
        );
        for model in [main, extender] {
            assert_eq!(
                (model.family, model.protocol),
                (Family::Icon, Protocol::IconQconG2Midi)
            );
            assert!(model.protocol.is_midi() && !model.protocol.has_brightness());
            assert!(model.product_ids.is_empty() && model.more_usb_ids.is_empty());
            assert_eq!(ModelView::from(model).vendor_id, "0000");
            assert_eq!(model.dial_lights, DialLights::PerDial);
            assert_eq!(model.dial_light_colors(), 8);
        }
    }

    #[test]
    fn the_faderfox_ec4_is_found_by_its_port_name() {
        let ec4 = identify_midi(&midi::normalise("Faderfox EC4 MIDI 1"), false).unwrap();
        assert_eq!(
            (ec4.id, ec4.family, ec4.protocol),
            (
                faderfox_ec4::FADERFOX_EC4_ID,
                Family::Faderfox,
                Protocol::FaderfoxEc4Midi
            )
        );
        assert!(ec4.protocol.is_midi() && !ec4.protocol.has_brightness());
        assert!(ec4.product_ids.is_empty() && ec4.more_usb_ids.is_empty());
        assert!(identify(0, 0, 0).is_none());
        assert_eq!(ModelView::from(ec4).vendor_id, "0000");
        assert_eq!(
            serde_json::to_value(Family::Faderfox).unwrap(),
            serde_json::json!("faderfox")
        );
    }

    #[test]
    fn the_xone_k2_is_found_by_its_port_name_and_shows_allen_heaths_vendor_id() {
        let k2 = identify_midi(&midi::normalise("XONE:K2:XONE:K2 MIDI 1 20:0"), false).unwrap();
        assert_eq!(
            (k2.id, k2.family, k2.protocol, k2.keys, k2.dials),
            (
                xone_k2::XONE_K2_ID,
                Family::AllenHeath,
                Protocol::AllenHeathXoneK2Midi,
                34,
                22
            )
        );
        assert!(k2.protocol.is_midi() && !k2.protocol.has_brightness());
        assert!(k2.product_ids.is_empty() && k2.more_usb_ids.is_empty());
        assert_eq!(k2.dial_light_colors(), 0);
        assert_eq!(ModelView::from(k2).vendor_id, "22f0");
        assert!(identify(ALLEN_HEATH_VENDOR_ID, 0x0001, 0xff00).is_none());
        assert!(identify_midi(&midi::normalise("MIDIIN2 (XONE:K2)"), true).is_none());
        assert_eq!(
            serde_json::to_value(Family::AllenHeath).unwrap(),
            serde_json::json!("allen_heath")
        );
    }

    #[test]
    fn the_exquis_is_found_by_its_port_name() {
        let exquis = identify_midi(&midi::normalise("Exquis:Exquis MIDI 1 24:0"), false).unwrap();
        assert_eq!(
            (
                exquis.id,
                exquis.family,
                exquis.protocol,
                exquis.keys,
                exquis.dials
            ),
            (
                exquis::EXQUIS_ID,
                Family::IntuitiveInstruments,
                Protocol::ExquisMidi,
                75,
                4
            )
        );
        assert!(exquis.protocol.is_midi() && exquis.protocol.has_brightness());
        assert!(exquis.product_ids.is_empty() && exquis.more_usb_ids.is_empty());
        assert_eq!(exquis.dial_light_colors(), 4);
        assert_eq!(ModelView::from(exquis).vendor_id, "0000");
        assert!(identify_midi(&midi::normalise("exquis-midi"), false).is_none());
        assert_eq!(
            serde_json::to_value(Family::IntuitiveInstruments).unwrap(),
            serde_json::json!("intuitive_instruments")
        );
    }

    #[test]
    fn the_oxygen_pros_are_found_by_their_daw_port_names() {
        let found = |raw: &str, windows: bool| {
            identify_midi(&midi::normalise(raw), windows).map(|model| model.id)
        };
        assert_eq!(
            found("MIDIIN3 (Oxygen Pro 61)", true),
            Some(maudio_oxygen_pro::OXYGEN_PRO_61_ID)
        );
        assert_eq!(
            found("Oxygen Pro 25 Mackie/HUI", false),
            Some(maudio_oxygen_pro::OXYGEN_PRO_25_ID)
        );
        assert_eq!(
            found("Hammer 88 Pro Mackie/HUI", false),
            Some(maudio_oxygen_pro::HAMMER_88_PRO_ID)
        );
        for raw in [
            "Oxygen Pro 61",
            "MIDIIN2 (Oxygen Pro 61)",
            "Oxygen Pro 61 USB MIDI",
            "Oxygen 61",
        ] {
            assert_eq!(found(raw, true), None, "{raw}");
            assert_eq!(found(raw, false), None, "{raw}");
        }
        for pid in [0x0038, 0x0039, 0x003a, 0x103b, 0x003c] {
            assert!(identify(MAUDIO_VENDOR_ID, pid, 0).is_none(), "{pid:04x}");
        }
        let pro_61 = by_id(maudio_oxygen_pro::OXYGEN_PRO_61_ID).unwrap();
        assert_eq!(
            (pro_61.family, pro_61.protocol, pro_61.keys, pro_61.dials),
            (Family::MAudio, Protocol::MAudioOxygenProMidi, 48, 18)
        );
        assert!(pro_61.protocol.is_midi() && pro_61.protocol.has_brightness());
        assert_eq!(ModelView::from(pro_61).usb_ids, vec!["0763:003a"]);
        for model in MODELS.iter().filter(|model| model.family == Family::MAudio) {
            assert_eq!(model.dial_light_colors(), 0, "{}", model.id);
        }
        assert_eq!(
            serde_json::to_value(Family::MAudio).unwrap(),
            serde_json::json!("maudio")
        );
    }

    #[test]
    fn the_livid_bases_are_found_by_their_port_names() {
        let base_ii = identify_midi(&midi::normalise("Base2:Base2 MIDI 1 24:0"), false).unwrap();
        assert_eq!(
            (base_ii.id, base_ii.family, base_ii.protocol),
            (
                livid_base::BASE_II_ID,
                Family::Livid,
                Protocol::LividBaseMidi
            )
        );
        let base = identify_midi(&midi::normalise("MIDIIN1 (Base8)"), true).unwrap();
        assert_eq!(
            (base.id, base.family, base.protocol),
            (livid_base::BASE_ID, Family::Livid, Protocol::LividBaseMidi)
        );
        assert!(identify_midi(&midi::normalise("Base"), false).is_none());
        for model in [base, base_ii] {
            assert!(model.protocol.is_midi() && !model.protocol.has_brightness());
            assert!(model.product_ids.is_empty() && model.more_usb_ids.is_empty());
            assert_eq!(model.dial_light_colors(), 0);
            assert_eq!(ModelView::from(model).vendor_id, "0000");
        }
        assert_eq!(
            serde_json::to_value(Family::Livid).unwrap(),
            serde_json::json!("livid")
        );
    }

    #[test]
    fn the_bcr2000_and_bcf2000_share_a_usb_id_and_differ_by_port_name() {
        let bcr = identify_midi(&midi::normalise("BCR2000 Port 1"), false).unwrap();
        let bcf = identify_midi(&midi::normalise("BCF2000"), false).unwrap();
        assert_eq!(
            (bcr.id, bcf.id),
            (
                behringer_bcontrol::BCR2000_ID,
                behringer_bcontrol::BCF2000_ID
            )
        );
        for model in [bcr, bcf] {
            assert_eq!(
                (model.family, model.protocol, model.vendor_id),
                (
                    Family::Behringer,
                    Protocol::BehringerBControlMidi,
                    BEHRINGER_VENDOR_ID
                )
            );
            assert!(model.protocol.is_midi() && !model.protocol.has_brightness());
        }
        assert_eq!(bcr.product_ids, &[0x00bc]);
        // Never matched by USB id.
        assert!(identify(0x1397, 0x00bc, 0x0001).is_none());
    }

    #[test]
    fn the_xtouch_one_is_found_by_its_port_name() {
        let one = identify_midi(&midi::normalise("X-Touch One"), false).unwrap();
        assert_eq!(
            (one.id, one.family, one.protocol),
            (
                xtouch_one::XTOUCH_ONE_ID,
                Family::Behringer,
                Protocol::BehringerXTouchOneMidi
            )
        );
        assert!(one.protocol.is_midi() && !one.protocol.is_serial());
        assert!(!one.protocol.has_brightness());
        assert!(one.product_ids.is_empty());
        // The X-Touch Mini stays its own model.
        assert_eq!(
            identify_midi(&midi::normalise("X-TOUCH MINI"), false)
                .unwrap()
                .id,
            xtouch_mini::XTOUCH_MINI_ID
        );
        let view = ModelView::from(one);
        assert!(view.usb_ids.is_empty());
        assert_eq!(view.vendor_id, "1397");
        assert_eq!(view.key_light_colors, "on_off");
        assert_eq!(view.dial_lights, DialLights::None);
        assert_eq!(view.deck_lights, 1);
        assert!(view.key_lights && !view.brightness);
    }

    #[test]
    fn launchkeys_are_found_by_their_daw_port_alone() {
        let mini = identify_midi(
            &midi::normalise("Launchkey Mini MK3:Launchkey Mini MK3 MIDI 2 24:1"),
            false,
        )
        .unwrap();
        assert_eq!(
            (mini.id, mini.protocol),
            (launchkey::MINI_ID, Protocol::LaunchkeyMk3Midi)
        );
        assert!(identify_midi(
            &midi::normalise("Launchkey Mini MK3:Launchkey Mini MK3 MIDI 1 24:0"),
            false
        )
        .is_none());
        let k49 = identify_midi(&midi::normalise("Launchkey MK3 49 LKMK3 DAW Out"), false).unwrap();
        assert_eq!(k49.id, launchkey::K49_ID);
        assert_eq!(k49.dials, 17);
        // Shown only: the USB ids never identify a Launchkey.
        for product in [0x0102, 0x0134, 0x0135, 0x0136, 0x0137] {
            assert!(identify(NOVATION_VENDOR_ID, product, 0x0001).is_none());
            assert!(MODELS
                .iter()
                .any(|model| model.matches_usb(NOVATION_VENDOR_ID, product)
                    && model.protocol == Protocol::LaunchkeyMk3Midi));
        }
    }

    #[test]
    fn launchkey_mk2s_are_found_by_their_incontrol_port_and_show_their_ids() {
        let k25 = identify_midi(&midi::normalise("MIDIIN2 (Launchkey MIDI)"), true).unwrap();
        assert_eq!(
            (k25.id, k25.family, k25.protocol, k25.keys, k25.dials),
            (
                launchkey_mk2::K25_ID,
                Family::Novation,
                Protocol::LaunchkeyMk2Midi,
                26,
                8
            )
        );
        assert!(k25.protocol.is_midi() && k25.protocol.has_brightness());
        assert!(identify_midi(&midi::normalise("Launchkey MIDI"), true).is_none());
        let k49 = identify_midi(&midi::normalise("Launchkey MK2 49 InControl"), false).unwrap();
        assert_eq!(
            (k49.id, k49.keys, k49.dials),
            (launchkey_mk2::K49_ID, 35, 17)
        );
        assert_eq!(k49.dial_light_colors(), 0);
        // Ableton's script lists two ids for each size: 0x7b7a.. and 0x007b..
        for (product, short, id) in [
            (0x7b7a, 0x007b, launchkey_mk2::K25_ID),
            (0x7c7a, 0x007c, launchkey_mk2::K49_ID),
            (0x7d7a, 0x007d, launchkey_mk2::K61_ID),
        ] {
            for usb in [product, short] {
                assert!(identify(NOVATION_VENDOR_ID, usb, 0).is_none());
                assert!(identify_serial(NOVATION_VENDOR_ID, usb).is_none());
                assert!(by_id(id).unwrap().matches_usb(NOVATION_VENDOR_ID, usb));
            }
            let view = ModelView::from(by_id(id).unwrap());
            assert_eq!(
                view.usb_ids,
                vec![format!("1235:{product:04x}"), format!("1235:{short:04x}")]
            );
            assert_eq!(view.key_light_colors, "rgb");
            assert!(view.key_lights && view.brightness);
        }
    }

    #[test]
    fn the_first_launchkey_mini_is_found_by_its_incontrol_port_alone() {
        let mini =
            identify_midi(&midi::normalise("Launchkey Mini LK Mini InControl"), false).unwrap();
        assert_eq!(
            (mini.id, mini.family, mini.protocol, mini.keys, mini.dials),
            (
                launchkey_mini::LAUNCHKEY_MINI_ID,
                Family::Novation,
                Protocol::LaunchkeyMiniMidi,
                22,
                8
            )
        );
        assert!(mini.protocol.is_midi() && mini.protocol.has_brightness());
        assert_eq!(mini.dial_light_colors(), 0);
        // Shown only: the USB id never identifies it.
        assert!(identify(NOVATION_VENDOR_ID, 0x0035, 0).is_none());
        assert!(identify_serial(NOVATION_VENDOR_ID, 0x0035).is_none());
        assert!(mini.matches_usb(NOVATION_VENDOR_ID, 0x0035));
        assert_eq!(ModelView::from(mini).key_light_colors, "red_green");
        // The piano-key port is never taken.
        assert!(identify_midi(&midi::normalise("Launchkey Mini LK Mini MIDI"), false).is_none());
        assert!(identify_midi(&midi::normalise("Launchkey Mini"), true).is_none());
        // The Mini MK3's and MK4's ports stay theirs, and the first Mini's
        // are never taken for a Launchkey MK2, MK3 or MK4.
        for raw in [
            "Launchkey Mini MK3 DAW Port",
            "MIDIIN2 (Launchkey Mini MK3 MID",
            "Launchkey Mini MK4 25 DAW Out",
            "Launchkey Mini MK4 37:Launchkey Mini MK4 37 MIDI 2 28:1",
        ] {
            let found = identify_midi(&midi::normalise(raw), raw.starts_with("MIDI"));
            assert_ne!(
                found.map(|m| m.protocol),
                Some(Protocol::LaunchkeyMiniMidi),
                "{raw}"
            );
        }
        for (raw, windows) in [
            ("MIDIIN2 (Launchkey Mini)", true),
            ("Launchkey Mini:Launchkey Mini MIDI 2 20:1", false),
            ("Launchkey Mini MIDI 2", false),
        ] {
            assert_eq!(
                identify_midi(&midi::normalise(raw), windows).map(|m| m.id),
                Some(launchkey_mini::LAUNCHKEY_MINI_ID),
                "{raw}"
            );
        }
    }

    #[test]
    fn launchkey_mk4s_are_found_by_their_daw_port_alone() {
        let k49 = identify_midi(&midi::normalise("Launchkey MK4 49 DAW Out"), false).unwrap();
        assert_eq!(
            (k49.id, k49.family, k49.protocol),
            (
                launchkey_mk4::K49_ID,
                Family::Novation,
                Protocol::LaunchkeyMk4Midi
            )
        );
        assert_eq!(k49.dials, 17);
        let mini =
            identify_midi(&midi::normalise("MIDIIN2 (Launchkey Mini MK4 25 "), true).unwrap();
        assert_eq!(mini.id, launchkey_mk4::MINI_25_ID);
        // Never the MK3 25 that `launchkey::size_of_port` falls back to.
        let k25 = identify_midi(&midi::normalise("MIDIIN2 (Launchkey MK4 25 MIDI)"), true).unwrap();
        assert_eq!(k25.id, launchkey_mk4::K25_ID);
        assert!(identify_midi(&midi::normalise("Launchkey MK4 25 MIDI Out"), false).is_none());
        // Shown only: the USB ids never identify a Launchkey MK4.
        for product in 0x0141..=0x0146 {
            assert!(identify(NOVATION_VENDOR_ID, product, 0).is_none());
            assert!(identify_serial(NOVATION_VENDOR_ID, product).is_none());
            assert!(MODELS
                .iter()
                .any(|model| model.matches_usb(NOVATION_VENDOR_ID, product)
                    && model.protocol == Protocol::LaunchkeyMk4Midi));
        }
        assert_eq!(ModelView::from(k49).usb_ids, vec!["1235:0145".to_owned()]);
    }

    #[test]
    fn launch_control_3s_are_found_by_their_daw_port_alone() {
        let xl = identify_midi(&midi::normalise("LCXL3 1 DAW Out"), false).unwrap();
        assert_eq!(
            (xl.id, xl.family, xl.protocol),
            (
                launch_control3::XL3_ID,
                Family::Novation,
                Protocol::NovationLaunchControlXl3
            )
        );
        assert_eq!((xl.keys, xl.dials, xl.info_key()), (26, 32, Some(26)));
        let lc = identify_midi(&midi::normalise("MIDIIN2 (LC3 2)"), true).unwrap();
        assert_eq!(
            (lc.id, lc.protocol),
            (launch_control3::LC3_ID, Protocol::NovationLaunchControl3)
        );
        assert!(identify_midi(&midi::normalise("LCXL3 1 MIDI Out"), false).is_none());
        // Shown only: the USB ids never identify one.
        for product in (0x0148..=0x014f).chain(0x0157..=0x015e) {
            assert!(identify(NOVATION_VENDOR_ID, product, 0).is_none());
            assert!(identify_serial(NOVATION_VENDOR_ID, product).is_none());
            assert!(MODELS
                .iter()
                .any(|model| model.matches_usb(NOVATION_VENDOR_ID, product)
                    && matches!(
                        model.protocol,
                        Protocol::NovationLaunchControlXl3 | Protocol::NovationLaunchControl3
                    )));
        }
        assert_eq!(ModelView::from(lc).usb_ids.len(), 8);
        assert_eq!(ModelView::from(xl).usb_ids[0], "1235:0148");
    }

    #[test]
    fn launch_controls_are_found_by_port_name_alone() {
        let xl = identify_midi(
            &midi::normalise("Launch Control XL:Launch Control XL MIDI 1 24:0"),
            false,
        )
        .unwrap();
        assert_eq!(
            (xl.id, xl.family, xl.protocol),
            (
                launch_control::XL_ID,
                Family::Novation,
                Protocol::NovationLaunchControlXl
            )
        );
        let lc = identify_midi(&midi::normalise("Launch Control"), true).unwrap();
        assert_eq!(
            (lc.id, lc.protocol),
            (launch_control::LC_ID, Protocol::NovationLaunchControl)
        );
        // Their USB ids are shown and never claimed by another scan.
        for pid in [0x0061, 0x0034] {
            assert!(identify(NOVATION_VENDOR_ID, pid, 0).is_none());
            assert!(identify_serial(NOVATION_VENDOR_ID, pid).is_none());
        }
        assert!(xl.protocol.has_brightness() && lc.protocol.has_brightness());
        assert_eq!(ModelView::from(xl).usb_ids, vec!["1235:0061".to_owned()]);
        assert_eq!(ModelView::from(lc).usb_ids, vec!["1235:0034".to_owned()]);
    }

    #[test]
    fn the_dicer_is_found_by_port_name_and_shows_its_id() {
        let dicer = identify_midi(&midi::normalise("Dicer"), false).unwrap();
        assert_eq!(
            (
                dicer.id,
                dicer.family,
                dicer.protocol,
                dicer.keys,
                dicer.dials
            ),
            (
                novation_dicer::DICER_ID,
                Family::Novation,
                Protocol::NovationDicerMidi,
                10,
                0
            )
        );
        assert!(dicer.protocol.is_midi() && dicer.protocol.has_brightness());
        assert_eq!(dicer.dial_light_colors(), 0);
        assert!(identify(NOVATION_VENDOR_ID, 0x0015, 0).is_none());
        assert!(identify_serial(NOVATION_VENDOR_ID, 0x0015).is_none());
        assert!(identify_evdev(NOVATION_VENDOR_ID, 0x0015).is_none());
        let view = ModelView::from(dicer);
        assert_eq!(view.usb_ids, vec!["1235:0015".to_owned()]);
        assert_eq!(view.key_light_colors, "green_orange");
        assert!(identify_midi(&midi::normalise("MIDIIN2 (Dicer)"), true).is_none());
    }

    #[test]
    fn the_launchpad_pro_is_found_by_its_standalone_port_and_shows_its_id() {
        let pro = identify_midi(&midi::normalise("MIDIIN2 (Launchpad Pro)"), true).unwrap();
        assert_eq!(
            (pro.id, pro.family, pro.protocol, pro.keys, pro.deck_lights),
            (
                launchpad_pro::PRO_ID,
                Family::Novation,
                Protocol::LaunchpadProMidi,
                96,
                1
            )
        );
        assert!(pro.protocol.is_midi() && pro.protocol.has_brightness());
        assert!(identify(NOVATION_VENDOR_ID, 0x0051, 0).is_none());
        assert!(identify_serial(NOVATION_VENDOR_ID, 0x0051).is_none());
        assert!(identify_evdev(NOVATION_VENDOR_ID, 0x0051).is_none());
        let view = ModelView::from(pro);
        assert_eq!(view.usb_ids, vec!["1235:0051".to_owned()]);
        assert_eq!(view.key_light_colors, "rgb");
        assert!(view.key_lights && view.brightness && view.lit_keys.is_none());
        assert!(identify_midi(&midi::normalise("MIDIIN1 (Launchpad Pro)"), true).is_none());
    }

    #[test]
    fn the_launchpad_mk2_is_found_by_port_name_and_shows_its_ids() {
        let mk2 = identify_midi(
            &midi::normalise("Launchpad MK2:Launchpad MK2 MIDI 1 24:0"),
            false,
        )
        .unwrap();
        assert_eq!(
            (mk2.id, mk2.family, mk2.protocol, mk2.keys),
            (
                launchpad_mk2::MK2_ID,
                Family::Novation,
                Protocol::LaunchpadMk2Midi,
                80
            )
        );
        assert!(mk2.protocol.is_midi() && mk2.protocol.has_brightness());
        for pid in [0x0069, 0x0070, 0x0078] {
            assert!(identify(NOVATION_VENDOR_ID, pid, 0).is_none());
            assert!(identify_serial(NOVATION_VENDOR_ID, pid).is_none());
            assert!(identify_evdev(NOVATION_VENDOR_ID, pid).is_none());
        }
        let view = ModelView::from(mk2);
        assert_eq!(view.usb_ids.len(), 16);
        assert_eq!(view.usb_ids[0], "1235:0069");
        assert_eq!(view.usb_ids[15], "1235:0078");
        assert_eq!(view.key_light_colors, "rgb");
        assert!(view.key_lights && view.brightness && view.lit_keys.is_none());
        assert_eq!(view.deck_lights, 0);
    }

    #[test]
    fn bi_colour_launchpads_are_found_by_port_name_alone() {
        let original =
            identify_midi(&midi::normalise("Launchpad:Launchpad MIDI 1 24:0"), false).unwrap();
        assert_eq!(
            (original.id, original.family),
            (launchpad_legacy::LAUNCHPAD_ID, Family::Novation)
        );
        assert_eq!(
            identify_midi(&midi::normalise("Launchpad S"), true)
                .unwrap()
                .id,
            launchpad_legacy::S_ID
        );
        assert_eq!(
            identify_midi(&midi::normalise("Launchpad Mini"), false)
                .unwrap()
                .id,
            launchpad_legacy::MINI_ID
        );
        // Its USB id is shown and never claimed by another scan.
        assert!(identify(NOVATION_VENDOR_ID, 0x000e, 0).is_none());
        assert!(identify_serial(NOVATION_VENDOR_ID, 0x000e).is_none());
        let view = ModelView::from(original);
        assert_eq!(view.usb_ids, vec!["1235:000e".to_owned()]);
        assert!(view.brightness && view.key_lights);
        assert_eq!(view.key_light_colors, "red_green");
        assert!(by_id(launchpad_legacy::S_ID)
            .unwrap()
            .product_ids
            .is_empty());
    }

    #[test]
    fn every_family_is_one_the_companion_accepts() {
        // The Companion parses `/models` and `/devices` strictly: one unknown
        // family name and the whole list fails to load.
        let api = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../ui/src/api.ts"))
            .expect("the Companion's api.ts sits beside the backend");
        let start = api
            .find("const familySchema = z.enum([")
            .expect("familySchema");
        let end = start + api[start..].find("]);").expect("end of familySchema");
        // Vial boards' models are built at run time, never listed.
        let families = MODELS.iter().map(|model| (model.id, model.family)).chain([
            ("a Vial board", Family::Qmk),
            ("a LampArray", Family::LampArray),
        ]);
        for (id, family) in families {
            let name = serde_json::to_value(family).unwrap();
            let name = name.as_str().unwrap();
            assert!(
                api[start..end].contains(&format!("\"{name}\"")),
                "{id} has family {name:?}, which the Companion does not accept"
            );
        }
    }

    #[test]
    fn vial_boards_are_never_identified_by_usb_id() {
        // vial-qmk's example boards use feed:0000; any id can carry Vial, so
        // the device loop claims a board by its serial and raw-HID collection.
        assert!(identify(0xfeed, 0x0000, vial::RAW_USAGE_PAGE).is_none());
        assert!(MODELS
            .iter()
            .all(|model| model.protocol != Protocol::VialRawHid));
        assert_eq!(
            serde_json::to_value(Family::Qmk).unwrap(),
            serde_json::json!("qmk")
        );
        // The Framework macropad's Companion firmware stays its own deck.
        assert_eq!(
            identify(0x32ac, 0x0013, 0xffdd).unwrap().id,
            "framework-16-macropad"
        );
    }

    #[test]
    fn lamp_arrays_are_never_identified_by_usb_id() {
        // The Gigabyte AERO X16's keyboard (0414:8104) is a LampArray, as are
        // Dynamic Lighting keyboards, mice and DIY boards on any id: the device
        // loop claims the Lighting and Illumination collection instead.
        assert!(identify(0x0414, 0x8104, lamparray::USAGE_PAGE).is_none());
        assert!(lamparray::is_lamp_array_collection(0x59, 0x01));
        assert!(!lamparray::is_lamp_array_collection(0x01, 0x06));
        assert!(MODELS
            .iter()
            .all(|model| model.protocol != Protocol::LampArrayHid));
        assert_eq!(
            serde_json::to_value(Family::LampArray).unwrap(),
            serde_json::json!("lamp_array")
        );
        let unread = ModelView::from(&lamparray::UNREAD);
        assert_eq!((unread.keys, unread.key_lights), (0, false));
    }

    #[test]
    fn xbox_controllers_match_their_gamepad_collection() {
        for (pid, id) in [
            (0x0b12, "microsoft-xbox-series"),
            (0x0b21, "microsoft-xbox-series"),
            (0x02fd, "microsoft-xbox-one-s"),
            (0x0b22, "microsoft-xbox-elite-2"),
        ] {
            let pad = identify(0x045e, pid, 0x0001).unwrap();
            assert_eq!((pad.id, pad.family), (id, Family::Microsoft));
            assert_eq!(identify(0x045e, pid, 0).unwrap().id, id);
        }
        assert!(identify(0x045e, 0x0b0a, 0x0001).is_none(), "Adaptive");
        let protocol = Protocol::XboxGip;
        for usage in [0x04, 0x05, 0x08, 0] {
            assert!(protocol.accepts_collection(0x0001, usage));
        }
        assert!(protocol.accepts_collection(0, 0));
        assert!(!protocol.accepts_collection(0x0001, 0x06), "keyboard");
        assert!(!protocol.accepts_collection(0x000c, 0x01), "consumer");
        assert!(!protocol.accepts_collection(0xff00, 0));
        assert!(Protocol::SpaceMouseHid.accepts_collection(0x000c, 0x01));
        assert!(!protocol.wired_only(), "Bluetooth is a supported path");
        let view = ModelView::from(by_id("microsoft-xbox-series").unwrap());
        assert_eq!((view.keys, view.dials, view.deck_lights), (24, 4, 0));
        assert!(!view.brightness && !view.key_screens && !view.key_lights);
        assert_eq!(view.key_names.as_ref().unwrap()[0], "A");
        assert_eq!(view.key_names.as_ref().unwrap()[23], "RS right");
    }

    #[test]
    fn playstation_controllers_match_their_gamepad_collection_over_usb() {
        for (pid, id, protocol) in [
            (0x05c4, "sony-dualshock-4", Protocol::PlayStationDualShock4),
            (0x09cc, "sony-dualshock-4", Protocol::PlayStationDualShock4),
            (0x0ce6, "sony-dualsense", Protocol::PlayStationDualSense),
            (
                0x0df2,
                "sony-dualsense-edge",
                Protocol::PlayStationDualSense,
            ),
            (
                0x0e5f,
                "sony-access-controller",
                Protocol::PlayStationAccess,
            ),
        ] {
            let pad = identify(SONY_VENDOR_ID, pid, 0x0001).unwrap();
            assert_eq!(
                (pad.id, pad.family, pad.protocol),
                (id, Family::Sony, protocol)
            );
            assert_eq!(identify(SONY_VENDOR_ID, pid, 0).unwrap().id, id);
            assert!(protocol.accepts_collection(0x0001, 0x05));
            assert!(protocol.accepts_collection(0, 0));
            assert!(!protocol.accepts_collection(0x0001, 0x06), "keyboard");
            assert!(!protocol.accepts_collection(0xff00, 0));
            assert!(protocol.wired_only() && protocol.opens_exclusive_first());
            let view = ModelView::from(pad);
            assert_eq!(view.deck_lights, 1);
            assert!(view.brightness && !view.key_screens && !view.key_lights);
            assert_eq!(
                view.key_names.as_ref().unwrap().len(),
                usize::from(pad.keys)
            );
        }
        assert!(
            identify(SONY_VENDOR_ID, 0x0ba0, 0x0001).is_none(),
            "the DS4 dongle"
        );
        assert_eq!(
            serde_json::to_value(Family::Sony).unwrap(),
            serde_json::json!("sony")
        );
    }

    #[test]
    fn gamesir_cyclone_2_is_found_only_on_its_configuration_collection() {
        for pid in [0x100b, 0x1053, 0x101d, 0x102a] {
            let pad = identify(GAMESIR_VENDOR_ID, pid, 0xfff0).unwrap();
            assert_eq!(
                (pad.id, pad.family, pad.protocol),
                (
                    "gamesir-cyclone-2",
                    Family::GameSir,
                    Protocol::GameSirConfig
                )
            );
            assert!(identify(GAMESIR_VENDOR_ID, pid, 0x0001).is_none());
            assert!(identify(GAMESIR_VENDOR_ID, pid, 0x000c).is_none());
        }
        assert!(
            identify(GAMESIR_VENDOR_ID, 0x0575, 0xfff0).is_none(),
            "the dongle with no controller"
        );
        let protocol = Protocol::GameSirConfig;
        assert!(protocol.accepts_collection(0xfff0, 0x40));
        assert!(protocol.accepts_collection(0xfff0, 0));
        assert!(!protocol.accepts_collection(0x0001, 0x06), "keyboard");
        assert!(!protocol.accepts_collection(0x0001, 0x02), "mouse");
        assert!(!protocol.accepts_collection(0x000c, 0x01), "consumer");
        assert!(protocol.wired_only() && !protocol.opens_exclusive_first());
        let view = ModelView::from(by_id("gamesir-cyclone-2").unwrap());
        assert_eq!((view.rows, view.cols, view.keys, view.dials), (3, 9, 27, 4));
        assert_eq!(view.deck_lights, 4);
        assert!(view.brightness && !view.key_screens && !view.key_lights);
        let names = view.key_names.as_ref().unwrap();
        assert_eq!((names[16], names[18], names[26]), ("L4", "M", "RS right"));
        assert_eq!(
            serde_json::to_value(Family::GameSir).unwrap(),
            serde_json::json!("gamesir")
        );
    }

    #[test]
    fn steelseries_apex_keyboards_are_only_their_oled() {
        let rows = [
            (0x1610, "steelseries-apex-pro", ImageMode::MonoRows, 1),
            (0x1612, "steelseries-apex-7", ImageMode::MonoRows, 1),
            (0x1614, "steelseries-apex-pro-tkl", ImageMode::MonoRows, 1),
            (0x1618, "steelseries-apex-7-tkl", ImageMode::MonoRows, 1),
            (0x161c, "steelseries-apex-5", ImageMode::MonoRows, 1),
            (
                0x1640,
                "steelseries-apex-pro-gen3",
                ImageMode::MonoColumns,
                1,
            ),
            (
                0x1646,
                "steelseries-apex-pro-tkl-wireless-gen3",
                ImageMode::MonoColumns,
                3,
            ),
            (0x1644, steelseries::DONGLE_ID, ImageMode::MonoColumns, 3),
        ];
        for (pid, id, mode, interface) in rows {
            // The OLED interface's usage page is not known, so any is taken.
            let board = identify(STEELSERIES_VENDOR_ID, pid, 0xffc0).unwrap();
            assert_eq!((board.id, board.family), (id, Family::SteelSeries));
            assert_eq!(identify(STEELSERIES_VENDOR_ID, pid, 0x0001).unwrap().id, id);
            let panel = board.protocol.hid_interface().unwrap();
            assert!(panel.accepts(interface, 0x01) && panel.accepts(interface, 0));
            // The keyboard's own interface is never opened.
            assert!(!panel.accepts(0, 0x06) && !panel.accepts(0, 0));
            let image = board.info_image.unwrap();
            assert_eq!((image.width, image.height, image.mode), (128, 40, mode));
            assert_eq!((board.total_keys(), board.info_key()), (1, Some(0)));
            let view = ModelView::from(board);
            assert_eq!(view.info_face, Some([128, 40]));
            assert!(!view.brightness && !view.key_screens && !view.key_lights);
            assert_eq!((view.keys, view.dials, view.deck_lights), (0, 0, 0));
        }
        assert_eq!(
            by_id(steelseries::DONGLE_ID).unwrap().protocol,
            Protocol::SteelSeriesOledChunked
        );
        // Other interfaces keep their usage check.
        let galleon = Protocol::ElgatoGalleon.hid_interface().unwrap();
        assert!(!galleon.accepts(0, 0x06));
        assert_eq!(
            serde_json::to_value(Family::SteelSeries).unwrap(),
            serde_json::json!("steelseries")
        );
    }

    #[test]
    fn griffin_powermate_is_one_lit_dial_on_hid_and_on_linux_evdev() {
        for pid in [0x0410, 0x04aa] {
            let knob = identify(GRIFFIN_VENDOR_ID, pid, 0).unwrap();
            assert_eq!(
                (knob.id, knob.family, knob.protocol),
                (
                    "griffin-powermate",
                    Family::Griffin,
                    Protocol::GriffinPowerMate
                )
            );
            assert_eq!(identify_evdev(GRIFFIN_VENDOR_ID, pid).unwrap().id, knob.id);
        }
        assert!(identify_evdev(GRIFFIN_VENDOR_ID, 0x0001).is_none());
        // Only the PowerMate is looked for among event nodes.
        assert!(identify_evdev(XKEYS_VENDOR_ID, 0x0240).is_none());
        let view = ModelView::from(by_id("griffin-powermate").unwrap());
        assert_eq!((view.rows, view.cols, view.keys, view.dials), (0, 0, 0, 1));
        assert_eq!(view.deck_lights, 1);
        assert_eq!(view.dial_lights, DialLights::None);
        assert!(view.brightness && !view.key_screens && !view.key_lights);
        assert_eq!(
            serde_json::to_value(Family::Griffin).unwrap(),
            serde_json::json!("griffin")
        );
    }

    #[test]
    fn sony_buzz_buzzers_light_only_their_domes() {
        for pid in [0x0002, 0x1000] {
            let buzz = identify(SONY_VENDOR_ID, pid, 0).unwrap();
            assert_eq!(
                (buzz.id, buzz.family, buzz.protocol),
                ("sony-buzz", Family::Sony, Protocol::SonyBuzz)
            );
            assert_eq!((buzz.rows, buzz.cols, buzz.keys, buzz.dials), (4, 5, 20, 0));
            let view = ModelView::from(buzz);
            assert!(view.key_lights && !view.brightness && !view.key_screens);
            assert_eq!(view.key_light_colors, "on_off");
            assert_eq!(view.lit_keys.as_deref(), Some(&[0, 5, 10, 15][..]));
            let names = view.key_names.as_ref().unwrap();
            assert_eq!((names[0], names[19]), ("P1 Buzz", "P4 Yellow"));
        }
    }

    #[test]
    fn device_ids_are_single_safe_path_segments() {
        let mk2 = by_id("elgato-mk2").unwrap();
        assert_eq!(device_id(mk2, Some("CL12/K3 4")), "elgato-mk2-cl12k34");
        assert_eq!(device_id(mk2, Some("")), "elgato-mk2");
        assert_eq!(device_id(mk2, None), "elgato-mk2");
        // An all-zero serial names no Galleon in particular.
        let galleon = by_id("corsair-galleon-100-sd").unwrap();
        assert_eq!(usable_serial(galleon, Some("000000000000")), None);
        assert_eq!(usable_serial(galleon, Some("A1B2")), Some("A1B2"));
        assert_eq!(usable_serial(mk2, Some("0000")), Some("0000"));
    }

    #[test]
    fn stored_ids_resolve_to_their_model() {
        assert_eq!(
            model_for_device_id("elgato-mini-abc").unwrap().id,
            "elgato-mini"
        );
        assert_eq!(
            model_for_device_id("elgato-mini").unwrap().id,
            "elgato-mini"
        );
        assert_eq!(
            model_for_device_id("worklouder-creator-micro-2-ff01")
                .unwrap()
                .id,
            "worklouder-creator-micro-2"
        );
        assert!(model_for_device_id("elgato-minix").is_none());
        assert!(model_for_device_id("../etc").is_none());
    }

    #[test]
    fn prehkeytec_keyboards_are_found_only_on_their_poskey_collection() {
        let mci84 = identify(0x053a, 0x0b01, 0xfffa).unwrap();
        assert_eq!(
            (mci84.id, mci84.family),
            ("prehkeytec-mci-84", Family::PrehKeyTec)
        );
        assert_eq!(
            identify(0x053a, 0x0b06, 0xfffa).unwrap().id,
            "prehkeytec-mci-128"
        );
        // The ordinary keyboard interface on the same ids is never claimed.
        assert!(identify(0x053a, 0x0b01, 0x0001).is_none());
        assert!(identify(0x053a, 0x0b06, 0).is_none());
        assert!(Protocol::PrehPosKey.asks_identity());
        // The other models are reached only by the identity query.
        let mci3100 = by_id("prehkeytec-mci-3100").unwrap();
        let view = ModelView::from(mci3100);
        assert!(!view.brightness && !view.key_screens && !view.key_lights);
        assert_eq!(view.key_cells.unwrap().len(), 26);
        assert_eq!(view.key_names.unwrap()[0], "POSKey 23");
        let full = ModelView::from(by_id("prehkeytec-mci-30").unwrap());
        assert!(full.key_cells.is_none());
        assert_eq!(full.key_names.unwrap()[25], "POSKey 1");
        assert_eq!(
            serde_json::to_value(Family::PrehKeyTec).unwrap(),
            serde_json::json!("prehkeytec")
        );
        assert_eq!(
            model_for_device_id("prehkeytec-mci-128a-24342").unwrap().id,
            "prehkeytec-mci-128a"
        );
        assert_eq!(
            model_for_device_id("prehkeytec-mci-128-24342").unwrap().id,
            "prehkeytec-mci-128"
        );
    }
}
