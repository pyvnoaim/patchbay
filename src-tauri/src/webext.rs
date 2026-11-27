//! Bitwarden's own browser extension, loaded over the web tabs.
//!
//! macOS 15.4 added `WKWebExtension`, which runs a Chrome-format extension inside a
//! `WKWebView` an app already owns. That is the only way to autofill a device's web UI
//! without patchbay ever holding a credential: the extension's own popup does the
//! signing in, and the master password never reaches us.
//!
//! The cost is that patchbay has to answer as a *browser*. An extension asks for tabs,
//! windows, permissions and message ports, and each of those is a protocol answered
//! in `shim` below over the web tabs in `commands/web.rs`. Only what filling a login
//! needs is answered; the rest takes WebKit's defaults, which deny.
//!
//! Off unless ticked. Loaded from the installed Bitwarden for Mac, never bundled: the
//! Chrome build asks for offscreen documents and a side panel WebKit doesn't have, and
//! its worker waits on them forever. patchbay answers as a browser in `shim` - a tab
//! per web tab, one window, a delegate that draws the popup - and everything else takes
//! WebKit's default, which denies. Host access is the device urls in the list plus
//! Bitwarden's own servers (and a self-hosted one from `[settings].bitwarden_server`,
//! https only), never all hosts; without the servers its fetches fail CORS and sign-in
//! dies. Its web views say `Safari/` in the user agent because that is how Bitwarden
//! works out where it is, and a bare WKWebView leaves it off. The context's
//! `uniqueIdentifier` and `baseURL` are pinned to `BITWARDEN_ID`: left at WebKit's
//! random default the extension's storage is memory-only and every launch is signed
//! out, and changing the value later signs everyone out. WebKit state is
//! main-thread-only and lives in a thread-local; `RUNNING` is the atomic that command
//! threads read, because the thread-local is empty there.
//!
//! One controller for the life of the app, and only the context comes and goes. A web
//! tab's configuration names its controller for good, so a second controller (the
//! toggle off and on, a new server) left every open tab asking one that had gone. A
//! page loaded before a context did still has to load again: content scripts go in
//! with a page, not after it.
//!
//! `load` is also the refresh. Called again while running it grants the list's hosts
//! anew, so a device added since launch can be filled, and it loads Bitwarden again if
//! the app updated underneath: the background page would be the old build in memory
//! and the popup the new one from disk. `stale` is how that is noticed.

use serde::Serialize;

/// What a package turned out to be, for the Settings panel to show. Off macOS nothing
/// builds one - every command there refuses first - and CI's clippy runs on Linux.
#[derive(Serialize, Default, Debug, Clone)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub struct Package {
    pub name: String,
    pub version: String,
    /// Parse-time complaints. Present even when the extension loaded, because
    /// `WKWebExtension` reports what it ignored as well as what stopped it.
    pub errors: Vec<String>,
    /// Running now, not merely readable. "Enabled" and "loaded" are different things
    /// and the panel has to be able to tell them apart.
    #[serde(default)]
    pub loaded: bool,
    /// How many of your devices it may see. Its entire reach, so it is worth showing.
    #[serde(default)]
    pub hosts: usize,
}

/// Whether this machine can host an extension at all. Below macOS 15.4 the class
/// isn't there, and a `msg_send` to a class that doesn't exist is a crash rather
/// than an error, so every entry point checks this first.
#[cfg(target_os = "macos")]
pub fn supported() -> bool {
    objc2::runtime::AnyClass::get(c"WKWebExtension").is_some()
}

/// Bitwarden's Safari build, out of the installed Bitwarden for Mac. It is the one
/// written for WebKit's extension runtime: the Chrome build asks for offscreen
/// documents and a side panel WebKit doesn't have, and its background worker waits on
/// them forever. Loaded from the app rather than shipped, so it is always the version
/// already running there, and patchbay bundles nobody else's code.
// ponytail: the one install path; Spotlight's bundle-id lookup if it ever moves.
pub fn package() -> Result<std::path::PathBuf, String> {
    let p = std::path::Path::new(
        "/Applications/Bitwarden.app/Contents/PlugIns/safari.appex/Contents/Resources",
    );
    match p.join("manifest.json").is_file() {
        true => Ok(p.to_path_buf()),
        false => Err(
            "Bitwarden for Mac isn't installed - get it from bitwarden.com or the App Store".into(),
        ),
    }
}

/// Read a package without loading it: a directory holding a `manifest.json`, or a zip
/// of one. Nothing is run and no context is created.
#[cfg(target_os = "macos")]
pub async fn inspect(app: &tauri::AppHandle, path: std::path::PathBuf) -> Result<Package, String> {
    use block2::RcBlock;
    use objc2::MainThreadMarker;
    use objc2_foundation::{NSError, NSString, NSURL};
    use objc2_web_kit::WKWebExtension;

    if !supported() {
        return Err("this needs macOS 15.4 or newer".into());
    }
    if !path.exists() {
        return Err(format!("{}: no such file", path.display()));
    }

    // Only the main thread may touch WebKit, and a command runs on the runtime's. The
    // answer comes back down a channel rather than the main thread waiting for it:
    // reading a manifest is disk work, and a main thread spinning is a frozen window.
    let (tx, rx) = std::sync::mpsc::channel::<Result<Package, String>>();
    let name = path.display().to_string();
    app.run_on_main_thread(move || {
        let Some(mtm) = MainThreadMarker::new() else {
            let _ = tx.send(Err("not on the main thread".into()));
            return;
        };
        let url = NSURL::fileURLWithPath(&NSString::from_str(&name));
        let done = RcBlock::new(move |ext: *mut WKWebExtension, err: *mut NSError| {
            let _ = tx.send(unsafe { read_package(ext, err) });
        });
        unsafe { WKWebExtension::extensionWithResourceBaseURL_completionHandler(&url, &done, mtm) };
    })
    .map_err(|e| format!("could not reach the main thread: {e}"))?;

    // Bounded: a package WebKit never answers about must not hang the sheet open.
    tauri::async_runtime::spawn_blocking(move || {
        rx.recv_timeout(std::time::Duration::from_secs(30))
            .unwrap_or_else(|_| Err("reading the extension took too long".into()))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// The completion handler's two arguments, as one answer. Separate so the block above
/// stays the plumbing and this stays the reading.
#[cfg(target_os = "macos")]
unsafe fn read_package(
    ext: *mut objc2_web_kit::WKWebExtension,
    err: *mut objc2_foundation::NSError,
) -> Result<Package, String> {
    if let Some(e) = err.as_ref() {
        return Err(e.localizedDescription().to_string());
    }
    let Some(ext) = ext.as_ref() else {
        return Err("the extension could not be read, and said no more than that".into());
    };
    Ok(Package {
        name: ext
            .displayName()
            .map(|s| s.to_string())
            .unwrap_or_else(|| "unnamed".into()),
        version: ext
            .displayVersion()
            .map(|s| s.to_string())
            .unwrap_or_default(),
        errors: ext
            .errors()
            .iter()
            .map(|e| e.localizedDescription().to_string())
            .collect(),
        // Reading a package never runs it; `load` fills these in.
        loaded: false,
        hosts: 0,
    })
}

/// What answers as the browser, for the life of the app, and the extension loaded
/// into it while the toggle is on. One controller, because the tabs it serves are one
/// window's, and a tab keeps the controller it was built with.
///
/// `Retained` is not `Send`, and every WebKit call here is main-thread-only anyway, so
/// the handles live in a thread-local rather than Tauri's state.
#[cfg(target_os = "macos")]
mod live {
    use super::shim::{Delegate, Tab, Window};
    use objc2::rc::Retained;
    use objc2_app_kit::NSView;
    use objc2_foundation::{NSRect, NSRectEdge};
    use objc2_web_kit::{WKWebExtensionContext, WKWebExtensionController};
    use std::cell::RefCell;
    use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
    use std::sync::Mutex;
    use std::time::SystemTime;

    pub struct Held {
        pub controller: Retained<WKWebExtensionController>,
        /// `None` while the toggle is off. The tabs stay, so turning it on finds them.
        pub context: Option<Retained<WKWebExtensionContext>>,
        /// What loaded, kept so a second `load` reports what is running instead of
        /// what is on disk, which after an update is a different version.
        pub facts: super::Package,
        /// The hosts and server last granted. The window asks after every reload of
        /// the list, and the same answer twice is not worth telling WebKit about.
        pub granted: super::Reach,
        /// Held here because AppKit delegates are weak: the controller alone would let
        /// it go, and the popup would quietly never show.
        pub _delegate: Retained<Delegate>,
        pub window: Retained<Window>,
        pub tabs: Vec<Retained<Tab>>,
        pub active: Option<u32>,
        /// Where the popup goes: the view the key button is drawn in, the button's
        /// rect in that view's coordinates, and the edge to hang from.
        pub anchor: Option<(Retained<NSView>, NSRect, NSRectEdge)>,
    }

    thread_local! {
        static HELD: RefCell<Option<Held>> = const { RefCell::new(None) };
    }
    // Read from command threads, where the thread-local above is always empty: asking
    // it there said "not running" to every web tab, so none was ever attached.
    static ATTACHED: AtomicBool = AtomicBool::new(false);
    static RUNNING: AtomicBool = AtomicBool::new(false);
    /// The loaded manifest's mtime, which is how a Bitwarden updated since is told.
    static STAMP: Mutex<Option<SystemTime>> = Mutex::new(None);

    pub fn set(h: Held) {
        HELD.with(|x| *x.borrow_mut() = Some(h));
        ATTACHED.store(true, Relaxed);
    }

    pub fn up(
        context: Retained<WKWebExtensionContext>,
        facts: super::Package,
        granted: super::Reach,
        at: Option<SystemTime>,
    ) {
        *STAMP.lock().unwrap_or_else(|e| e.into_inner()) = at;
        read(|h| {
            h.context = Some(context);
            h.facts = facts;
            h.granted = granted;
        });
        RUNNING.store(true, Relaxed);
    }

    pub fn down() -> Option<(
        Retained<WKWebExtensionController>,
        Retained<WKWebExtensionContext>,
    )> {
        RUNNING.store(false, Relaxed);
        read(|h| Some((h.controller.clone(), h.context.take()?))).flatten()
    }

    pub fn attached() -> bool {
        ATTACHED.load(Relaxed)
    }

    pub fn running() -> bool {
        RUNNING.load(Relaxed)
    }

    pub fn stamp() -> Option<SystemTime> {
        *STAMP.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A short borrow, cloning out what the caller needs. WebKit calls back into the
    /// tab, window and delegate synchronously and each reads this again, so calling
    /// WebKit while the borrow is held is a `RefCell` panic.
    pub fn read<R>(f: impl FnOnce(&mut Held) -> R) -> Option<R> {
        HELD.with(|x| x.borrow_mut().as_mut().map(f))
    }
}

/// What the extension may see: the devices' match patterns, and a server of your own.
#[cfg(target_os = "macos")]
type Reach = (Vec<String>, Option<String>);

/// Whether the extension is up, which is when a web tab gets its key.
#[cfg(target_os = "macos")]
pub fn running() -> bool {
    live::running()
}

/// Whether there is a controller to tell about tabs. True from the first load on, the
/// toggle off included: a tab opened meanwhile is then one Bitwarden finds when it
/// comes back, rather than one to close and open again.
#[cfg(target_os = "macos")]
pub fn attached() -> bool {
    live::attached()
}

/// The installed manifest's mtime. An update replaces the bundle, so it moves.
#[cfg(target_os = "macos")]
fn stamp() -> Option<std::time::SystemTime> {
    std::fs::metadata(package().ok()?.join("manifest.json"))
        .ok()?
        .modified()
        .ok()
}

/// Whether Bitwarden for Mac was updated under the one running here. A bundle that is
/// missing is one mid-update, not a newer one: it is asked again on the next call.
#[cfg(target_os = "macos")]
pub fn stale() -> bool {
    running() && stamp().is_some_and(|now| live::stamp() != Some(now))
}

/// A webview configuration carrying the controller, or `None` before there is one.
/// Main thread only, and it has to be handed to the *builder*: WebKit reads the
/// controller when the view is created, so setting it afterwards is too late.
#[cfg(target_os = "macos")]
pub fn tab_configuration(
    mtm: objc2::MainThreadMarker,
) -> Option<objc2::rc::Retained<objc2_web_kit::WKWebViewConfiguration>> {
    let controller = live::read(|h| h.controller.clone())?;
    let conf = unsafe { objc2_web_kit::WKWebViewConfiguration::new(mtm) };
    unsafe {
        conf.setWebExtensionController(Some(&controller));
        // The content scripts run in the page's own web view, and ask the same question.
        conf.setApplicationNameForUserAgent(Some(&objc2_foundation::NSString::from_str(SAFARI_UA)));
    }
    Some(conf)
}

/// Bitwarden's identity here, fixed for good: its storage (the account, the encrypted
/// vault) is filed under it, so a new value is a signed-out Bitwarden for everyone.
#[cfg(target_os = "macos")]
const BITWARDEN_ID: &str = "5f6fd93f-7c8d-4e18-beb4-7c1cbda7f36c";

/// Safari's user-agent suffix. Bitwarden decides which browser it is in by looking for
/// `" Safari/"`, and a bare WKWebView leaves that off: it found no browser at all, and
/// the popup died on a null device before it could draw anything.
// ponytail: a fixed version; read Safari's Info.plist if a gate ever needs the real one.
#[cfg(target_os = "macos")]
const SAFARI_UA: &str = "Version/26.0 Safari/605.1.15";

/// Bitwarden's own servers: the US and EU clouds and the icon server. An extension's
/// fetches skip CORS only where it holds host access, so with just your devices it
/// could see the NAS and not its own vault - sign-in died on `api.bitwarden.com`
/// refusing the `webkit-extension://` origin. Its own domains, and nobody else's.
/// A server of your own joins them from `[settings].bitwarden_server`.
#[cfg(target_os = "macos")]
const BITWARDEN_SERVERS: [&str; 3] = [
    "https://*.bitwarden.com/*",
    "https://*.bitwarden.eu/*",
    "https://*.bitwarden.net/*",
];

/// The three Objective-C objects WebKit asks questions of: a tab per web tab, the one
/// window they sit in, and the controller's delegate. Only what Bitwarden needs to
/// fill a login is answered; everything else takes WebKit's default, which for the
/// permission prompts is "denied" - it gets the hosts it was given and no more.
#[cfg(target_os = "macos")]
mod shim {
    use objc2::rc::Retained;
    use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
    use objc2::{define_class, msg_send, DefinedClass, MainThreadMarker, MainThreadOnly};
    use objc2_foundation::{NSArray, NSError};
    use objc2_web_kit::{
        WKWebExtensionAction, WKWebExtensionContext, WKWebExtensionController,
        WKWebExtensionControllerDelegate, WKWebExtensionTab, WKWebExtensionWindow, WKWebView,
    };

    pub struct TabIvars {
        pub id: u32,
        pub view: Retained<WKWebView>,
    }

    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "PatchbayExtensionTab"]
        #[ivars = TabIvars]
        pub struct Tab;

        unsafe impl NSObjectProtocol for Tab {}

        // The url and title default to the web view's own, which is exactly right.
        unsafe impl WKWebExtensionTab for Tab {
            #[unsafe(method_id(windowForWebExtensionContext:))]
            fn window_for(
                &self,
                _context: &WKWebExtensionContext,
            ) -> Option<Retained<ProtocolObject<dyn WKWebExtensionWindow>>> {
                super::live::read(|h| h.window.clone()).map(ProtocolObject::from_retained)
            }

            #[unsafe(method_id(webViewForWebExtensionContext:))]
            fn web_view_for(
                &self,
                _context: &WKWebExtensionContext,
            ) -> Option<Retained<WKWebView>> {
                Some(self.ivars().view.clone())
            }
        }
    );

    impl Tab {
        pub fn new(mtm: MainThreadMarker, id: u32, view: Retained<WKWebView>) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(TabIvars { id, view });
            unsafe { msg_send![super(this), init] }
        }
    }

    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "PatchbayExtensionWindow"]
        pub struct Window;

        unsafe impl NSObjectProtocol for Window {}

        unsafe impl WKWebExtensionWindow for Window {
            #[unsafe(method_id(tabsForWebExtensionContext:))]
            fn tabs_for(
                &self,
                _context: &WKWebExtensionContext,
            ) -> Retained<NSArray<ProtocolObject<dyn WKWebExtensionTab>>> {
                let tabs: Vec<_> = super::live::read(|h| h.tabs.clone())
                    .unwrap_or_default()
                    .into_iter()
                    .map(ProtocolObject::from_retained)
                    .collect();
                NSArray::from_retained_slice(&tabs)
            }

            #[unsafe(method_id(activeTabForWebExtensionContext:))]
            fn active_for(
                &self,
                _context: &WKWebExtensionContext,
            ) -> Option<Retained<ProtocolObject<dyn WKWebExtensionTab>>> {
                super::live::read(|h| {
                    let id = h.active?;
                    h.tabs.iter().find(|t| t.ivars().id == id).cloned()
                })
                .flatten()
                .map(ProtocolObject::from_retained)
            }

            // Bitwarden places its unlock window from this one's `left` and `width`.
            // Left at WebKit's null rect those are missing, the sum is NaN, and
            // `windows.create` throws before the delegate below is ever asked: a fill
            // while signed out or locked did nothing at all.
            #[unsafe(method(frameForWebExtensionContext:))]
            fn frame_for(&self, _context: &WKWebExtensionContext) -> objc2_foundation::NSRect {
                super::live::read(|h| {
                    let view = h.tabs.first()?.ivars().view.clone();
                    Some(view.window()?.frame())
                })
                .flatten()
                .unwrap_or(objc2_foundation::NSRect::ZERO)
            }
        }
    );

    impl Window {
        pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(());
            unsafe { msg_send![super(this), init] }
        }
    }

    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "PatchbayExtensionDelegate"]
        pub struct Delegate;

        unsafe impl NSObjectProtocol for Delegate {}

        unsafe impl WKWebExtensionControllerDelegate for Delegate {
            #[unsafe(method_id(webExtensionController:openWindowsForExtensionContext:))]
            fn open_windows(
                &self,
                _controller: &WKWebExtensionController,
                _context: &WKWebExtensionContext,
            ) -> Retained<NSArray<ProtocolObject<dyn WKWebExtensionWindow>>> {
                let w: Vec<_> = super::live::read(|h| h.window.clone())
                    .into_iter()
                    .map(ProtocolObject::from_retained)
                    .collect();
                NSArray::from_retained_slice(&w)
            }

            #[unsafe(method_id(webExtensionController:focusedWindowForExtensionContext:))]
            fn focused_window(
                &self,
                _controller: &WKWebExtensionController,
                _context: &WKWebExtensionContext,
            ) -> Option<Retained<ProtocolObject<dyn WKWebExtensionWindow>>> {
                super::live::read(|h| h.window.clone()).map(ProtocolObject::from_retained)
            }

            // Bitwarden's Safari build never touches the clipboard itself: a copy is a
            // message to the app around the extension, which in Safari is Bitwarden's
            // own app extension and here is us. Unanswered, Copy password in the popup
            // quietly did nothing at all.
            #[unsafe(method(webExtensionController:sendMessage:toApplicationWithIdentifier:forExtensionContext:replyHandler:))]
            fn send_message(
                &self,
                _controller: &WKWebExtensionController,
                message: &objc2::runtime::AnyObject,
                _application: Option<&objc2_foundation::NSString>,
                _context: &WKWebExtensionContext,
                reply: &block2::DynBlock<dyn Fn(*mut objc2::runtime::AnyObject, *mut NSError)>,
            ) {
                let answer = super::app_message(message);
                let ptr = answer
                    .as_ref()
                    .map(|s| Retained::as_ptr(s) as *mut objc2::runtime::AnyObject)
                    .unwrap_or(std::ptr::null_mut());
                reply.call((ptr, std::ptr::null_mut()));
            }

            // A locked Bitwarden asked to fill opens its unlock screen as a window of its
            // own. There is one window here, so it opens as the popup on the key instead:
            // declined, the key did nothing at all and said nothing about why. Anything
            // else it would open as a window lands on the popup too.
            #[unsafe(method(webExtensionController:openNewWindowUsingConfiguration:forExtensionContext:completionHandler:))]
            fn open_new_window(
                &self,
                _controller: &WKWebExtensionController,
                _configuration: &objc2::runtime::AnyObject,
                context: &WKWebExtensionContext,
                done: &block2::DynBlock<
                    dyn Fn(*mut ProtocolObject<dyn WKWebExtensionWindow>, *mut NSError),
                >,
            ) {
                let tab = super::live::read(|h| {
                    let id = h.active?;
                    h.tabs.iter().find(|t| t.ivars().id == id).cloned()
                })
                .flatten();
                unsafe {
                    context.performActionForTab(tab.as_deref().map(ProtocolObject::from_ref))
                };
                done.call((std::ptr::null_mut(), std::ptr::null_mut()));
            }

            #[unsafe(method(webExtensionController:presentPopupForAction:forExtensionContext:completionHandler:))]
            fn present_popup(
                &self,
                _controller: &WKWebExtensionController,
                action: &WKWebExtensionAction,
                _context: &WKWebExtensionContext,
                done: &block2::DynBlock<dyn Fn(*mut NSError)>,
            ) {
                super::show_popup(action);
                done.call((std::ptr::null_mut(),));
            }
        }
    );

    impl Delegate {
        pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(());
            unsafe { msg_send![super(this), init] }
        }
    }
}

/// A web tab, told to the extension. `view` is the tab's `WKWebView`, handed over by
/// `with_webview`, which runs this on the main thread.
#[cfg(target_os = "macos")]
pub fn open_tab(id: u32, view: *mut std::ffi::c_void) {
    use objc2::{rc::Retained, runtime::ProtocolObject, MainThreadMarker};
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let Some(view) = (unsafe { Retained::retain(view.cast::<objc2_web_kit::WKWebView>()) }) else {
        return;
    };
    let tab = shim::Tab::new(mtm, id, view);
    let Some(controller) = live::read(|h| {
        h.tabs.push(tab.clone());
        h.controller.clone()
    }) else {
        return;
    };
    unsafe { controller.didOpenTab(ProtocolObject::from_ref(&*tab)) };
    activate(id);
}

#[cfg(target_os = "macos")]
pub fn close_tab(id: u32) {
    use objc2::{runtime::ProtocolObject, DefinedClass};
    let gone = live::read(|h| {
        let at = h.tabs.iter().position(|t| t.ivars().id == id)?;
        if h.active == Some(id) {
            h.active = None;
        }
        Some((h.controller.clone(), h.tabs.remove(at)))
    })
    .flatten();
    if let Some((controller, tab)) = gone {
        unsafe { controller.didCloseTab_windowIsClosing(ProtocolObject::from_ref(&*tab), false) };
    }
}

/// Called on every placement of a web tab, so WebKit is only told when the tab in
/// front actually changed.
#[cfg(target_os = "macos")]
pub fn activate(id: u32) {
    use objc2::{runtime::ProtocolObject, DefinedClass};
    let change = live::read(|h| {
        if h.active == Some(id) {
            return None;
        }
        let now = h.tabs.iter().find(|t| t.ivars().id == id)?.clone();
        let before = h
            .active
            .and_then(|a| h.tabs.iter().find(|t| t.ivars().id == a).cloned());
        h.active = Some(id);
        Some((h.controller.clone(), now, before))
    })
    .flatten();
    if let Some((controller, now, before)) = change {
        unsafe {
            controller.didActivateTab_previousActiveTab(
                ProtocolObject::from_ref(&*now),
                before.as_deref().map(ProtocolObject::from_ref),
            )
        };
    }
}

/// The key button on a web tab. A click fills, the way ⌘⇧L does in a browser: a login
/// in steps (a Synology asks for the name, then the password) is one click per step,
/// where the popup closed itself after every fill and had to be opened again. `popup`
/// opens Bitwarden instead, for signing in and picking by hand.
///
/// `id` is the tab the key sits on, or `None` from the toolbar's own key, which is a
/// vault to look something up in rather than a page to fill: whatever tab is in front
/// is used if there is one, and Bitwarden opens unattached when there isn't.
///
/// `view` is the window's content view and `at` the button's rect, in CSS pixels from
/// the top. A locked vault answers a fill with the popup, which hangs from there too.
///
/// A fill hands back a receiver that answers whether the page saw it land, for the key
/// to pulse until then and open the popup on a miss; see `watch_fill`.
#[cfg(target_os = "macos")]
pub fn key(
    view: *mut std::ffi::c_void,
    id: Option<u32>,
    at: (f64, f64, f64, f64),
    popup: bool,
) -> Result<Option<std::sync::mpsc::Receiver<bool>>, String> {
    let (x, y, width, height) = at;
    use objc2::{rc::Retained, runtime::ProtocolObject, DefinedClass, MainThreadMarker};
    use objc2_app_kit::NSView;
    use objc2_foundation::{NSPoint, NSRect, NSRectEdge, NSSize};

    if MainThreadMarker::new().is_none() {
        return Err("not on the main thread".into());
    }
    if !running() {
        return Err("Bitwarden isn't running - turn it on in Settings".into());
    }
    let view =
        unsafe { Retained::retain(view.cast::<NSView>()) }.ok_or("no window to show it in")?;
    // CSS counts down from the top; an unflipped AppKit view counts up from the bottom.
    let (y, edge) = match view.isFlipped() {
        true => (y, NSRectEdge::MaxY),
        false => (view.bounds().size.height - y - height, NSRectEdge::MinY),
    };
    let rect = NSRect::new(NSPoint::new(x, y), NSSize::new(width, height));
    if let Some(id) = id {
        activate(id);
    }
    let (context, tab) = live::read(|h| {
        h.anchor = Some((view.clone(), rect, edge));
        // The toolbar's key names no tab, so it acts on whatever is in front.
        let want = id.or(h.active);
        let tab = want.and_then(|w| h.tabs.iter().find(|t| t.ivars().id == w).cloned());
        Some((h.context.clone()?, tab))
    })
    .flatten()
    .ok_or("Bitwarden isn't running - turn it on in Settings")?;
    let mut landed = None;
    if popup {
        unsafe { context.performActionForTab(tab.as_deref().map(ProtocolObject::from_ref)) };
    } else {
        let tab =
            tab.ok_or("this tab opened before Bitwarden was on - close it and open it again")?;
        // Bitwarden's own "autofill_login", which acts on the active tab `activate` set.
        let fill = unsafe { context.commands() }
            .iter()
            .find(|c| unsafe { c.identifier() }.to_string() == "autofill_login")
            .ok_or("this Bitwarden has no fill command - right-click the key to fill by hand")?;
        // Listening before the command goes, or a quick fill lands before the listener.
        landed = Some(watch_fill(&tab.ivars().view));
        unsafe { context.performCommand(&fill) };
    }
    log_errors(&context);
    Ok(landed)
}

/// Answers `true` once the page sees an `input` event, which is how Bitwarden's fill
/// announces itself to the form, or once the page goes away under it; `false` when
/// nothing came. Bitwarden itself says nothing when a fill is done or failed: signed
/// out, no login saved for this address, or a form it doesn't recognise (a Vistas NAS)
/// all look the same, so a miss opens the popup and the user sees which it was. Its
/// worker can take seconds to wake, hence the 5 s rather than one. The listener runs
/// in a world of its own, so the page can neither see nor touch it.
/// ponytail: a field filled inside an iframe doesn't reach the top document, so a fill
/// there also opens the popup after 5 s; listen in every frame if a device does that.
#[cfg(target_os = "macos")]
fn watch_fill(view: &objc2_web_kit::WKWebView) -> std::sync::mpsc::Receiver<bool> {
    use objc2::MainThreadMarker;
    use objc2_foundation::NSString;
    use objc2_web_kit::WKContentWorld;
    const LISTEN: &str = "return await new Promise((r) => {
        addEventListener(\"input\", () => r(\"landed\"), { capture: true, once: true });
        setTimeout(() => r(\"missed\"), 5000);
    });";
    let (tx, rx) = std::sync::mpsc::channel();
    let tx = std::cell::Cell::new(Some(tx));
    let done = block2::RcBlock::new(
        move |got: *mut objc2::runtime::AnyObject, err: *mut objc2_foundation::NSError| {
            // An error is the page navigating away mid-wait, which a submitted login does.
            let missed = err.is_null()
                && unsafe { got.as_ref() }
                    .and_then(|g| g.downcast_ref::<NSString>())
                    .is_some_and(|g| g.to_string() == "missed");
            if let Some(tx) = tx.take() {
                let _ = tx.send(!missed);
            }
        },
    );
    let mtm = MainThreadMarker::from(view);
    unsafe {
        view.callAsyncJavaScript_arguments_inFrame_inContentWorld_completionHandler(
            &NSString::from_str(LISTEN),
            None,
            None,
            &WKContentWorld::defaultClientWorld(mtm),
            Some(&done),
        )
    };
    rx
}

/// What WebKit recorded going wrong in the extension since it loaded, to the dev log.
#[cfg(target_os = "macos")]
fn log_errors(context: &objc2_web_kit::WKWebExtensionContext) {
    for e in unsafe { context.errors() }.iter() {
        eprintln!("patchbay: bitwarden: {}", e.localizedDescription());
    }
}

/// The `{ command, data }` an extension sends to the application around it. Only
/// Bitwarden's two clipboard commands are answered - the rest of its native messaging
/// talks to the Bitwarden desktop app's own IPC, which is not us and never will be.
#[cfg(target_os = "macos")]
fn app_message(
    message: &objc2::runtime::AnyObject,
) -> Option<objc2::rc::Retained<objc2_foundation::NSString>> {
    use objc2::runtime::AnyObject;
    use objc2_foundation::{NSDictionary, NSString};

    let dict = message.downcast_ref::<NSDictionary>()?;
    let at = |k: &str| -> Option<objc2::rc::Retained<AnyObject>> {
        dict.objectForKey(&NSString::from_str(k))
    };
    let command = at("command")?.downcast::<NSString>().ok()?.to_string();
    match command.as_str() {
        "copyToClipboard" => {
            let text = at("data")?.downcast::<NSString>().ok()?;
            crate::clipboard::set_local_text(&text.to_string());
            None
        }
        "readFromClipboard" => Some(NSString::from_str(&crate::clipboard::local_text()?)),
        _ => None,
    }
}

/// What the delegate does once the popup page has loaded.
#[cfg(target_os = "macos")]
fn show_popup(action: &objc2_web_kit::WKWebExtensionAction) {
    let Some((view, rect, edge)) = live::read(|h| h.anchor.clone()).flatten() else {
        return;
    };
    if let Some(popover) = unsafe { action.popupPopover() } {
        popover.showRelativeToRect_ofView_preferredEdge(rect, &view, edge);
    }
}

/// The toggle going off. Only the context goes: the controller and its tabs stay for
/// the next `load`, and an unloaded context runs nothing in them meanwhile.
#[cfg(target_os = "macos")]
pub fn unload() -> Result<(), String> {
    if objc2::MainThreadMarker::new().is_none() {
        return Err("not on the main thread".into());
    }
    let Some((controller, context)) = live::down() else {
        return Ok(());
    };
    unsafe { controller.unloadExtensionContext_error(&context) }
        .map_err(|e| e.localizedDescription().to_string())
}

/// Hand a context the pages it may see, and say how many of them are devices. Before
/// it loads, and again on a running one: host access set once at launch left a device
/// added since with a key that could fill nothing until the next restart.
#[cfg(target_os = "macos")]
unsafe fn grant(
    ctx: &objc2_web_kit::WKWebExtensionContext,
    mtm: objc2::MainThreadMarker,
    hosts: &[String],
    server: Option<&str>,
) -> usize {
    use objc2_foundation::{NSDate, NSDictionary, NSString};
    use objc2_web_kit::WKWebExtensionMatchPattern;
    let pattern =
        |h: &str| WKWebExtensionMatchPattern::matchPatternWithString(&NSString::from_str(h), mtm);
    // None is fine: with no device page to fill, the vault still opens.
    let mut patterns: Vec<_> = hosts.iter().filter_map(|h| pattern(h)).collect();
    let devices = patterns.len();
    patterns.extend(
        BITWARDEN_SERVERS
            .iter()
            .copied()
            .chain(server)
            .filter_map(pattern),
    );
    // A dictionary of pattern -> when it expires, and an expiry is not what decides
    // this: the toggle is, and turning it off unloads the context outright.
    let forever = NSDate::distantFuture();
    let pk: Vec<_> = patterns.iter().map(|p| &**p).collect();
    let pw = vec![&*forever; pk.len()];
    ctx.setGrantedPermissionMatchPatterns(&NSDictionary::from_slices(&pk, &pw));
    devices
}

/// Load the installed Bitwarden extension into the controller, on the main thread. A
/// second call grants the hosts anew and reports what is running, and loads again
/// only when Bitwarden for Mac has been updated since.
///
/// `hosts` are the pages it may see: one match pattern per device URL in the list, plus
/// Bitwarden's own servers. Never `allHostsAndSchemesMatchPattern` - an extension that
/// can read every site is the thing patchbay would be adding, and the pages here are a
/// handful of appliances you named yourself. An empty list is not a refusal: the vault
/// is worth opening for a password to read even where nothing here can be filled.
#[cfg(target_os = "macos")]
pub async fn load(
    app: &tauri::AppHandle,
    hosts: Vec<String>,
    server: Option<String>,
) -> Result<Package, String> {
    use block2::RcBlock;
    use objc2::{runtime::ProtocolObject, MainThreadMarker};
    use objc2_foundation::{NSDate, NSDictionary, NSError, NSString, NSURL};
    use objc2_web_kit::{
        WKWebExtension, WKWebExtensionContext, WKWebExtensionController,
        WKWebExtensionControllerConfiguration,
    };

    if !supported() {
        return Err("this needs macOS 15.4 or newer".into());
    }
    let path = package()?;
    let name = path.display().to_string();
    // Read before the package is, so an update landing mid-load is seen next time.
    let at = stamp();
    let (tx, rx) = std::sync::mpsc::channel::<Result<Package, String>>();

    app.run_on_main_thread(move || {
        let Some(mtm) = MainThreadMarker::new() else {
            let _ = tx.send(Err("not on the main thread".into()));
            return;
        };
        // Settings asks every time it opens and the window after every change to the
        // list. The same build still running only needs its hosts brought up to date.
        let reach = (hosts, server);
        let held = live::read(|h| Some((h.context.clone()?, h.granted != reach))).flatten();
        if let Some((ctx, moved)) = held {
            if at.is_none() || at == live::stamp() {
                if moved {
                    let n = unsafe { grant(&ctx, mtm, &reach.0, reach.1.as_deref()) };
                    live::read(|h| {
                        h.facts.hosts = n;
                        h.granted = reach;
                    });
                }
                let _ =
                    tx.send(live::read(|h| h.facts.clone()).ok_or("Bitwarden went away".into()));
                return;
            }
            if let Err(e) = unload() {
                eprintln!("patchbay: bitwarden: unloading the old build: {e}");
            }
        }
        let url = NSURL::fileURLWithPath(&NSString::from_str(&name));
        let done = RcBlock::new(move |ext: *mut WKWebExtension, err: *mut NSError| {
            let facts = match unsafe { read_package(ext, err) } {
                Ok(p) => p,
                Err(e) => {
                    let _ = tx.send(Err(e));
                    return;
                }
            };
            let out = unsafe {
                let ext = &*ext;
                let ctx = WKWebExtensionContext::contextForExtension(ext);
                // Dev builds only: Safari's Develop menu can then open the background
                // page and the popup, which is the only place their console errors are.
                ctx.setInspectable(cfg!(debug_assertions));
                ctx.setInspectionName(Some(&NSString::from_str("patchbay: Bitwarden")));
                // Left at its random default, WebKit keeps the extension's storage in
                // memory only, and every launch is a Bitwarden to sign in to again. The
                // base URL is the origin its own pages store under, so it stays put too.
                ctx.setUniqueIdentifier(&NSString::from_str(BITWARDEN_ID));
                let base = format!("webkit-extension://{BITWARDEN_ID}/");
                if let Some(base) = NSURL::URLWithString(&NSString::from_str(&base)) {
                    ctx.setBaseURL(&base);
                }
                // The API permissions it declared (storage, tabs, and so on), each
                // until the toggle goes off. Host access is `grant`'s separate list,
                // and that is the one that matters.
                let forever = NSDate::distantFuture();
                let asked = ext.requestedPermissions();
                let asked: Vec<_> = asked.iter().collect();
                let keys: Vec<_> = asked.iter().map(|p| &**p).collect();
                let whens = vec![&*forever; keys.len()];
                ctx.setGrantedPermissions(&NSDictionary::from_slices(&keys, &whens));
                let devices = grant(&ctx, mtm, &reach.0, reach.1.as_deref());

                let before = live::read(|h| (h.controller.clone(), h.window.clone()));
                let fresh = before.is_none();
                let (controller, window) = before.unwrap_or_else(|| {
                    let conf = WKWebExtensionControllerConfiguration::defaultConfiguration(mtm);
                    // The basis for the extension's own web views: background and popup.
                    let web = conf.webViewConfiguration();
                    web.setApplicationNameForUserAgent(Some(&NSString::from_str(SAFARI_UA)));
                    conf.setWebViewConfiguration(Some(&web));
                    (
                        WKWebExtensionController::initWithConfiguration(mtm.alloc(), &conf),
                        shim::Window::new(mtm),
                    )
                });
                match controller.loadExtensionContext_error(&ctx) {
                    Ok(()) => {
                        let facts = Package {
                            loaded: ctx.isLoaded(),
                            hosts: devices,
                            ..facts
                        };
                        // Kept only once something loaded in it: a controller that
                        // never ran an extension is not one to hand to every tab.
                        if fresh {
                            let delegate = shim::Delegate::new(mtm);
                            controller.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
                            live::set(live::Held {
                                controller: controller.clone(),
                                context: None,
                                facts: Package::default(),
                                granted: Default::default(),
                                _delegate: delegate,
                                window: window.clone(),
                                tabs: Vec::new(),
                                active: None,
                                anchor: None,
                            });
                        }
                        live::up(ctx.clone(), facts.clone(), reach.clone(), at);
                        // After `set`: WebKit asks the window for its tabs straight away.
                        // Said again to a context loaded later, which was not there to
                        // hear it the first time.
                        controller.didOpenWindow(ProtocolObject::from_ref(&*window));
                        controller.didFocusWindow(Some(ProtocolObject::from_ref(&*window)));
                        // A popup that spins forever is waiting on the background
                        // worker; starting it now says whether it can start at all.
                        let watched = ctx.clone();
                        let started = RcBlock::new(move |err: *mut NSError| {
                            if let Some(e) = err.as_ref() {
                                eprintln!(
                                    "patchbay: bitwarden background failed: {}",
                                    e.localizedDescription()
                                );
                            }
                            log_errors(&watched);
                        });
                        ctx.loadBackgroundContentWithCompletionHandler(&started);
                        Ok(facts)
                    }
                    Err(e) => Err(e.localizedDescription().to_string()),
                }
            };
            let _ = tx.send(out);
        });
        unsafe { WKWebExtension::extensionWithResourceBaseURL_completionHandler(&url, &done, mtm) };
    })
    .map_err(|e| format!("could not reach the main thread: {e}"))?;

    tauri::async_runtime::spawn_blocking(move || {
        rx.recv_timeout(std::time::Duration::from_secs(30))
            .unwrap_or_else(|_| Err("loading the extension took too long".into()))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// A device's url as the one match pattern that covers it and nothing beside it: the
/// host exactly, every path. A device listed as http is granted on https too, because
/// the url in the list is often not the page with the form on it: a Synology's http
/// port is three lines of JavaScript pointing at its https one. Never the other way:
/// a device listed as https has no business being filled over cleartext. A port is not
/// part of a match pattern, so two ports on one appliance are the same pattern, which
/// is the same machine either way.
///
/// A `*` in the host is refused. The url parser lets it through and a match pattern
/// reads it as a wildcard, so `http://*/` in a colleague's list was every site there is.
pub fn host_pattern(url: &str) -> Option<String> {
    let u = tauri::Url::parse(url.trim()).ok()?;
    let host = u.host_str().filter(|h| !h.contains('*'))?;
    match u.scheme() {
        "http" => Some(format!("*://{host}/*")),
        "https" => Some(format!("https://{host}/*")),
        _ => None,
    }
}

/// A self-hosted Bitwarden's address as its match pattern. https only: the vault's
/// own traffic, and Bitwarden's clients refuse a cleartext server anyway.
pub fn server_pattern(url: &str) -> Option<String> {
    host_pattern(url).filter(|p| p.starts_with("https://"))
}

#[cfg(not(target_os = "macos"))]
pub fn stale() -> bool {
    false
}

#[cfg(not(target_os = "macos"))]
pub fn supported() -> bool {
    false
}

#[cfg(not(target_os = "macos"))]
pub async fn load(
    _app: &tauri::AppHandle,
    _hosts: Vec<String>,
    _server: Option<String>,
) -> Result<Package, String> {
    Err("hosting a browser extension only works on macOS".into())
}

#[cfg(not(target_os = "macos"))]
pub fn unload() -> Result<(), String> {
    Ok(())
}

/// WebView2 can host a Chrome extension and WebKitGTK cannot, but neither shares this
/// path: `WKWebExtension` is what the tab protocols below are written against.
#[cfg(not(target_os = "macos"))]
pub async fn inspect(
    _app: &tauri::AppHandle,
    _path: std::path::PathBuf,
) -> Result<Package, String> {
    Err("hosting a browser extension only works on macOS".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_is_granted_on_https_too_never_the_reverse_and_never_a_wildcard() {
        let on = |u| host_pattern(u);
        assert_eq!(
            on("http://nas.local:5000/").as_deref(),
            Some("*://nas.local/*")
        );
        assert_eq!(
            on(" https://nas.local:5001/x ").as_deref(),
            Some("https://nas.local/*")
        );
        assert_eq!(on("ftp://nas.local/"), None);
        assert_eq!(on("-oProxyCommand=x"), None);
        for wild in [
            "http://*/",
            "https://*.example.com/",
            "http://nas*.local/",
            "http://%2A/",
        ] {
            assert_eq!(on(wild), None, "{wild}");
            assert_eq!(server_pattern(wild), None, "{wild}");
        }
        assert_eq!(server_pattern("http://vault.example.com"), None);
    }
}
