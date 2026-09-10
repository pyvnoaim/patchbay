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

use serde::Serialize;

/// What a package turned out to be, for the Settings panel to show. Off macOS nothing
/// builds one - every command there refuses first - and CI's clippy runs on Linux.
#[derive(Serialize, Default, Debug)]
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

/// The loaded extension, for the life of the app. One controller, because the tabs it
/// serves are one window's, and the extension's own state (its vault session, above
/// all) is meant to outlive any single tab.
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
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed};

    pub struct Held {
        pub controller: Retained<WKWebExtensionController>,
        pub context: Retained<WKWebExtensionContext>,
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
    static RUNNING: AtomicBool = AtomicBool::new(false);
    static HOSTS: AtomicUsize = AtomicUsize::new(0);

    pub fn set(h: Held, hosts: usize) {
        HELD.with(|x| *x.borrow_mut() = Some(h));
        HOSTS.store(hosts, Relaxed);
        RUNNING.store(true, Relaxed);
    }

    pub fn take() -> Option<Held> {
        RUNNING.store(false, Relaxed);
        HELD.with(|x| x.borrow_mut().take())
    }

    pub fn running() -> bool {
        RUNNING.load(Relaxed)
    }

    pub fn hosts() -> usize {
        HOSTS.load(Relaxed)
    }

    /// A short borrow, cloning out what the caller needs. WebKit calls back into the
    /// tab, window and delegate synchronously and each reads this again, so calling
    /// WebKit while the borrow is held is a `RefCell` panic.
    pub fn read<R>(f: impl FnOnce(&mut Held) -> R) -> Option<R> {
        HELD.with(|x| x.borrow_mut().as_mut().map(f))
    }
}

/// Whether the extension is up, so a web tab knows to hand it their configuration.
#[cfg(target_os = "macos")]
pub fn running() -> bool {
    live::running()
}

/// A webview configuration carrying the loaded controller, or `None` when nothing is
/// loaded. Main thread only, and it has to be handed to the *builder*: WebKit reads
/// the controller when the view is created, so setting it afterwards is too late.
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
// ponytail: cloud only; a self-hosted server's address joins these from a setting once
// someone runs one.
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
        view: Retained<WKWebView>,
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
/// `view` is the window's content view and `at` the button's rect, in CSS pixels from
/// the top. A locked vault answers a fill with the popup, which hangs from there too.
#[cfg(target_os = "macos")]
pub fn key(
    view: *mut std::ffi::c_void,
    id: u32,
    at: (f64, f64, f64, f64),
    popup: bool,
) -> Result<(), String> {
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
    activate(id);
    let (context, tab) = live::read(|h| {
        h.anchor = Some((view.clone(), rect, edge));
        let tab = h.tabs.iter().find(|t| t.ivars().id == id).cloned();
        (h.context.clone(), tab)
    })
    .ok_or("Bitwarden isn't running")?;
    let tab = tab.ok_or("this tab opened before Bitwarden was on - close it and open it again")?;
    if popup {
        unsafe { context.performActionForTab(Some(ProtocolObject::from_ref(&*tab))) };
    } else {
        // Bitwarden's own "autofill_login", which acts on the active tab `activate` set.
        let fill = unsafe { context.commands() }
            .iter()
            .find(|c| unsafe { c.identifier() }.to_string() == "autofill_login")
            .ok_or("this Bitwarden has no fill command - right-click the key to fill by hand")?;
        unsafe { context.performCommand(&fill) };
    }
    log_errors(&context);
    Ok(())
}

/// What WebKit recorded going wrong in the extension since it loaded, to the dev log.
#[cfg(target_os = "macos")]
fn log_errors(context: &objc2_web_kit::WKWebExtensionContext) {
    for e in unsafe { context.errors() }.iter() {
        eprintln!("patchbay: bitwarden: {}", e.localizedDescription());
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

/// The toggle going off. Tabs opened while it ran keep the controller in their
/// configuration, but an unloaded context runs nothing in them.
#[cfg(target_os = "macos")]
pub fn unload() -> Result<(), String> {
    if objc2::MainThreadMarker::new().is_none() {
        return Err("not on the main thread".into());
    }
    let Some(h) = live::take() else {
        return Ok(());
    };
    unsafe { h.controller.unloadExtensionContext_error(&h.context) }
        .map_err(|e| e.localizedDescription().to_string())
}

/// Load the installed Bitwarden extension into a controller, on the main thread. A
/// second call reports what is already running rather than loading it again.
///
/// `hosts` are the pages it may see: one match pattern per device URL in the list, plus
/// Bitwarden's own servers. Never `allHostsAndSchemesMatchPattern` - an extension that
/// can read every site is the thing patchbay would be adding, and the pages here are a
/// handful of appliances you named yourself.
#[cfg(target_os = "macos")]
pub async fn load(app: &tauri::AppHandle, hosts: Vec<String>) -> Result<Package, String> {
    use block2::RcBlock;
    use objc2::{runtime::ProtocolObject, MainThreadMarker};
    use objc2_foundation::{NSDate, NSDictionary, NSError, NSString, NSURL};
    use objc2_web_kit::{
        WKWebExtension, WKWebExtensionContext, WKWebExtensionController,
        WKWebExtensionControllerConfiguration, WKWebExtensionMatchPattern,
    };

    if !supported() {
        return Err("this needs macOS 15.4 or newer".into());
    }
    // Settings asks every time it opens; a second controller would strand every tab
    // already attached to the first.
    if running() {
        let mut p = inspect(app, package()?).await?;
        p.loaded = true;
        p.hosts = live::hosts();
        return Ok(p);
    }
    if hosts.is_empty() {
        return Err("no device in your list has a web address to fill in".into());
    }
    let path = package()?;
    let name = path.display().to_string();
    let (tx, rx) = std::sync::mpsc::channel::<Result<Package, String>>();

    app.run_on_main_thread(move || {
        let Some(mtm) = MainThreadMarker::new() else {
            let _ = tx.send(Err("not on the main thread".into()));
            return;
        };
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
                // Both grants are dictionaries of thing -> when it expires, and an
                // expiry is not what decides this: the toggle is, and turning it off
                // unloads the context outright.
                let forever = NSDate::distantFuture();
                // The API permissions it declared (storage, tabs, and so on). Host
                // access is the separate list below, and that is the one that matters.
                let asked = ext.requestedPermissions();
                let asked: Vec<_> = asked.iter().collect();
                let keys: Vec<_> = asked.iter().map(|p| &**p).collect();
                let whens = vec![&*forever; keys.len()];
                ctx.setGrantedPermissions(&NSDictionary::from_slices(&keys, &whens));

                let pattern = |h: &str| {
                    WKWebExtensionMatchPattern::matchPatternWithString(&NSString::from_str(h), mtm)
                };
                let mut patterns: Vec<_> = hosts.iter().filter_map(|h| pattern(h)).collect();
                if patterns.is_empty() {
                    let _ = tx.send(Err("none of those addresses is a usable pattern".into()));
                    return;
                }
                let devices = patterns.len();
                patterns.extend(BITWARDEN_SERVERS.iter().filter_map(|h| pattern(h)));
                let pk: Vec<_> = patterns.iter().map(|p| &**p).collect();
                let pw = vec![&*forever; pk.len()];
                ctx.setGrantedPermissionMatchPatterns(&NSDictionary::from_slices(&pk, &pw));
                let conf = WKWebExtensionControllerConfiguration::defaultConfiguration(mtm);
                // The basis for the extension's own web views: background and popup.
                let web = conf.webViewConfiguration();
                web.setApplicationNameForUserAgent(Some(&NSString::from_str(SAFARI_UA)));
                conf.setWebViewConfiguration(Some(&web));
                let controller =
                    WKWebExtensionController::initWithConfiguration(mtm.alloc(), &conf);
                match controller.loadExtensionContext_error(&ctx) {
                    Ok(()) => {
                        let seen = ctx.isLoaded();
                        let delegate = shim::Delegate::new(mtm);
                        controller.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
                        let window = shim::Window::new(mtm);
                        live::set(
                            live::Held {
                                controller: controller.clone(),
                                context: ctx.clone(),
                                _delegate: delegate,
                                window: window.clone(),
                                tabs: Vec::new(),
                                active: None,
                                anchor: None,
                            },
                            devices,
                        );
                        // After `set`: WebKit asks the window for its tabs straight away.
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
                        Ok(Package {
                            loaded: seen,
                            hosts: devices,
                            ..facts
                        })
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

/// A device's url as the one match pattern that covers it and nothing beside it:
/// scheme and host exactly, every path. A port is not part of a match pattern, so two
/// ports on one appliance are the same pattern, which is the same machine either way.
pub fn host_pattern(url: &str) -> Option<String> {
    let u = tauri::Url::parse(url.trim()).ok()?;
    let host = u.host_str()?;
    match u.scheme() {
        "http" | "https" => Some(format!("{}://{}/*", u.scheme(), host)),
        _ => None,
    }
}

#[cfg(not(target_os = "macos"))]
pub fn supported() -> bool {
    false
}

#[cfg(not(target_os = "macos"))]
pub async fn load(_app: &tauri::AppHandle, _hosts: Vec<String>) -> Result<Package, String> {
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
