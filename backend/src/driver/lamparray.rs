//! Microsoft HID LampArray devices: the lights Windows Dynamic Lighting drives,
//! on keyboards, mice, LED strips and DIY boards of any maker.
//!
//! A LampArray is a top-level HID collection on the Lighting and Illumination
//! page (0x59, usage 0x01) made only of feature reports. Their ids, sizes and
//! offsets are the device's own, so they are read from its report descriptor,
//! never assumed. Ryu asks for the array's attributes (lamp count, kind, the
//! shortest update interval), then for each lamp's position, colour levels and
//! key binding, and builds a model from them: every lamp up to 255 is a key,
//! placed by its position, and any lamp beyond that is a deck light.
//!
//! The collection has no input: presses on a keyboard's keys reach the computer
//! through its ordinary keyboard collection, which Ryu leaves alone, so these
//! keys are lights only. Ryu turns the device's own effect (autonomous mode)
//! off while it drives the lamps and back on when it lets go.
//!
//! Device facts come from the USB-IF HID Usage Tables 1.5, chapter 26, and were
//! cross-checked against Microsoft's ArduinoHidForWindows LampArray sample (MIT)
//! and keybackcon's host use on the Gigabyte AERO X16 (MIT). No code was copied.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use super::{Driver, DriverError, DriverResult, Face, Input, Transport};
use crate::catalog::{DeviceModel, DialLights, Family, Protocol, StripPlacement};

/// The Lighting and Illumination usage page.
pub const USAGE_PAGE: u16 = 0x59;
/// The LampArray application collection's usage.
pub const LAMP_ARRAY_USAGE: u16 = 0x01;
/// The most bytes a report descriptor may have (hidapi's limit).
pub const DESCRIPTOR_CAPACITY: usize = 4096;
/// Lamps beyond this make the device too slow to interrogate; it is refused.
pub const MAX_LAMPS: u16 = 1024;
/// Keys are numbered in a byte.
const MAX_KEYS: usize = 255;
/// Lamps whose Y positions are this close (in micrometres) share a row.
const ROW_TOLERANCE_UM: u32 = 4000;
/// Columns a layout with no usable positions is wrapped at.
const WRAP_COLUMNS: usize = 16;
/// The longest wait between updates Ryu honours, whatever the device asks.
const LONGEST_INTERVAL: Duration = Duration::from_secs(1);

/// Usages on the Lighting and Illumination page.
mod usage {
    pub const LAMP_COUNT: u16 = 0x03;
    pub const BOUNDING_BOX_WIDTH: u16 = 0x04;
    pub const BOUNDING_BOX_HEIGHT: u16 = 0x05;
    pub const BOUNDING_BOX_DEPTH: u16 = 0x06;
    pub const KIND: u16 = 0x07;
    pub const MIN_UPDATE_INTERVAL: u16 = 0x08;
    pub const LAMP_ID: u16 = 0x21;
    pub const POSITION_X: u16 = 0x23;
    pub const POSITION_Y: u16 = 0x24;
    pub const POSITION_Z: u16 = 0x25;
    pub const PURPOSES: u16 = 0x26;
    pub const RED_LEVELS: u16 = 0x28;
    pub const GREEN_LEVELS: u16 = 0x29;
    pub const BLUE_LEVELS: u16 = 0x2a;
    pub const INTENSITY_LEVELS: u16 = 0x2b;
    pub const IS_PROGRAMMABLE: u16 = 0x2c;
    pub const INPUT_BINDING: u16 = 0x2d;
    pub const RED: u16 = 0x51;
    pub const GREEN: u16 = 0x52;
    pub const BLUE: u16 = 0x53;
    pub const INTENSITY: u16 = 0x54;
    pub const UPDATE_FLAGS: u16 = 0x55;
    pub const LAMP_ID_START: u16 = 0x61;
    pub const LAMP_ID_END: u16 = 0x62;
    pub const AUTONOMOUS_MODE: u16 = 0x71;
}

/// The colour channels of an update, in the order Ryu keeps them.
const CHANNELS: [u16; 4] = [usage::RED, usage::GREEN, usage::BLUE, usage::INTENSITY];

/// `LampUpdateComplete`: the device shows everything sent so far.
const UPDATE_COMPLETE: u64 = 0x01;

/// The array is a keyboard (input bindings are keyboard usages).
const KIND_KEYBOARD: u32 = 1;
/// The array is a mouse (input bindings are button numbers).
const KIND_MOUSE: u32 = 2;

/// Whether an enumerated HID collection is a LampArray.
#[must_use]
pub const fn is_lamp_array_collection(usage_page: u16, usage: u16) -> bool {
    usage_page == USAGE_PAGE && usage == LAMP_ARRAY_USAGE
}

/// One field of a feature report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Field {
    usage: u16,
    /// Bits from the end of the report id byte.
    offset: u32,
    size: u32,
    logical_max: i64,
}

/// One feature report of the LampArray collection, as its descriptor lays it out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    id: u8,
    bits: u32,
    fields: Vec<Field>,
}

impl Report {
    /// Bytes on the wire, the report id included.
    fn len(&self) -> usize {
        1 + usize::try_from(self.bits.div_ceil(8)).unwrap_or(0)
    }

    fn has(&self, usage: u16) -> bool {
        self.fields.iter().any(|field| field.usage == usage)
    }

    fn count(&self, usage: u16) -> usize {
        self.fields
            .iter()
            .filter(|field| field.usage == usage)
            .count()
    }

    /// The `nth` field with this usage.
    fn field(&self, usage: u16, nth: usize) -> Option<&Field> {
        self.fields
            .iter()
            .filter(|field| field.usage == usage)
            .nth(nth)
    }

    /// An empty report, its id in place.
    fn buffer(&self) -> Vec<u8> {
        let mut buf = vec![0; self.len()];
        buf[0] = self.id;
        buf
    }

    /// The value of the first field with this usage, or 0 when there is none.
    fn read(&self, buf: &[u8], usage: u16) -> u64 {
        self.field(usage, 0)
            .map_or(0, |field| read_bits(buf, 8 + field.offset, field.size))
    }

    fn write(&self, buf: &mut [u8], usage: u16, nth: usize, value: u64) {
        if let Some(field) = self.field(usage, nth) {
            write_bits(buf, 8 + field.offset, field.size, value);
        }
    }
}

fn read_bits(buf: &[u8], start: u32, size: u32) -> u64 {
    let mut value = 0;
    for bit in 0..size.min(64) {
        let at = start + bit;
        let set = usize::try_from(at / 8)
            .ok()
            .and_then(|byte| buf.get(byte))
            .is_some_and(|byte| byte >> (at % 8) & 1 == 1);
        if set {
            value |= 1 << bit;
        }
    }
    value
}

fn write_bits(buf: &mut [u8], start: u32, size: u32, value: u64) {
    for bit in 0..size.min(64) {
        let at = start + bit;
        let Some(byte) = usize::try_from(at / 8)
            .ok()
            .and_then(|byte| buf.get_mut(byte))
        else {
            return;
        };
        let mask = 1 << (at % 8);
        if value >> bit & 1 == 1 {
            *byte |= mask;
        } else {
            *byte &= !mask;
        }
    }
}

/// Where the LampArray's six reports are, read from its report descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    attributes: Report,
    request: Report,
    response: Report,
    multi: Option<Report>,
    range: Option<Report>,
    control: Option<Report>,
    /// Lamps one multi-update report carries.
    slots: usize,
}

/// A local Usage, Usage Minimum or Usage Maximum. A 4-byte one carries its
/// own usage page; a shorter one takes the page in force at the main item.
#[derive(Debug, Clone, Copy)]
struct LocalUsage {
    value: u32,
    has_page: bool,
}

impl LocalUsage {
    const fn resolve(self, page: u16) -> u32 {
        if self.has_page {
            self.value
        } else {
            (page as u32) << 16 | (self.value & 0xffff)
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct Globals {
    page: u16,
    logical_min: i64,
    /// Logical Maximum as signed and as unsigned; which applies depends on
    /// whether the minimum is negative.
    logical_max: (i64, i64),
    size: u32,
    count: u32,
    id: u8,
}

impl Globals {
    const fn logical_max(&self) -> i64 {
        if self.logical_min >= 0 && self.logical_max.0 < 0 {
            self.logical_max.1
        } else {
            self.logical_max.0
        }
    }
}

#[derive(Default)]
struct Locals {
    usages: Vec<LocalUsage>,
    minimum: Option<LocalUsage>,
    maximum: Option<LocalUsage>,
}

impl Locals {
    /// The usage of the `n`th value of a main item: the list's, the last one
    /// repeating, or the range's, stopping at its maximum.
    fn usage(&self, n: u32, page: u16) -> Option<u32> {
        if let Some(last) = self.usages.last() {
            let index = usize::try_from(n).unwrap_or(usize::MAX);
            return Some(self.usages.get(index).unwrap_or(last).resolve(page));
        }
        let minimum = self.minimum?.resolve(page);
        let maximum = self
            .maximum
            .map_or(minimum, |maximum| maximum.resolve(page));
        Some(minimum.saturating_add(n).min(maximum.max(minimum)))
    }
}

/// Reads a report descriptor and finds the LampArray's reports in it. `None`
/// when it has no LampArray collection, or one Ryu cannot drive.
#[must_use]
pub fn parse_descriptor(bytes: &[u8]) -> Option<Layout> {
    let mut globals = Globals::default();
    let mut stack: Vec<Globals> = Vec::new();
    let mut locals = Locals::default();
    let mut depth = 0usize;
    let mut array_depth: Option<usize> = None;
    // Feature report id -> bits so far and the LampArray fields in it.
    let mut reports: Vec<Report> = Vec::new();
    let mut at = 0;
    while let Some(&prefix) = bytes.get(at) {
        if prefix == 0xfe {
            // A long item: its size is the next byte; nothing here uses one.
            let size = usize::from(*bytes.get(at + 1)?);
            at += 3 + size;
            continue;
        }
        let size = match prefix & 0x03 {
            3 => 4,
            other => usize::from(other),
        };
        let data = bytes.get(at + 1..at + 1 + size)?;
        at += 1 + size;
        let unsigned = data
            .iter()
            .rev()
            .fold(0u32, |value, byte| value << 8 | u32::from(*byte));
        let signed = match size {
            1 => i64::from(unsigned as u8 as i8),
            2 => i64::from(unsigned as u16 as i16),
            4 => i64::from(unsigned as i32),
            _ => 0,
        };
        let local = |value| LocalUsage {
            value,
            has_page: size == 4,
        };
        match prefix & 0xfc {
            // Input and Output: other directions, not part of feature reports.
            0x80 | 0x90 => locals = Locals::default(),
            // Feature.
            0xb0 => {
                let report = match reports.iter_mut().find(|report| report.id == globals.id) {
                    Some(report) => report,
                    None => {
                        reports.push(Report {
                            id: globals.id,
                            bits: 0,
                            fields: Vec::new(),
                        });
                        reports.last_mut()?
                    }
                };
                for n in 0..globals.count {
                    let usage = locals.usage(n, globals.page);
                    let offset = report.bits.checked_add(n.checked_mul(globals.size)?)?;
                    // Constant items still carry values here (the attributes
                    // report is all constant), so only usages decide.
                    if let Some(usage) = usage.filter(|usage| usage >> 16 == u32::from(USAGE_PAGE))
                    {
                        if array_depth.is_some() && globals.size <= 32 {
                            report.fields.push(Field {
                                usage: (usage & 0xffff) as u16,
                                offset,
                                size: globals.size,
                                logical_max: globals.logical_max(),
                            });
                        }
                    }
                }
                report.bits = report
                    .bits
                    .checked_add(globals.count.checked_mul(globals.size)?)?;
                if report.bits > 8 * DESCRIPTOR_CAPACITY as u32 {
                    return None;
                }
                locals = Locals::default();
            }
            // Collection.
            0xa0 => {
                depth += 1;
                let usage = locals.usage(0, globals.page);
                if array_depth.is_none()
                    && unsigned == 0x01
                    && usage == Some(u32::from(USAGE_PAGE) << 16 | u32::from(LAMP_ARRAY_USAGE))
                {
                    array_depth = Some(depth);
                }
                locals = Locals::default();
            }
            // End Collection.
            0xc0 => {
                if array_depth == Some(depth) {
                    array_depth = None;
                }
                depth = depth.saturating_sub(1);
                locals = Locals::default();
            }
            0x04 => globals.page = unsigned as u16,
            0x14 => globals.logical_min = signed,
            0x24 => globals.logical_max = (signed, i64::from(unsigned)),
            0x74 => globals.size = unsigned,
            0x84 => globals.id = u8::try_from(unsigned).ok()?,
            0x94 => globals.count = unsigned.min(u32::from(u16::MAX)),
            0xa4 => stack.push(globals),
            0xb4 => globals = stack.pop()?,
            0x08 => locals.usages.push(local(unsigned)),
            0x18 => locals.minimum = Some(local(unsigned)),
            0x28 => locals.maximum = Some(local(unsigned)),
            // Physical extents, units, designators, strings, delimiters: no
            // bearing on where a value sits.
            _ => {}
        }
    }
    layout(reports)
}

/// Names the LampArray's reports by the usages in them, so the order and
/// numbering the device chose (or Windows rebuilt) does not matter.
fn layout(reports: Vec<Report>) -> Option<Layout> {
    let mut reports: Vec<Report> = reports
        .into_iter()
        .filter(|report| !report.fields.is_empty())
        .collect();
    // A LampArray needs several feature reports, which need report ids.
    if reports.iter().any(|report| report.id == 0) {
        return None;
    }
    let mut take = |matches: &dyn Fn(&Report) -> bool| {
        let index = reports.iter().position(matches)?;
        Some(reports.remove(index))
    };
    let attributes = take(&|report| report.has(usage::KIND) && report.has(usage::LAMP_COUNT))?;
    let response = take(&|report| report.has(usage::LAMP_ID) && report.has(usage::POSITION_X))?;
    let multi = take(&|report| report.has(usage::LAMP_ID) && report.has(usage::RED));
    let request = take(&|report| {
        report.has(usage::LAMP_ID) && !report.has(usage::RED) && !report.has(usage::POSITION_X)
    })?;
    let range = take(&|report| report.has(usage::LAMP_ID_START) && report.has(usage::LAMP_ID_END));
    let control = take(&|report| report.has(usage::AUTONOMOUS_MODE));
    let slots = multi.as_ref().map_or(0, |multi| {
        let declared = multi
            .field(usage::LAMP_COUNT, 0)
            .map_or(0, |field| usize::try_from(field.logical_max).unwrap_or(0));
        CHANNELS
            .iter()
            .map(|channel| multi.count(*channel))
            .chain([declared, multi.count(usage::LAMP_ID)])
            .min()
            .unwrap_or(0)
    });
    let multi = multi.filter(|_| slots > 0);
    if multi.is_none() && range.is_none() {
        return None;
    }
    Some(Layout {
        attributes,
        request,
        response,
        multi,
        range,
        control,
        slots,
    })
}

/// What a lamp reported about itself. Its id is its index in [`Array::lamps`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lamp {
    /// X, Y and Z in micrometres from the array's top-left-far corner.
    pub position: [u32; 3],
    pub purposes: u32,
    /// Red, green, blue and intensity level counts.
    pub levels: [u32; 4],
    /// Takes any colour; one that is not has a fixed colour and only an intensity.
    pub programmable: bool,
    /// The key (keyboard usage) or button the lamp sits under, or 0.
    pub binding: u32,
}

/// What reading a LampArray told Ryu. It never changes while the device stays
/// plugged in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Array {
    pub layout: Layout,
    pub kind: u32,
    /// Width, height and depth in micrometres.
    pub bounding_box: [u32; 3],
    pub min_update_interval_us: u32,
    pub lamps: Vec<Lamp>,
}

impl Array {
    /// The shortest gap between two finished updates.
    fn interval(&self) -> Duration {
        Duration::from_micros(u64::from(self.min_update_interval_us)).min(LONGEST_INTERVAL)
    }

    /// Brightness goes on the intensity channel only when every lamp has more
    /// than on and off there; otherwise it scales the colours.
    fn dims_by_intensity(&self) -> bool {
        self.lamps.iter().all(|lamp| lamp.levels[3] > 1)
    }
}

fn get_report(transport: &mut dyn Transport, report: &Report) -> DriverResult<Vec<u8>> {
    let mut buf = report.buffer();
    let len = transport.get_feature_report(&mut buf)?;
    if len == 0 {
        return Err(DriverError::Io(format!(
            "feature report {} came back empty",
            report.id
        )));
    }
    Ok(buf)
}

fn ask_for_lamp(transport: &mut dyn Transport, layout: &Layout, id: u16) -> DriverResult<()> {
    let mut buf = layout.request.buffer();
    layout
        .request
        .write(&mut buf, usage::LAMP_ID, 0, u64::from(id));
    transport.send_feature_report(&buf)
}

fn read_u32(report: &Report, buf: &[u8], usage: u16) -> u32 {
    u32::try_from(report.read(buf, usage)).unwrap_or(u32::MAX)
}

/// Reads the array's attributes and every lamp's, through the LampArray
/// collection of an opened device whose report descriptor is `descriptor`.
pub fn probe(transport: &mut dyn Transport, descriptor: &[u8]) -> DriverResult<Array> {
    let layout = parse_descriptor(descriptor).ok_or_else(|| {
        DriverError::Refused(
            "Ryu could not read this device's lighting reports (HID LampArray).".to_owned(),
        )
    })?;
    let attributes = get_report(transport, &layout.attributes)?;
    let report = &layout.attributes;
    let count = report.read(&attributes, usage::LAMP_COUNT);
    if count == 0 {
        return Err(DriverError::Refused(
            "This device reports no lamps to light.".to_owned(),
        ));
    }
    let count = u16::try_from(count)
        .ok()
        .filter(|count| *count <= MAX_LAMPS)
        .ok_or_else(|| {
            DriverError::Refused(format!(
                "This device has {count} lamps; Ryu drives devices with up to {MAX_LAMPS}."
            ))
        })?;
    let kind = read_u32(report, &attributes, usage::KIND);
    let bounding_box = [
        read_u32(report, &attributes, usage::BOUNDING_BOX_WIDTH),
        read_u32(report, &attributes, usage::BOUNDING_BOX_HEIGHT),
        read_u32(report, &attributes, usage::BOUNDING_BOX_DEPTH),
    ];
    let min_update_interval_us = read_u32(report, &attributes, usage::MIN_UPDATE_INTERVAL);

    // The device answers lamp after lamp from the one asked for, wrapping to
    // 0, and answers an id it does not have as lamp 0: each answer is checked.
    ask_for_lamp(transport, &layout, 0)?;
    let response = &layout.response;
    let mut lamps = Vec::with_capacity(usize::from(count));
    for id in 0..count {
        let mut buf = get_report(transport, response)?;
        if response.read(&buf, usage::LAMP_ID) != u64::from(id) {
            ask_for_lamp(transport, &layout, id)?;
            buf = get_report(transport, response)?;
            if response.read(&buf, usage::LAMP_ID) != u64::from(id) {
                return Err(DriverError::Refused(format!(
                    "This device did not describe its lamp {id}."
                )));
            }
        }
        lamps.push(Lamp {
            position: [
                read_u32(response, &buf, usage::POSITION_X),
                read_u32(response, &buf, usage::POSITION_Y),
                read_u32(response, &buf, usage::POSITION_Z),
            ],
            purposes: read_u32(response, &buf, usage::PURPOSES),
            levels: [
                read_u32(response, &buf, usage::RED_LEVELS),
                read_u32(response, &buf, usage::GREEN_LEVELS),
                read_u32(response, &buf, usage::BLUE_LEVELS),
                read_u32(response, &buf, usage::INTENSITY_LEVELS),
            ],
            programmable: response.read(&buf, usage::IS_PROGRAMMABLE) != 0,
            binding: read_u32(response, &buf, usage::INPUT_BINDING),
        });
    }
    Ok(Array {
        layout,
        kind,
        bounding_box,
        min_update_interval_us,
        lamps,
    })
}

/// Stands in for a LampArray that could not be read, so the reason can be
/// shown. Never driven.
pub static UNREAD: DeviceModel = DeviceModel {
    id: "lamparray",
    name: "Dynamic Lighting device",
    family: Family::LampArray,
    vendor_id: 0,
    product_ids: &[],
    more_usb_ids: &[],
    protocol: Protocol::LampArrayHid,
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
    deck_lights: 0,
    key_lights: false,
    usage_page: Some(USAGE_PAGE),
};

struct Registered {
    model: &'static DeviceModel,
    array: Arc<Array>,
    names: Vec<&'static str>,
    cells: Vec<u16>,
}

/// Every LampArray model built this run, each leaked once and reused when the
/// same device comes back.
static REGISTRY: Mutex<Vec<Registered>> = Mutex::new(Vec::new());

fn leak(text: String) -> &'static str {
    Box::leak(text.into_boxed_str())
}

/// FNV-1a, so a device keeps its model id from one run (and Rust release) to
/// the next.
fn fingerprint(vendor_id: u16, product_id: u16, array: &Array) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |value: u32| {
        for byte in value.to_le_bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
    };
    eat(u32::from(vendor_id) << 16 | u32::from(product_id));
    eat(array.kind);
    eat(u32::try_from(array.lamps.len()).unwrap_or(u32::MAX));
    for lamp in &array.lamps {
        lamp.position.iter().for_each(|value| eat(*value));
        eat(lamp.purposes);
        lamp.levels.iter().for_each(|value| eat(*value));
        eat(u32::from(lamp.programmable));
        eat(lamp.binding);
    }
    hash
}

/// The model for a device that answered [`probe`], named after its USB
/// product string.
pub fn register(
    array: Array,
    vendor_id: u16,
    product_id: u16,
    product: Option<&str>,
) -> &'static DeviceModel {
    let name = product
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or("Dynamic Lighting device");
    let mut registry = REGISTRY.lock().unwrap_or_else(PoisonError::into_inner);
    let known = registry.iter().find(|entry| {
        *entry.array == array
            && entry.model.vendor_id == vendor_id
            && entry.model.product_ids == [product_id]
            && entry.model.name == name
    });
    if let Some(entry) = known {
        return entry.model;
    }
    let keys = array.lamps.len().min(MAX_KEYS);
    let positions: Vec<[u32; 2]> = array.lamps[..keys]
        .iter()
        .map(|lamp| [lamp.position[0], lamp.position[1]])
        .collect();
    let grid = grid(&positions);
    let model: &'static DeviceModel = Box::leak(Box::new(DeviceModel {
        id: leak(format!(
            "lamparray-{vendor_id:04x}-{product_id:04x}-{:016x}",
            fingerprint(vendor_id, product_id, &array)
        )),
        name: leak(name.to_owned()),
        family: Family::LampArray,
        vendor_id,
        product_ids: Box::leak(Box::new([product_id])),
        more_usb_ids: &[],
        protocol: Protocol::LampArrayHid,
        rows: grid.rows,
        cols: grid.cols,
        keys: u8::try_from(keys).unwrap_or(u8::MAX),
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
        deck_lights: u8::try_from(array.lamps.len() - keys).unwrap_or(u8::MAX),
        key_lights: true,
        usage_page: Some(USAGE_PAGE),
    }));
    let names = lamp_names(array.kind, &array.lamps[..keys])
        .into_iter()
        .map(leak)
        .collect();
    registry.push(Registered {
        model,
        array: Arc::new(array),
        names,
        cells: grid.cells,
    });
    model
}

fn registered<T>(model: &DeviceModel, read: impl FnOnce(&Registered) -> T) -> Option<T> {
    let registry = REGISTRY.lock().unwrap_or_else(PoisonError::into_inner);
    registry
        .iter()
        .find(|entry| std::ptr::eq(entry.model, model))
        .map(read)
}

/// Every LampArray model built so far, oldest first.
#[must_use]
pub fn models() -> Vec<&'static DeviceModel> {
    let registry = REGISTRY.lock().unwrap_or_else(PoisonError::into_inner);
    registry.iter().map(|entry| entry.model).collect()
}

/// What the probe learned about a LampArray model's device.
#[must_use]
pub fn array(model: &DeviceModel) -> Option<Arc<Array>> {
    registered(model, |entry| Arc::clone(&entry.array))
}

/// Each key's name: the key or button its lamp sits under, or its lamp number.
#[must_use]
pub fn key_names(model: &DeviceModel) -> Option<Vec<&'static str>> {
    registered(model, |entry| entry.names.clone())
}

/// Each key's cell in the model's grid.
#[must_use]
pub fn key_cells(model: &DeviceModel) -> Option<Vec<u16>> {
    registered(model, |entry| entry.cells.clone())
}

/// Rows, columns and each lamp's cell, from lamp positions.
#[derive(Debug, PartialEq, Eq)]
struct Grid {
    rows: u8,
    cols: u8,
    cells: Vec<u16>,
}

/// Lays lamps out by their X and Y: lamps within [`ROW_TOLERANCE_UM`] of a
/// row's top share it, and columns step by the smallest gap between two lamps
/// of a row, so a staggered keyboard keeps its shape. Lamps with no usable
/// positions, or spread too thin for that, fill the rows in order.
fn grid(positions: &[[u32; 2]]) -> Grid {
    let count = positions.len();
    if count == 0 {
        return Grid {
            rows: 0,
            cols: 0,
            cells: Vec::new(),
        };
    }
    let wrapped = || {
        let cols = count.min(WRAP_COLUMNS);
        Grid {
            rows: u8::try_from(count.div_ceil(cols)).unwrap_or(u8::MAX),
            cols: u8::try_from(cols).unwrap_or(u8::MAX),
            cells: (0..count)
                .map(|index| u16::try_from(index).unwrap_or(u16::MAX))
                .collect(),
        }
    };
    if positions.iter().all(|position| *position == positions[0]) {
        return wrapped();
    }
    let mut order: Vec<usize> = (0..count).collect();
    order.sort_by_key(|index| (positions[*index][1], positions[*index][0]));
    let mut rows: Vec<Vec<usize>> = Vec::new();
    let mut row_top = 0;
    for index in order {
        let y = positions[index][1];
        match rows.last_mut() {
            Some(row) if y - row_top <= ROW_TOLERANCE_UM => row.push(index),
            _ => {
                row_top = y;
                rows.push(vec![index]);
            }
        }
    }
    for row in &mut rows {
        row.sort_by_key(|index| positions[*index][0]);
    }
    let pitch = rows
        .iter()
        .flat_map(|row| {
            row.windows(2)
                .map(|pair| positions[pair[1]][0] - positions[pair[0]][0])
        })
        .filter(|gap| *gap > 0)
        .min();
    let left = positions
        .iter()
        .map(|position| position[0])
        .min()
        .unwrap_or(0);
    let widest = rows.iter().map(Vec::len).max().unwrap_or(0);
    let mut places = vec![(0usize, 0usize); count];
    for (row_index, row) in rows.iter().enumerate() {
        let mut next = 0;
        for index in row {
            let measured = pitch.map_or(0, |pitch| {
                let offset = positions[*index][0] - left;
                usize::try_from((u64::from(offset) + u64::from(pitch) / 2) / u64::from(pitch))
                    .unwrap_or(usize::MAX)
            });
            let col = measured.max(next);
            next = col.saturating_add(1);
            places[*index] = (row_index, col);
        }
    }
    let cols = places.iter().map(|(_, col)| col + 1).max().unwrap_or(1);
    // Sparse placements (a few lamps far apart) would make a huge, empty grid.
    let cols = if cols > MAX_KEYS || cols > widest.max(4) * 3 {
        for row in &rows {
            for (col, index) in row.iter().enumerate() {
                places[*index].1 = col;
            }
        }
        widest
    } else {
        cols
    };
    if rows.len() > MAX_KEYS || cols > MAX_KEYS {
        return wrapped();
    }
    Grid {
        rows: u8::try_from(rows.len()).unwrap_or(u8::MAX),
        cols: u8::try_from(cols).unwrap_or(u8::MAX),
        cells: places
            .iter()
            .map(|(row, col)| u16::try_from(row * cols + col).unwrap_or(u16::MAX))
            .collect(),
    }
}

/// The key a keyboard usage (HID page 0x07) names, as printed on a US layout.
fn keyboard_usage_name(code: u32) -> Option<String> {
    const LETTERS: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    const DIGITS: &str = "1234567890";
    const PUNCTUATION: [&str; 30] = [
        "Enter",
        "Esc",
        "Backspace",
        "Tab",
        "Space",
        "-",
        "=",
        "[",
        "]",
        "\\",
        "#",
        ";",
        "'",
        "`",
        ",",
        ".",
        "/",
        "Caps Lock",
        "F1",
        "F2",
        "F3",
        "F4",
        "F5",
        "F6",
        "F7",
        "F8",
        "F9",
        "F10",
        "F11",
        "F12",
    ];
    const NAVIGATION: [&str; 20] = [
        "Print Screen",
        "Scroll Lock",
        "Pause",
        "Insert",
        "Home",
        "Page Up",
        "Delete",
        "End",
        "Page Down",
        "Right",
        "Left",
        "Down",
        "Up",
        "Num Lock",
        "Keypad /",
        "Keypad *",
        "Keypad -",
        "Keypad +",
        "Keypad Enter",
        "Keypad 1",
    ];
    const MODIFIERS: [&str; 8] = [
        "Left Ctrl",
        "Left Shift",
        "Left Alt",
        "Left GUI",
        "Right Ctrl",
        "Right Shift",
        "Right Alt",
        "Right GUI",
    ];
    let index = |base: u32| usize::try_from(code - base).ok();
    let name = match code {
        0x04..=0x1d => LETTERS.get(index(0x04)?..=index(0x04)?)?.to_owned(),
        0x1e..=0x27 => DIGITS.get(index(0x1e)?..=index(0x1e)?)?.to_owned(),
        0x28..=0x45 => (*PUNCTUATION.get(index(0x28)?)?).to_owned(),
        0x46..=0x59 => (*NAVIGATION.get(index(0x46)?)?).to_owned(),
        0x5a..=0x61 => format!("Keypad {}", code - 0x58),
        0x62 => "Keypad 0".to_owned(),
        0x63 => "Keypad .".to_owned(),
        0x64 => "\\ (ISO)".to_owned(),
        0x65 => "Menu".to_owned(),
        0x68..=0x73 => format!("F{}", code - 0x68 + 13),
        0xe0..=0xe7 => (*MODIFIERS.get(index(0xe0)?)?).to_owned(),
        _ => return None,
    };
    Some(name)
}

/// Each lamp's key name: the key or mouse button it lights, where the array
/// says, or its lamp number. Two lamps under one key are numbered apart.
fn lamp_names(kind: u32, lamps: &[Lamp]) -> Vec<String> {
    let mut names: Vec<String> = Vec::with_capacity(lamps.len());
    for (index, lamp) in lamps.iter().enumerate() {
        let base = match (kind, lamp.binding) {
            (_, 0) => None,
            (KIND_KEYBOARD, code) => {
                Some(keyboard_usage_name(code).unwrap_or_else(|| format!("Key {code:#04x}")))
            }
            (KIND_MOUSE, button) => Some(format!("Button {button}")),
            _ => None,
        };
        let name = match base {
            Some(base) => {
                let taken = names
                    .iter()
                    .filter(|name| **name == base || name.starts_with(&format!("{base} (")))
                    .count();
                if taken == 0 {
                    base
                } else {
                    format!("{base} ({})", taken + 1)
                }
            }
            None => format!("Lamp {}", index + 1),
        };
        names.push(name);
    }
    names
}

/// `value` of 255 in `levels` steps, rounded.
/// Done in 64 bits: a device may report a level count near `u32::MAX`.
fn quantise(value: u32, levels: u32) -> u32 {
    let steps = (u64::from(value.min(255)) * u64::from(levels) + 127) / 255;
    u32::try_from(steps.min(u64::from(levels))).unwrap_or(levels)
}

/// `levels` scaled by `percent`, rounded.
fn scale(levels: u32, percent: u8) -> u32 {
    let steps = (u64::from(levels) * u64::from(percent) + 50) / 100;
    u32::try_from(steps.min(u64::from(levels))).unwrap_or(levels)
}

/// The red, green, blue and intensity values that show `color` on `lamp`.
/// Every value stays within the lamp's level counts, since one value past them
/// makes the device drop the whole report.
fn encode(lamp: &Lamp, color: Option<[u8; 3]>, percent: u8, dims_by_intensity: bool) -> [u32; 4] {
    let Some(color) = color.filter(|color| color.iter().any(|channel| *channel > 0)) else {
        return [0; 4];
    };
    let [red, green, blue, intensity] = lamp.levels;
    if !lamp.programmable {
        // A fixed-colour lamp: only how bright, and never off while it is lit.
        let level = if percent == 0 {
            0
        } else {
            scale(intensity, percent).max(intensity.min(1))
        };
        return [0, 0, 0, level];
    }
    let [r, g, b] = color.map(u32::from);
    if dims_by_intensity {
        return [
            quantise(r, red),
            quantise(g, green),
            quantise(b, blue),
            scale(intensity, percent),
        ];
    }
    let dim = |value: u32| ((value * u32::from(percent) + 50) / 100).min(255);
    [
        quantise(dim(r), red),
        quantise(dim(g), green),
        quantise(dim(b), blue),
        intensity,
    ]
}

/// A lamp id and the red, green, blue and intensity values it should show.
type Change = (usize, [u32; 4]);

/// A HID LampArray: its lamps as key lights, any beyond 255 as deck lights.
pub struct LampArray {
    model: &'static DeviceModel,
    transport: Box<dyn Transport>,
    array: Option<Arc<Array>>,
    /// Each lamp's colour, by lamp id.
    colors: Vec<Option<[u8; 3]>>,
    /// What each lamp was last sent, once Ryu holds the lamps.
    sent: Vec<Option<[u32; 4]>>,
    brightness: u8,
    /// Autonomous mode is off: the lamps are Ryu's.
    holding: bool,
    /// When the next finished update may go.
    next_update: Instant,
    /// Lamps changed since the last update that went out.
    dirty: bool,
}

impl LampArray {
    #[must_use]
    pub fn new(model: &'static DeviceModel, transport: Box<dyn Transport>) -> Self {
        let array = array(model);
        let lamps = array.as_ref().map_or(0, |array| array.lamps.len());
        Self {
            model,
            transport,
            array,
            colors: vec![None; lamps],
            sent: vec![None; lamps],
            brightness: 100,
            holding: false,
            next_update: Instant::now(),
            dirty: false,
        }
    }

    fn set_autonomous(&mut self, autonomous: bool) -> DriverResult<()> {
        let Some(control) = self
            .array
            .as_ref()
            .and_then(|array| array.layout.control.as_ref())
        else {
            return Ok(());
        };
        let mut buf = control.buffer();
        control.write(&mut buf, usage::AUTONOMOUS_MODE, 0, u64::from(autonomous));
        self.transport.send_feature_report(&buf)
    }

    /// Takes the lamps from the device's own effect. The first update waits one
    /// update interval, as the device needs.
    fn hold(&mut self) -> DriverResult<()> {
        if self.holding {
            return Ok(());
        }
        let Some(interval) = self.array.as_ref().map(|array| array.interval()) else {
            return Ok(());
        };
        self.set_autonomous(false)?;
        self.holding = true;
        self.sent.fill(None);
        self.next_update = Instant::now() + interval;
        tracing::debug!(
            device = self.model.id,
            "ryu-decks: LampArray autonomous mode off"
        );
        Ok(())
    }

    /// The reports that bring the lamps from what they were sent to what they
    /// should show: runs of one colour as range updates, the rest in
    /// multi-update reports, the last one marked complete.
    fn updates(&self, array: &Array) -> (Vec<Vec<u8>>, Vec<Change>) {
        let dims_by_intensity = array.dims_by_intensity();
        let changes: Vec<Change> = array
            .lamps
            .iter()
            .zip(&self.colors)
            .map(|(lamp, color)| encode(lamp, *color, self.brightness, dims_by_intensity))
            .enumerate()
            .filter(|(id, value)| self.sent.get(*id).copied().flatten() != Some(*value))
            .collect();
        let layout = &array.layout;
        let mut reports: Vec<(&Report, Vec<u8>)> = Vec::new();
        let mut singles: Vec<Change> = Vec::new();
        let mut at = 0;
        while at < changes.len() {
            let (start, value) = changes[at];
            let mut end = at + 1;
            while changes
                .get(end)
                .is_some_and(|(id, next)| *id == start + (end - at) && *next == value)
            {
                end += 1;
            }
            match &layout.range {
                Some(range) if end - at >= 2 || layout.multi.is_none() => {
                    let mut buf = range.buffer();
                    let last = start + (end - at) - 1;
                    range.write(&mut buf, usage::LAMP_ID_START, 0, start as u64);
                    range.write(&mut buf, usage::LAMP_ID_END, 0, last as u64);
                    for (channel, level) in CHANNELS.iter().zip(value) {
                        range.write(&mut buf, *channel, 0, u64::from(level));
                    }
                    reports.push((range, buf));
                    at = end;
                }
                _ => {
                    singles.push(changes[at]);
                    at += 1;
                }
            }
        }
        if let Some(multi) = &layout.multi {
            for chunk in singles.chunks(layout.slots) {
                let mut buf = multi.buffer();
                multi.write(&mut buf, usage::LAMP_COUNT, 0, chunk.len() as u64);
                for (slot, (id, value)) in chunk.iter().enumerate() {
                    multi.write(&mut buf, usage::LAMP_ID, slot, *id as u64);
                    for (channel, level) in CHANNELS.iter().zip(value) {
                        multi.write(&mut buf, *channel, slot, u64::from(*level));
                    }
                }
                reports.push((multi, buf));
            }
        }
        if let Some((report, buf)) = reports.last_mut() {
            report.write(buf, usage::UPDATE_FLAGS, 0, UPDATE_COMPLETE);
        }
        (reports.into_iter().map(|(_, buf)| buf).collect(), changes)
    }

    /// Sends what changed, once the device's update interval allows; until
    /// then the change waits for the next poll, and later ones join it.
    fn send_due(&mut self) -> DriverResult<()> {
        if !self.dirty {
            return Ok(());
        }
        let Some(array) = self.array.clone() else {
            self.dirty = false;
            return Ok(());
        };
        self.hold()?;
        let now = Instant::now();
        if now < self.next_update {
            return Ok(());
        }
        let (reports, changes) = self.updates(&array);
        for report in &reports {
            self.transport.send_feature_report(report)?;
        }
        for (id, value) in changes {
            if let Some(sent) = self.sent.get_mut(id) {
                *sent = Some(value);
            }
        }
        if !reports.is_empty() {
            self.next_update = now + array.interval();
        }
        self.dirty = false;
        Ok(())
    }
}

impl Driver for LampArray {
    /// A LampArray has no input; this only sends an update that was waiting
    /// for the device's update interval.
    fn poll(&mut self, timeout_ms: i32) -> DriverResult<Vec<Input>> {
        self.send_due()?;
        let wait = u64::try_from(timeout_ms).unwrap_or(0);
        if wait > 0 {
            std::thread::sleep(Duration::from_millis(wait));
        }
        Ok(Vec::new())
    }

    fn set_face(&mut self, key: u8, face: Option<&Face>) -> DriverResult<()> {
        if usize::from(key) >= usize::from(self.model.keys) {
            return Ok(());
        }
        if let Some(color) = self.colors.get_mut(usize::from(key)) {
            *color = face.map(|face| face.color);
        }
        Ok(())
    }

    fn set_deck_light(&mut self, color: Option<[u8; 3]>) -> DriverResult<()> {
        let keys = usize::from(self.model.keys);
        for lamp in self.colors.iter_mut().skip(keys) {
            *lamp = color;
        }
        Ok(())
    }

    fn flush(&mut self) -> DriverResult<()> {
        self.dirty = true;
        self.send_due()
    }

    fn set_brightness(&mut self, percent: u8) -> DriverResult<()> {
        self.brightness = percent.min(100);
        self.dirty = true;
        self.send_due()
    }

    fn heartbeat(&mut self) -> DriverResult<()> {
        self.send_due()
    }
}

impl Drop for LampArray {
    /// Gives the device its own lighting back.
    fn drop(&mut self) {
        if self.holding {
            let _ = self.set_autonomous(true);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::testing::Recorder;
    use super::*;

    /// One short item: `tag` with its size bits, then `data` little-endian.
    fn item(tag: u8, data: &[u8]) -> Vec<u8> {
        let size = match data.len() {
            0 => 0,
            1 => 1,
            2 => 2,
            _ => 3,
        };
        let mut bytes = vec![tag | size];
        bytes.extend_from_slice(data);
        bytes
    }

    fn usage(id: u8) -> Vec<u8> {
        item(0x08, &[id])
    }

    fn feature(constant: bool) -> Vec<u8> {
        item(0xb0, &[if constant { 0x03 } else { 0x02 }])
    }

    fn logical(id: u8, usages: &[u8], body: &[Vec<u8>]) -> Vec<u8> {
        let mut bytes = item(0x84, &[id]);
        bytes.extend(usage(usages[0]));
        bytes.extend(item(0xa0, &[0x02]));
        for part in body {
            bytes.extend_from_slice(part);
        }
        bytes.extend(item(0xc0, &[]));
        bytes
    }

    fn usages(ids: &[u8]) -> Vec<u8> {
        ids.iter().flat_map(|id| usage(*id)).collect()
    }

    /// A LampArray laid out like Microsoft's sample: reports 1 to 6, with
    /// report sizes and maxima carried over from one report to the next, the
    /// attributes declared constant, latency before purposes, eight
    /// multi-update slots.
    fn descriptor() -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend(item(0x04, &[0x59]));
        bytes.extend(usage(0x01));
        bytes.extend(item(0xa0, &[0x01]));
        bytes.extend(logical(
            1,
            &[0x02],
            &[
                usage(0x03),
                item(0x14, &[0x00]),
                item(0x24, &[0xff, 0xff, 0x00, 0x00]),
                item(0x94, &[1]),
                item(0x74, &[16]),
                feature(true),
                usages(&[0x04, 0x05, 0x06, 0x07, 0x08]),
                item(0x24, &[0xff, 0xff, 0xff, 0x7f]),
                item(0x94, &[5]),
                item(0x74, &[32]),
                feature(true),
            ],
        ));
        bytes.extend(logical(
            2,
            &[0x20],
            &[
                usage(0x21),
                item(0x24, &[0xff, 0xff, 0x00, 0x00]),
                item(0x94, &[1]),
                item(0x74, &[16]),
                feature(false),
            ],
        ));
        bytes.extend(logical(
            3,
            &[0x22],
            &[
                // Size 16 and count 1 carry over from report 2.
                usage(0x21),
                feature(false),
                usages(&[0x23, 0x24, 0x25, 0x27, 0x26]),
                item(0x24, &[0xff, 0xff, 0xff, 0x7f]),
                item(0x94, &[5]),
                item(0x74, &[32]),
                feature(false),
                usages(&[0x28, 0x29, 0x2a, 0x2b, 0x2c, 0x2d]),
                item(0x24, &[0xff, 0x00]),
                item(0x94, &[6]),
                item(0x74, &[8]),
                feature(false),
            ],
        ));
        let channels: Vec<u8> = (0..8).flat_map(|_| [0x51, 0x52, 0x53, 0x54]).collect();
        bytes.extend(logical(
            4,
            &[0x50],
            &[
                // Size 8 carries over from report 3.
                usage(0x03),
                item(0x24, &[8]),
                item(0x94, &[1]),
                feature(false),
                usage(0x55),
                item(0x24, &[1]),
                feature(false),
                // One usage for eight ids: it repeats.
                usage(0x21),
                item(0x24, &[0xff, 0xff, 0x00, 0x00]),
                item(0x94, &[8]),
                item(0x74, &[16]),
                feature(false),
                usages(&channels),
                item(0x24, &[0xff, 0x00]),
                item(0x94, &[32]),
                item(0x74, &[8]),
                feature(false),
            ],
        ));
        bytes.extend(logical(
            5,
            &[0x60],
            &[
                usage(0x55),
                item(0x24, &[1]),
                item(0x94, &[1]),
                feature(false),
                usages(&[0x61, 0x62]),
                item(0x24, &[0xff, 0xff, 0x00, 0x00]),
                item(0x94, &[2]),
                item(0x74, &[16]),
                feature(false),
                usages(&[0x51, 0x52, 0x53, 0x54]),
                item(0x24, &[0xff, 0x00]),
                item(0x94, &[4]),
                item(0x74, &[8]),
                feature(false),
            ],
        ));
        bytes.extend(logical(
            6,
            &[0x70],
            &[
                usage(0x71),
                item(0x24, &[1]),
                item(0x94, &[1]),
                feature(false),
            ],
        ));
        bytes.extend(item(0xc0, &[]));
        bytes
    }

    fn attributes_reply(count: u16, kind: u32, interval_us: u32) -> Vec<u8> {
        let mut reply = vec![0x01];
        reply.extend(count.to_le_bytes());
        for value in [70_000u32, 55_000, 1_000, kind, interval_us] {
            reply.extend(value.to_le_bytes());
        }
        reply
    }

    fn lamp_reply(id: u16, x_mm: u32, y_mm: u32, levels: [u8; 4], binding: u8) -> Vec<u8> {
        let mut reply = vec![0x03];
        reply.extend(id.to_le_bytes());
        // X, Y, Z, latency 4 ms, then purposes (Accent).
        for value in [x_mm * 1000, y_mm * 1000, 0, 4_000, 0x02] {
            reply.extend(value.to_le_bytes());
        }
        reply.extend(levels);
        reply.push(u8::from(levels[0] > 0));
        reply.push(binding);
        reply
    }

    #[test]
    fn the_descriptor_gives_the_sample_layout() {
        let layout = parse_descriptor(&descriptor()).unwrap();
        let lengths = [
            &layout.attributes,
            &layout.request,
            &layout.response,
            layout.multi.as_ref().unwrap(),
            layout.range.as_ref().unwrap(),
            layout.control.as_ref().unwrap(),
        ]
        .map(|report| (report.id, report.len()));
        assert_eq!(
            lengths,
            [(1, 23), (2, 3), (3, 29), (4, 51), (5, 10), (6, 2)]
        );
        assert_eq!(layout.slots, 8);
        let at = |report: &Report, usage, nth| {
            let field = report.field(usage, nth).unwrap();
            (field.offset / 8 + 1, field.size)
        };
        // Constant attribute fields are kept.
        assert_eq!(at(&layout.attributes, usage::LAMP_COUNT, 0), (1, 16));
        assert_eq!(
            at(&layout.attributes, usage::MIN_UPDATE_INTERVAL, 0),
            (19, 32)
        );
        assert_eq!(at(&layout.response, usage::LAMP_ID, 0), (1, 16));
        // Latency comes before purposes on the wire.
        assert_eq!(at(&layout.response, usage::PURPOSES, 0), (19, 32));
        assert_eq!(at(&layout.response, usage::INPUT_BINDING, 0), (28, 8));
        let multi = layout.multi.as_ref().unwrap();
        assert_eq!(at(multi, usage::LAMP_COUNT, 0), (1, 8));
        assert_eq!(at(multi, usage::LAMP_ID, 7), (17, 16));
        assert_eq!(at(multi, usage::RED, 1), (23, 8));
        assert_eq!(at(multi, usage::INTENSITY, 7), (50, 8));
        let range = layout.range.as_ref().unwrap();
        assert_eq!(at(range, usage::LAMP_ID_END, 0), (4, 16));
        assert_eq!(at(range, usage::INTENSITY, 0), (9, 8));
    }

    #[test]
    fn push_pop_four_byte_usages_and_ranges_are_followed() {
        // The sample's request and response reports, written with a pushed
        // and popped size, a page-carrying 4-byte usage and a usage range.
        let mut bytes = item(0x04, &[0x01]);
        bytes.extend(item(0x08, &[0x01, 0x00, 0x59, 0x00]));
        bytes.extend(item(0xa0, &[0x01]));
        bytes.extend(item(0x04, &[0x59]));
        bytes.extend(item(0x84, &[7]));
        bytes.extend(usages(&[0x03, 0x07]));
        bytes.extend(item(0x94, &[2]));
        bytes.extend(item(0x74, &[16]));
        bytes.extend(feature(true));
        bytes.extend(item(0xa4, &[]));
        bytes.extend(item(0x84, &[8]));
        bytes.extend(usage(0x21));
        bytes.extend(item(0x94, &[1]));
        bytes.extend(feature(false));
        bytes.extend(item(0x84, &[9]));
        bytes.extend(usages(&[0x21, 0x23]));
        bytes.extend(item(0x94, &[2]));
        bytes.extend(feature(false));
        bytes.extend(item(0xb4, &[]));
        // Popped back to report 7 and size 16: report 10 is a range update.
        bytes.extend(item(0x84, &[10]));
        bytes.extend(item(0x18, &[0x61]));
        bytes.extend(item(0x28, &[0x62]));
        bytes.extend(feature(false));
        bytes.extend(usages(&[0x51, 0x52, 0x53, 0x54]));
        bytes.extend(item(0x74, &[8]));
        bytes.extend(item(0x94, &[4]));
        bytes.extend(feature(false));
        bytes.extend(item(0xc0, &[]));
        let layout = parse_descriptor(&bytes).unwrap();
        assert_eq!(layout.attributes.id, 7);
        assert_eq!(layout.request.id, 8);
        assert_eq!(layout.response.id, 9);
        let range = layout.range.unwrap();
        assert_eq!((range.id, range.len()), (10, 9));
        assert_eq!(range.field(usage::LAMP_ID_END, 0).unwrap().offset, 16);
        assert!(layout.multi.is_none() && layout.control.is_none());
    }

    #[test]
    fn other_descriptors_and_garbage_are_refused() {
        assert!(parse_descriptor(&[]).is_none());
        // A boot keyboard.
        let keyboard = [
            0x05, 0x01, 0x09, 0x06, 0xa1, 0x01, 0x05, 0x07, 0x19, 0xe0, 0x29, 0xe7, 0x15, 0x00,
            0x25, 0x01, 0x75, 0x01, 0x95, 0x08, 0x81, 0x02, 0xc0,
        ];
        assert!(parse_descriptor(&keyboard).is_none());
        // Cut inside the attributes report: no LampArray to drive.
        assert!(parse_descriptor(&descriptor()[..20]).is_none());
        let full = descriptor();
        for cut in 0..full.len() {
            let _ = parse_descriptor(&full[..cut]);
        }
    }

    fn probed(count: u16, interval_us: u32, lamps: &[Vec<u8>]) -> (Recorder, Array) {
        let recorder = Recorder::default();
        recorder.queue_feature_reply(attributes_reply(count, 4, interval_us));
        for lamp in lamps {
            recorder.queue_feature_reply(lamp.clone());
        }
        let mut transport = recorder.clone();
        let array = probe(&mut transport, &descriptor()).unwrap();
        (recorder, array)
    }

    #[test]
    fn probing_asks_from_lamp_zero_and_checks_every_answer() {
        let recorder = Recorder::default();
        recorder.queue_feature_reply(attributes_reply(3, 1, 33_000));
        recorder.queue_feature_reply(lamp_reply(0, 3, 8, [255, 255, 255, 1], 0x29));
        // The device skipped ahead: lamp 1 is asked for by id.
        recorder.queue_feature_reply(lamp_reply(2, 21, 8, [255, 255, 255, 1], 0x04));
        recorder.queue_feature_reply(lamp_reply(1, 12, 8, [0, 0, 0, 4], 0x04));
        recorder.queue_feature_reply(lamp_reply(2, 21, 8, [255, 255, 255, 1], 0x04));
        let mut transport = recorder.clone();
        let array = probe(&mut transport, &descriptor()).unwrap();
        assert_eq!(recorder.features(), [vec![0x02, 0, 0], vec![0x02, 1, 0]]);
        assert_eq!(array.kind, 1);
        assert_eq!(array.min_update_interval_us, 33_000);
        assert_eq!(array.bounding_box, [70_000, 55_000, 1_000]);
        assert_eq!(array.lamps.len(), 3);
        assert_eq!(array.lamps[1].position, [12_000, 8_000, 0]);
        assert_eq!(array.lamps[1].levels, [0, 0, 0, 4]);
        assert!(!array.lamps[1].programmable);
        assert_eq!(array.lamps[0].purposes, 0x02);
        assert_eq!(array.lamps[0].binding, 0x29);
        assert_eq!(lamp_names(array.kind, &array.lamps), ["Esc", "A", "A (2)"]);
        assert_eq!(lamp_names(4, &array.lamps), ["Lamp 1", "Lamp 2", "Lamp 3"]);
    }

    #[test]
    fn a_device_that_never_names_the_lamp_or_has_none_is_refused() {
        let recorder = Recorder::default();
        recorder.queue_feature_reply(attributes_reply(0, 4, 0));
        let mut transport = recorder.clone();
        assert!(matches!(
            probe(&mut transport, &descriptor()),
            Err(DriverError::Refused(_))
        ));
        let recorder = Recorder::default();
        recorder.queue_feature_reply(attributes_reply(2, 4, 0));
        recorder.queue_feature_reply(lamp_reply(0, 0, 0, [255; 4], 0));
        recorder.queue_feature_reply(lamp_reply(0, 0, 0, [255; 4], 0));
        recorder.queue_feature_reply(lamp_reply(0, 0, 0, [255; 4], 0));
        let mut transport = recorder.clone();
        assert!(matches!(
            probe(&mut transport, &descriptor()),
            Err(DriverError::Refused(_))
        ));
        // Short answers read as zeros rather than panicking.
        let recorder = Recorder::default();
        recorder.queue_feature_reply(vec![0x01, 0x01]);
        recorder.queue_feature_reply(vec![0x03]);
        let mut transport = recorder.clone();
        let array = probe(&mut transport, &descriptor()).unwrap();
        assert_eq!(array.lamps[0].levels, [0; 4]);
    }

    /// The sample's 8 x 5 grid of RGB lamps at a 9 mm pitch.
    fn sample_grid(interval_us: u32) -> (Recorder, Array) {
        let lamps: Vec<Vec<u8>> = (0u16..40)
            .map(|id| {
                let x = 3 + 9 * u32::from(id % 8);
                let y = 8 + 9 * u32::from(id / 8);
                lamp_reply(id, x, y, [255, 255, 255, 1], 0)
            })
            .collect();
        probed(40, interval_us, &lamps)
    }

    #[test]
    fn the_sample_grid_is_forty_keys_in_eight_columns() {
        let (_, array) = sample_grid(33_000);
        let model = register(array.clone(), 0x2341, 0x8036, Some(" LampArray "));
        assert_eq!(model.name, "LampArray");
        assert_eq!((model.rows, model.cols, model.keys), (5, 8, 40));
        assert_eq!(model.deck_lights, 0);
        assert!(model.key_lights);
        assert!(model.id.starts_with("lamparray-2341-8036-"));
        assert_eq!(key_cells(model).unwrap(), (0..40).collect::<Vec<u16>>());
        assert_eq!(key_names(model).unwrap()[39], "Lamp 40");
        // The same device again is the same model.
        assert!(std::ptr::eq(
            register(array, 0x2341, 0x8036, Some("LampArray")),
            model
        ));
        assert!(models().iter().any(|known| std::ptr::eq(*known, model)));
    }

    #[test]
    fn staggered_rows_keep_their_shape_and_positionless_lamps_wrap() {
        // Two rows of 19 mm keys, the second shifted by a quarter key.
        let positions = [
            [9_500, 10_000],
            [28_500, 10_000],
            [47_500, 10_000],
            [14_250, 29_500],
            [33_250, 30_500],
        ];
        let grid = grid(&positions);
        assert_eq!((grid.rows, grid.cols), (2, 3));
        assert_eq!(grid.cells, [0, 1, 2, 3, 4]);
        let flat = grid_of(20, [0, 0]);
        assert_eq!((flat.rows, flat.cols), (2, 16));
        // Two lamps far apart do not make a huge grid.
        let sparse = super::grid(&[[0, 0], [1, 0], [5_000_000, 0]]);
        assert_eq!((sparse.rows, sparse.cols), (1, 3));
        assert_eq!(sparse.cells, [0, 1, 2]);
    }

    fn grid_of(count: usize, position: [u32; 2]) -> Grid {
        grid(&vec![position; count])
    }

    fn face(color: [u8; 3]) -> Face {
        Face {
            label: String::new(),
            color,
            image: None,
        }
    }

    fn driver_for(array: Array, product: &str) -> (Recorder, LampArray) {
        let model = register(array, 0x1234, 0x5678, Some(product));
        let recorder = Recorder::default();
        let driver = LampArray::new(model, Box::new(recorder.clone()));
        (recorder, driver)
    }

    #[test]
    fn colours_go_out_as_ranges_and_multi_updates_with_one_complete_flag() {
        let (_, array) = sample_grid(0);
        let (recorder, mut driver) = driver_for(array, "Grid A");
        for key in 0..40 {
            driver.set_face(key, Some(&face([0, 0, 0]))).unwrap();
        }
        driver.set_face(3, Some(&face([255, 0, 0]))).unwrap();
        driver.set_face(4, Some(&face([255, 0, 0]))).unwrap();
        driver.set_face(9, Some(&face([0x11, 0x22, 0x33]))).unwrap();
        driver.flush().unwrap();
        let features = recorder.features();
        // Autonomous mode off first.
        assert_eq!(features[0], [0x06, 0x00]);
        // Lamps 0-2 off, 3-4 red, 5-8 off, 10-39 off as ranges; lamp 9 alone.
        assert_eq!(features[1], [0x05, 0x00, 0, 0, 2, 0, 0, 0, 0, 0]);
        assert_eq!(features[2], [0x05, 0x00, 3, 0, 4, 0, 0xff, 0, 0, 1]);
        assert_eq!(features[3], [0x05, 0x00, 5, 0, 8, 0, 0, 0, 0, 0]);
        assert_eq!(features[4], [0x05, 0x00, 10, 0, 39, 0, 0, 0, 0, 0]);
        let mut multi = vec![0x04, 1, 0x01, 9, 0];
        multi.extend([0; 14]);
        multi.extend([0x11, 0x22, 0x33, 0x01]);
        multi.extend([0; 28]);
        assert_eq!(features[5], multi);
        assert_eq!(features.len(), 6);
        // Nothing changed: nothing is sent.
        driver.flush().unwrap();
        assert_eq!(recorder.features().len(), 6);
        // One lamp changes: one multi update, marked complete.
        driver.set_face(9, None).unwrap();
        driver.flush().unwrap();
        let last = recorder.features().pop().unwrap();
        assert_eq!(&last[..5], [0x04, 1, 0x01, 9, 0]);
        assert_eq!(&last[19..23], [0, 0, 0, 0]);
    }

    #[test]
    fn single_lamps_fill_multi_updates_eight_at_a_time() {
        let (_, array) = sample_grid(0);
        let (recorder, mut driver) = driver_for(array, "Grid B");
        for key in 0..40u8 {
            let shade = key * 6 + 1;
            driver.set_face(key, Some(&face([shade, 0, 0]))).unwrap();
        }
        driver.flush().unwrap();
        let features = recorder.features();
        assert_eq!(features.len(), 6);
        for (index, report) in features[1..].iter().enumerate() {
            assert_eq!(report.len(), 51);
            assert_eq!(report[..2], [0x04, 8]);
            assert_eq!(report[2], u8::from(index == 4));
            assert_eq!(report[3], u8::try_from(index * 8).unwrap());
        }
    }

    #[test]
    fn levels_brightness_and_fixed_colour_lamps_stay_in_range() {
        let rgb = Lamp {
            position: [0; 3],
            purposes: 0x02,
            levels: [255, 255, 255, 1],
            programmable: true,
            binding: 0,
        };
        assert_eq!(
            encode(&rgb, Some([200, 100, 0]), 50, false),
            [100, 50, 0, 1]
        );
        assert_eq!(encode(&rgb, Some([0, 0, 0]), 100, false), [0; 4]);
        assert_eq!(encode(&rgb, None, 100, false), [0; 4]);
        let coarse = Lamp {
            levels: [7, 3, 1, 10],
            ..rgb
        };
        assert_eq!(
            encode(&coarse, Some([255, 128, 100]), 100, true),
            [7, 2, 0, 10]
        );
        assert_eq!(
            encode(&coarse, Some([255, 255, 255]), 40, true),
            [7, 3, 1, 4]
        );
        let fixed = Lamp {
            levels: [0, 0, 0, 1],
            programmable: false,
            ..rgb
        };
        assert_eq!(encode(&fixed, Some([10, 0, 0]), 10, false), [0, 0, 0, 1]);
        assert_eq!(encode(&fixed, Some([10, 0, 0]), 0, false), [0; 4]);
        assert_eq!(encode(&fixed, None, 100, false), [0; 4]);
    }

    #[test]
    fn huge_device_reported_levels_and_positions_do_not_overflow() {
        let huge = Lamp {
            position: [0; 3],
            purposes: 0x02,
            levels: [u32::MAX; 4],
            programmable: true,
            binding: 0,
        };
        assert_eq!(
            encode(&huge, Some([255, 0, 255]), 100, true),
            [u32::MAX, 0, u32::MAX, u32::MAX]
        );
        assert_eq!(quantise(255, 0x0100_0000), 0x0100_0000);
        assert_eq!(scale(u32::MAX, 100), u32::MAX);
        let far = grid(&[
            [u32::MAX - 20_000, 0],
            [u32::MAX, 0],
            [u32::MAX - 10_000, 0],
        ]);
        assert_eq!(far.cells.len(), 3);
    }

    #[test]
    fn updates_wait_for_the_interval_and_the_device_gets_its_effect_back() {
        let (_, array) = sample_grid(20_000);
        let (recorder, mut driver) = driver_for(array, "Grid C");
        driver.set_face(0, Some(&face([1, 2, 3]))).unwrap();
        driver.flush().unwrap();
        // Only autonomous mode off: the first update waits one interval.
        assert_eq!(recorder.features(), [vec![0x06, 0x00]]);
        assert!(driver.poll(0).unwrap().is_empty());
        assert_eq!(recorder.features().len(), 1);
        std::thread::sleep(Duration::from_millis(25));
        driver.poll(0).unwrap();
        assert_eq!(recorder.features().len(), 3);
        drop(driver);
        assert_eq!(recorder.features().pop().unwrap(), [0x06, 0x01]);
    }

    #[test]
    fn lamps_past_255_are_deck_lights() {
        let lamps: Vec<Vec<u8>> = (0u16..300)
            .map(|id| {
                let mut reply = lamp_reply(0, 0, 0, [255, 255, 255, 1], 0);
                reply[1..3].copy_from_slice(&id.to_le_bytes());
                reply[3..7].copy_from_slice(&(u32::from(id) * 1000).to_le_bytes());
                reply
            })
            .collect();
        let (_, array) = probed(300, 0, &lamps);
        let (recorder, mut driver) = driver_for(array, "Strip");
        let model = driver.model;
        assert_eq!((model.keys, model.deck_lights), (255, 45));
        assert_eq!((model.rows, model.cols), (1, 255));
        driver.set_deck_light(Some([0, 0, 255])).unwrap();
        driver.set_face(254, Some(&face([0, 255, 0]))).unwrap();
        driver.flush().unwrap();
        let features = recorder.features();
        assert_eq!(features[1], [0x05, 0, 0, 0, 253, 0, 0, 0, 0, 0]);
        assert_eq!(features[2], [0x05, 0x00, 255, 0, 43, 1, 0, 0, 255, 1]);
        // The complete flag is on the last report only.
        assert_eq!(&features[3][..5], [0x04, 1, 0x01, 254, 0]);
        assert_eq!(features.len(), 4);
    }
}
