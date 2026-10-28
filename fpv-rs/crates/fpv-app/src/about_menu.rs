//! Route the native macOS About command to the same Rust view as Settings.
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[derive(Clone, Default)]
struct AboutRequest(Arc<AtomicBool>);

impl AboutRequest {
    fn open(&self) {
        self.0.store(true, Ordering::Release);
    }
    fn take(&self) -> bool {
        self.0.swap(false, Ordering::AcqRel)
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use super::AboutRequest;
    use eframe::egui;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, Sel};
    use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send, sel};
    use objc2_app_kit::{NSApplication, NSMenu, NSMenuItem};
    use objc2_foundation::{MainThreadMarker, NSObject, NSObjectProtocol};

    struct Ivars {
        request: AboutRequest,
        context: egui::Context,
    }

    define_class!(
        // SAFETY: NSObject has no subclassing requirements; this target lives
        // only on the AppKit main thread and retains its Rust callback state.
        #[unsafe(super = NSObject)]
        #[name = "FPVRushAboutTarget"]
        #[thread_kind = MainThreadOnly]
        #[ivars = Ivars]
        struct AboutTarget;

        // SAFETY: NSObjectProtocol imposes no additional requirements.
        unsafe impl NSObjectProtocol for AboutTarget {}

        impl AboutTarget {
            // SAFETY: A Cocoa target/action takes a sender object and returns void.
            #[unsafe(method(showFPVRushAbout:))]
            fn show_about(&self, _sender: Option<&AnyObject>) {
                self.ivars().request.open();
                self.ivars().context.request_repaint();
            }
        }
    );

    pub(crate) struct AboutMenu {
        // NSMenuItem.target is weak. Keep its receiver alive until restoration.
        _target: Retained<AboutTarget>,
        item: Retained<NSMenuItem>,
        menu: Retained<NSMenu>,
        index: isize,
        previous_target: Option<Retained<AnyObject>>,
        previous_action: Option<Sel>,
        request: AboutRequest,
        smoke_checked: bool,
    }

    fn find_about(
        menu: &Retained<NSMenu>,
    ) -> Option<(Retained<NSMenu>, isize, Retained<NSMenuItem>)> {
        for index in 0..menu.numberOfItems() {
            let Some(item) = menu.itemAtIndex(index) else {
                continue;
            };
            // Match the selector, not the localized menu title or its position.
            if item.action() == Some(sel!(orderFrontStandardAboutPanel:)) {
                return Some((menu.clone(), index, item));
            }
            if let Some(submenu) = item.submenu()
                && let Some(found) = find_about(&submenu)
            {
                return Some(found);
            }
        }
        None
    }

    impl AboutMenu {
        pub fn install(context: &egui::Context) -> Option<Self> {
            let mtm = MainThreadMarker::new()?;
            let app = NSApplication::sharedApplication(mtm);
            let (menu, index, item) = find_about(&app.mainMenu()?)?;
            let request = AboutRequest::default();
            let target = AboutTarget::alloc(mtm).set_ivars(Ivars {
                request: request.clone(),
                context: context.clone(),
            });
            // SAFETY: NSObject.init takes no arguments and returns the initialized receiver.
            let target: Retained<AboutTarget> = unsafe { msg_send![super(target), init] };
            let previous_target = item.target();
            let previous_action = item.action();
            // SAFETY: The retained target implements this exact action signature.
            unsafe {
                item.setTarget(Some(&target));
                item.setAction(Some(sel!(showFPVRushAbout:)));
            }
            Some(Self {
                _target: target,
                item,
                menu,
                index,
                previous_target,
                previous_action,
                request,
                smoke_checked: false,
            })
        }

        pub fn take_request(&self) -> bool {
            self.request.take()
        }

        pub fn smoke_test_if_requested(&mut self) {
            if self.smoke_checked {
                return;
            }
            self.smoke_checked = true;
            if std::env::var_os("FPV_SMOKE_ABOUT_MENU").is_none() {
                return;
            }
            // Exercise actual Cocoa menu dispatch repeatedly, not the callback
            // directly. Leave the last request for App's normal About renderer.
            for _ in 0..2 {
                self.menu.performActionForItemAtIndex(self.index);
                assert!(self.request.take(), "macOS About menu failed to dispatch");
                assert!(!self.request.take(), "About request was not consumed");
            }
            self.menu.performActionForItemAtIndex(self.index);
            eprintln!("FPV_MACOS_ABOUT_MENU_SMOKE_OK");
        }
    }

    impl Drop for AboutMenu {
        fn drop(&mut self) {
            // SAFETY: Restore the original matching receiver/action before our
            // retained receiver is dropped, so the menu never has a dangling target.
            unsafe {
                self.item.setAction(self.previous_action);
                self.item.setTarget(self.previous_target.as_deref());
            }
        }
    }
}

#[cfg(target_os = "macos")]
pub(crate) use macos::AboutMenu;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn about_requests_are_consumed_and_can_be_opened_again() {
        let request = AboutRequest::default();
        let native_target = request.clone();
        assert!(!request.take());
        for _ in 0..3 {
            native_target.open();
            assert!(request.take());
            assert!(!request.take());
        }
    }
}
