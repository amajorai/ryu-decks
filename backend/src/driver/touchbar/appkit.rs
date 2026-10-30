//! The AppKit side of the Touch Bar, on the main thread.
//!
//! Apps can only fill the Touch Bar while they are frontmost, so Ryu uses the
//! same private DFRFoundation calls as the open-source Touch Bar customisers:
//! a button in the Control Strip (`addSystemTrayItem:`, then
//! `DFRElementSetControlStripPresenceForIdentifier`) that opens Ryu's bar over
//! the whole Touch Bar (`presentSystemModalTouchBar:systemTrayItemIdentifier:`),
//! with the close box that returns to the normal Touch Bar. Every private call is
//! looked up at run time and skipped when the system does not have it, so a
//! macOS release that drops one degrades to "no Ryu button" rather than a crash.

use std::cell::RefCell;
use std::ffi::{c_void, CStr};
use std::sync::mpsc::{Receiver, Sender, TryRecvError};

use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Bool, NSObject, NSObjectProtocol, Sel};
use objc2::{
    define_class, msg_send, sel, AllocAnyThread, ClassType, DefinedClass, MainThreadMarker,
    MainThreadOnly,
};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSButton, NSCellImagePosition, NSColor,
    NSCustomTouchBarItem, NSImage, NSImageScaling, NSTouchBar, NSTouchBarItem,
};
use objc2_foundation::{NSArray, NSData, NSSet, NSSize, NSString, NSTimer};

use super::{Button, Command, MainEnd};

/// The Control Strip button's identifier.
const TRAY_ID: &str = "com.ryuhq.decks.touch-bar";
const KEY_ID_PREFIX: &str = "com.ryuhq.decks.touch-bar.key.";
const DFR_FOUNDATION: &CStr =
    c"/System/Library/PrivateFrameworks/DFRFoundation.framework/DFRFoundation";
/// How often the main thread picks up new layouts from the device loop.
const POLL_SECONDS: f64 = 0.05;
/// A key's size in points (its face is drawn at 2x).
const KEY_SIZE: NSSize = NSSize {
    width: 112.0,
    height: 30.0,
};

type SetPresence = unsafe extern "C" fn(*const NSString, Bool);
type ShowCloseBox = unsafe extern "C" fn(Bool);

/// The DFRFoundation functions, if this macOS has them.
struct Dfr {
    set_presence: Option<SetPresence>,
    show_close_box: Option<ShowCloseBox>,
}

impl Dfr {
    fn load() -> Self {
        // SAFETY: dlopen/dlsym with NUL-terminated names; a missing library or
        // symbol yields null, which maps to `None`. The signatures match the
        // functions' declarations in the open-source Touch Bar tools.
        unsafe {
            let handle = libc::dlopen(DFR_FOUNDATION.as_ptr(), libc::RTLD_LAZY);
            if handle.is_null() {
                return Self {
                    set_presence: None,
                    show_close_box: None,
                };
            }
            let symbol = |name: &CStr| {
                let found = libc::dlsym(handle, name.as_ptr());
                (!found.is_null()).then_some(found)
            };
            Self {
                set_presence: symbol(c"DFRElementSetControlStripPresenceForIdentifier")
                    .map(|found| std::mem::transmute::<*mut c_void, SetPresence>(found)),
                show_close_box: symbol(c"DFRSystemModalShowsCloseBoxWhenFrontMost")
                    .map(|found| std::mem::transmute::<*mut c_void, ShowCloseBox>(found)),
            }
        }
    }
}

struct State {
    bar: Retained<NSTouchBar>,
    tray: Retained<NSCustomTouchBarItem>,
    tray_installed: bool,
}

struct Ivars {
    commands: Receiver<Command>,
    taps: Sender<u8>,
    dfr: Dfr,
    state: RefCell<Option<State>>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and `Controller` does
    // not implement `Drop`.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "RyuDecksTouchBarController"]
    #[ivars = Ivars]
    struct Controller;

    unsafe impl NSObjectProtocol for Controller {}

    impl Controller {
        #[unsafe(method(keyTapped:))]
        fn key_tapped(&self, sender: &NSButton) {
            if let Ok(key) = u8::try_from(sender.tag()) {
                // The device loop is gone only while the process exits.
                let _ = self.ivars().taps.send(key);
            }
        }

        #[unsafe(method(openBar:))]
        fn open_bar(&self, _sender: &NSButton) {
            self.present();
        }

        #[unsafe(method(tick:))]
        fn tick(&self, _timer: &NSTimer) {
            self.drain();
        }
    }
);

/// Whether `class` answers `selector` (private API may be missing).
fn responds(class: &AnyClass, selector: Sel) -> bool {
    // SAFETY: `respondsToSelector:` is defined on every NSObject class.
    unsafe { msg_send![class, respondsToSelector: selector] }
}

fn color(rgb: [u8; 3]) -> Retained<NSColor> {
    let [red, green, blue] = rgb.map(|channel| f64::from(channel) / 255.0);
    NSColor::colorWithSRGBRed_green_blue_alpha(red, green, blue, 1.0)
}

impl Controller {
    fn new(mtm: MainThreadMarker, end: MainEnd) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Ivars {
            commands: end.commands,
            taps: end.taps,
            dfr: Dfr::load(),
            state: RefCell::new(None),
        });
        // SAFETY: NSObject's designated initialiser.
        let this: Retained<Self> = unsafe { msg_send![super(this), init] };

        let tray = NSCustomTouchBarItem::initWithIdentifier(
            NSCustomTouchBarItem::alloc(mtm),
            &NSString::from_str(TRAY_ID),
        );
        let target: &AnyObject = &this;
        // SAFETY: `this` answers `openBar:` and outlives the button (it lives
        // for the rest of the process).
        let opener = unsafe {
            NSButton::buttonWithTitle_target_action(
                &NSString::from_str("Ryu"),
                Some(target),
                Some(sel!(openBar:)),
                mtm,
            )
        };
        tray.setView(&opener);
        *this.ivars().state.borrow_mut() = Some(State {
            bar: NSTouchBar::new(mtm),
            tray,
            tray_installed: false,
        });
        this
    }

    fn drain(&self) {
        loop {
            match self.ivars().commands.try_recv() {
                Ok(Command::Show(buttons)) => self.show(&buttons),
                Ok(Command::Hide) => self.hide(),
                Err(TryRecvError::Empty) => return,
                // The device loop ended: the process is exiting.
                Err(TryRecvError::Disconnected) => {
                    self.hide();
                    return;
                }
            }
        }
    }

    fn show(&self, buttons: &[Button]) {
        let mtm = self.mtm();
        let target: &AnyObject = self;
        let mut items: Vec<Retained<NSTouchBarItem>> = Vec::with_capacity(buttons.len());
        let mut identifiers = Vec::with_capacity(buttons.len());
        for button in buttons {
            let identifier = NSString::from_str(&format!("{KEY_ID_PREFIX}{}", button.key));
            let item = NSCustomTouchBarItem::initWithIdentifier(
                NSCustomTouchBarItem::alloc(mtm),
                &identifier,
            );
            // SAFETY: `self` answers `keyTapped:` and lives for the process.
            let view = unsafe {
                NSButton::buttonWithTitle_target_action(
                    &NSString::from_str(&button.label),
                    Some(target),
                    Some(sel!(keyTapped:)),
                    mtm,
                )
            };
            view.setTag(isize::from(button.key));
            view.setBezelColor(Some(&color(button.color)));
            let image = button
                .png
                .as_deref()
                .and_then(|png| NSImage::initWithData(NSImage::alloc(), &NSData::with_bytes(png)));
            if let Some(image) = image {
                image.setSize(KEY_SIZE);
                view.setImage(Some(&image));
                view.setImageScaling(NSImageScaling::ScaleProportionallyUpOrDown);
                // The label stays the button's title, for VoiceOver.
                view.setImagePosition(NSCellImagePosition::ImageOnly);
            }
            item.setView(&view);
            items.push(Retained::into_super(item));
            identifiers.push(identifier);
        }

        let mut state = self.ivars().state.borrow_mut();
        let Some(state) = state.as_mut() else {
            return;
        };
        state
            .bar
            .setTemplateItems(&NSSet::from_retained_slice(&items));
        state
            .bar
            .setDefaultItemIdentifiers(&NSArray::from_retained_slice(&identifiers));
        if !state.tray_installed {
            state.tray_installed = self.set_tray(&state.tray, true);
        }
    }

    fn hide(&self) {
        let mut state = self.ivars().state.borrow_mut();
        let Some(state) = state.as_mut() else {
            return;
        };
        if !state.tray_installed {
            return;
        }
        let class = NSTouchBar::class();
        // SAFETY: private class methods, called only when present, with the bar
        // they expect.
        unsafe {
            if responds(class, sel!(dismissSystemModalTouchBar:)) {
                let _: () = msg_send![class, dismissSystemModalTouchBar: &*state.bar];
            } else if responds(class, sel!(dismissSystemModalFunctionBar:)) {
                let _: () = msg_send![class, dismissSystemModalFunctionBar: &*state.bar];
            }
        }
        self.set_tray(&state.tray, false);
        state.tray_installed = false;
    }

    /// Add the Control Strip button, or take it away. Returns whether it is there.
    fn set_tray(&self, tray: &NSCustomTouchBarItem, present: bool) -> bool {
        let class = NSTouchBarItem::class();
        let known = if present {
            responds(class, sel!(addSystemTrayItem:))
        } else {
            responds(class, sel!(removeSystemTrayItem:))
        };
        let Some(set_presence) = self.ivars().dfr.set_presence.filter(|_| known) else {
            if present {
                tracing::warn!(
                    "ryu-decks: this macOS has no Control Strip API; the Touch Bar stays as it is"
                );
            }
            return false;
        };
        let identifier = NSString::from_str(TRAY_ID);
        // SAFETY: both calls are present (checked above) and take a touch bar
        // item and an identifier string respectively.
        unsafe {
            if present {
                let _: () = msg_send![class, addSystemTrayItem: tray];
            } else {
                let _: () = msg_send![class, removeSystemTrayItem: tray];
            }
            set_presence(&*identifier, Bool::new(present));
        }
        present
    }

    /// Open Ryu's keys over the Touch Bar, with the close box that returns to
    /// the normal Touch Bar.
    fn present(&self) {
        let state = self.ivars().state.borrow();
        let Some(state) = state.as_ref() else {
            return;
        };
        if let Some(show_close_box) = self.ivars().dfr.show_close_box {
            // SAFETY: present in this DFRFoundation; takes a BOOL.
            unsafe { show_close_box(Bool::YES) };
        }
        let class = NSTouchBar::class();
        let identifier = NSString::from_str(TRAY_ID);
        // SAFETY: private class methods, called only when present, with a bar
        // and the identifier of the Control Strip item that opened it.
        unsafe {
            if responds(
                class,
                sel!(presentSystemModalTouchBar:systemTrayItemIdentifier:),
            ) {
                let _: () = msg_send![
                    class,
                    presentSystemModalTouchBar: &*state.bar,
                    systemTrayItemIdentifier: &*identifier
                ];
            } else if responds(
                class,
                sel!(presentSystemModalFunctionBar:systemTrayItemIdentifier:),
            ) {
                let _: () = msg_send![
                    class,
                    presentSystemModalFunctionBar: &*state.bar,
                    systemTrayItemIdentifier: &*identifier
                ];
            }
        }
    }
}

/// Wait for the first bound key, then run AppKit on this (the main) thread for
/// the rest of the process.
pub fn run(end: MainEnd) {
    let first = loop {
        match end.commands.recv() {
            Ok(Command::Show(buttons)) => break buttons,
            Ok(Command::Hide) => {}
            Err(_) => return,
        }
    };
    let Some(mtm) = MainThreadMarker::new() else {
        tracing::warn!("ryu-decks: the Touch Bar needs the main thread; leaving it alone");
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    // A background helper: no Dock icon, no menu bar.
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    let controller = Controller::new(mtm, end);
    controller.show(&first);
    let target: &AnyObject = &controller;
    // SAFETY: `controller` answers `tick:` and is kept alive below for as long as
    // the run loop (and so the repeating timer) runs.
    unsafe {
        NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(
            POLL_SECONDS,
            target,
            sel!(tick:),
            None,
            true,
        );
    }
    tracing::info!("ryu-decks: Touch Bar ready; open it from the Control Strip");
    app.run();
    drop(controller);
}
