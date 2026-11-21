use std::collections::BTreeMap;
use std::ffi::{CStr, CString, c_char, c_double, c_int, c_void};
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use libloading::Library;
use thiserror::Error;
use tokio::sync::mpsc;
use tracing::warn;

#[derive(Debug, Error)]
pub enum LibMpvError {
    #[error(
        "libmpv.so.2 not found: install mpv (Arch: sudo pacman -S mpv, Debian/Ubuntu: sudo apt install libmpv2, Fedora: sudo dnf install mpv-libs)"
    )]
    LibraryNotFound,
    #[error("libmpv missing symbol {0}")]
    MissingSymbol(&'static str),
    #[error("libmpv: {0}")]
    Mpv(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum MpvValue {
    None,
    String(String),
    Bool(bool),
    Int(i64),
    Double(f64),
    Array(Vec<MpvValue>),
    Map(BTreeMap<String, MpvValue>),
}

impl MpvValue {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(s) => Some(s.as_str()),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Double(d) => Some(*d),
            Self::Int(i) => Some(*i as f64),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Int(i) => Some(*i),
            Self::Double(d) => Some(*d as i64),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[MpvValue]> {
        match self {
            Self::Array(a) => Some(a.as_slice()),
            _ => None,
        }
    }

    pub fn as_map(&self) -> Option<&BTreeMap<String, MpvValue>> {
        match self {
            Self::Map(m) => Some(m),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub enum MpvEventMsg {
    CommandReply {
        request_id: u64,
        error: Option<String>,
        data: MpvValue,
    },
    GetPropertyReply {
        request_id: u64,
        error: Option<String>,
        data: MpvValue,
    },
    PropertyChange {
        name: String,
        data: MpvValue,
    },
    StartFile {
        playlist_entry_id: i64,
    },
    EndFile {
        reason: String,
        playlist_entry_id: i64,
        file_error: Option<String>,
    },
    FileLoaded,
    Shutdown {
        unexpected: bool,
    },
}

mod format {
    pub const STRING: i32 = 1;
    pub const OSD_STRING: i32 = 2;
    pub const FLAG: i32 = 3;
    pub const INT64: i32 = 4;
    pub const DOUBLE: i32 = 5;
    pub const NODE: i32 = 6;
    pub const NODE_ARRAY: i32 = 7;
    pub const NODE_MAP: i32 = 8;
}

mod event_id {
    pub const NONE: i32 = 0;
    pub const SHUTDOWN: i32 = 1;
    pub const GET_PROPERTY_REPLY: i32 = 3;
    pub const COMMAND_REPLY: i32 = 5;
    pub const START_FILE: i32 = 6;
    pub const END_FILE: i32 = 7;
    pub const FILE_LOADED: i32 = 8;
    pub const PROPERTY_CHANGE: i32 = 22;
}

#[repr(C)]
struct MpvEvent {
    event_id: c_int,
    error: c_int,
    reply_userdata: u64,
    data: *mut c_void,
}

#[repr(C)]
struct MpvEventProperty {
    name: *const c_char,
    format: c_int,
    data: *mut c_void,
}

#[repr(C)]
struct MpvEventStartFile {
    playlist_entry_id: i64,
}

#[repr(C)]
struct MpvEventEndFile {
    reason: c_int,
    error: c_int,
    playlist_entry_id: i64,
    playlist_insert_id: i64,
    playlist_insert_num_entries: c_int,
}

#[repr(C)]
union MpvNodeUnion {
    string: *mut c_char,
    flag: c_int,
    int64: i64,
    double_: c_double,
    list: *mut MpvNodeList,
    ba: *mut c_void,
}

#[repr(C)]
struct MpvNode {
    u: MpvNodeUnion,
    format: c_int,
}

#[repr(C)]
struct MpvNodeList {
    num: c_int,
    values: *mut MpvNode,
    keys: *mut *mut c_char,
}

#[repr(C)]
struct MpvEventCommand {
    result: MpvNode,
}

struct MpvVtable {
    _lib: Library,
    create: unsafe extern "C" fn() -> *mut c_void,
    initialize: unsafe extern "C" fn(*mut c_void) -> c_int,
    terminate_destroy: unsafe extern "C" fn(*mut c_void),
    wakeup: unsafe extern "C" fn(*mut c_void),
    error_string: unsafe extern "C" fn(c_int) -> *const c_char,
    wait_event: unsafe extern "C" fn(*mut c_void, c_double) -> *mut MpvEvent,
    set_option_string: unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> c_int,
    command_async: unsafe extern "C" fn(*mut c_void, u64, *mut *const c_char) -> c_int,
    get_property_async: unsafe extern "C" fn(*mut c_void, u64, *const c_char, c_int) -> c_int,
    set_property_async:
        unsafe extern "C" fn(*mut c_void, u64, *const c_char, c_int, *mut c_void) -> c_int,
    observe_property: unsafe extern "C" fn(*mut c_void, u64, *const c_char, c_int) -> c_int,
}

unsafe impl Send for MpvVtable {}
unsafe impl Sync for MpvVtable {}

static MPV_VTABLE: OnceLock<Result<MpvVtable, String>> = OnceLock::new();

fn load_vtable() -> Result<&'static MpvVtable, LibMpvError> {
    let res = MPV_VTABLE.get_or_init(|| unsafe {
        let lib = Library::new("libmpv.so.2")
            .or_else(|_| Library::new("libmpv.so"))
            .map_err(|_| "not_found".to_owned())?;

        macro_rules! sym {
            ($name:literal) => {
                *lib.get($name)
                    .map_err(|_| String::from_utf8_lossy($name).into_owned())?
            };
        }

        let vt = MpvVtable {
            create: sym!(b"mpv_create\0"),
            initialize: sym!(b"mpv_initialize\0"),
            terminate_destroy: sym!(b"mpv_terminate_destroy\0"),
            wakeup: sym!(b"mpv_wakeup\0"),
            error_string: sym!(b"mpv_error_string\0"),
            wait_event: sym!(b"mpv_wait_event\0"),
            set_option_string: sym!(b"mpv_set_option_string\0"),
            command_async: sym!(b"mpv_command_async\0"),
            get_property_async: sym!(b"mpv_get_property_async\0"),
            set_property_async: sym!(b"mpv_set_property_async\0"),
            observe_property: sym!(b"mpv_observe_property\0"),
            _lib: lib,
        };
        Ok(vt)
    });

    match res {
        Ok(vt) => Ok(vt),
        Err(e) if e == "not_found" => Err(LibMpvError::LibraryNotFound),
        Err(_) => Err(LibMpvError::MissingSymbol("mpv_*")),
    }
}

/// Safe wrapper around a running `mpv_handle`.
/// All C pointer calls happen through this type; [`LibMpvClient::shutdown`] (also called by `Drop`)
/// joins the event thread before calling `mpv_terminate_destroy` under the handle write lock.
pub struct LibMpvClient {
    vt: &'static MpvVtable,
    handle: RwLock<Option<usize>>,
    disposing: Arc<AtomicBool>,
    shut_down: Arc<AtomicBool>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl LibMpvClient {
    pub fn start(
        options: &[(&str, &str)],
        tx: mpsc::UnboundedSender<MpvEventMsg>,
    ) -> Result<Self, LibMpvError> {
        let vt = load_vtable()?;
        let handle = unsafe { (vt.create)() };
        if handle.is_null() {
            return Err(LibMpvError::Mpv("mpv_create failed".to_owned()));
        }

        for &(name, value) in options {
            if let (Ok(cn), Ok(cv)) = (CString::new(name), CString::new(value)) {
                let rc = unsafe { (vt.set_option_string)(handle, cn.as_ptr(), cv.as_ptr()) };
                if rc < 0 {
                    let msg = unsafe { error_str(vt, rc) };
                    warn!(option = name, value, error = %msg, "mpv_set_option_string failed");
                }
            }
        }

        let err = unsafe { (vt.initialize)(handle) };
        if err < 0 {
            let msg = unsafe { error_str(vt, err) };
            unsafe { (vt.terminate_destroy)(handle) };
            return Err(LibMpvError::Mpv(msg));
        }

        let disposing = Arc::new(AtomicBool::new(false));
        let shut_down = Arc::new(AtomicBool::new(false));
        let h_usize = handle as usize;
        let disp_bg = Arc::clone(&disposing);
        let shut_bg = Arc::clone(&shut_down);

        let thread = thread::Builder::new()
            .name("libmpv-events".to_owned())
            .spawn(move || {
                let h = h_usize as *mut c_void;
                loop {
                    let ev = unsafe { (vt.wait_event)(h, -1.0) };
                    if disp_bg.load(Ordering::Acquire) {
                        break;
                    }
                    if ev.is_null() {
                        continue;
                    }
                    let id = unsafe { (*ev).event_id };
                    if id == event_id::SHUTDOWN {
                        break;
                    }
                    if id == event_id::NONE {
                        continue;
                    }
                    if let Some(msg) = unsafe { parse_event(vt, ev) } {
                        let _ = tx.send(msg);
                    }
                }
                let unexpected = !disp_bg.load(Ordering::Acquire);
                shut_bg.store(true, Ordering::Release);
                // Do not call mpv_terminate_destroy here: concurrent FFI callers holding `handle`
                // check `shut_down`/`disposing` under `handle.read()`, and `shutdown()` destroys
                // the handle under `handle.write()` after joining this thread.
                if unexpected {
                    let _ = tx.send(MpvEventMsg::Shutdown { unexpected: true });
                }
            })
            .map_err(|e| LibMpvError::Mpv(format!("failed to spawn mpv thread: {e}")))?;

        Ok(Self {
            vt,
            handle: RwLock::new(Some(h_usize)),
            disposing,
            shut_down,
            thread: Mutex::new(Some(thread)),
        })
    }

    pub fn is_alive(&self) -> bool {
        !self.shut_down.load(Ordering::Acquire) && !self.disposing.load(Ordering::Acquire)
    }

    fn with_live_handle<F>(&self, f: F)
    where
        F: FnOnce(*mut c_void),
    {
        let guard = match self.handle.read() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        if !self.is_alive() {
            return;
        }
        if let Some(h) = *guard {
            f(h as *mut c_void);
        }
    }

    pub fn command(&self, user_data: u64, args: &[&str]) {
        let Ok(cstrings) = args
            .iter()
            .map(|s| CString::new(*s))
            .collect::<Result<Vec<_>, _>>()
        else {
            return;
        };
        let mut ptrs: Vec<*const c_char> = cstrings.iter().map(|s| s.as_ptr()).collect();
        ptrs.push(ptr::null());
        self.with_live_handle(|h| unsafe {
            (self.vt.command_async)(h, user_data, ptrs.as_mut_ptr());
        });
    }

    pub fn get_property(&self, user_data: u64, name: &str) {
        let Ok(cn) = CString::new(name) else {
            return;
        };
        self.with_live_handle(|h| unsafe {
            (self.vt.get_property_async)(h, user_data, cn.as_ptr(), format::NODE);
        });
    }

    pub fn set_property(&self, name: &str, value: &str) {
        let (Ok(cn), Ok(cv)) = (CString::new(name), CString::new(value)) else {
            return;
        };
        let mut ptr = cv.as_ptr();
        self.with_live_handle(|h| unsafe {
            (self.vt.set_property_async)(
                h,
                0,
                cn.as_ptr(),
                format::STRING,
                (&raw mut ptr).cast::<c_void>(),
            );
        });
    }

    pub fn observe(&self, user_data: u64, name: &str) {
        let Ok(cn) = CString::new(name) else {
            return;
        };
        self.with_live_handle(|h| unsafe {
            (self.vt.observe_property)(h, user_data, cn.as_ptr(), format::NODE);
        });
    }

    pub fn shutdown(&self) {
        if self.disposing.swap(true, Ordering::AcqRel) {
            return;
        }
        if !self.shut_down.load(Ordering::Acquire) {
            let guard = match self.handle.read() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            if let Some(h) = *guard {
                unsafe { (self.vt.wakeup)(h as *mut c_void) };
            }
        }
        let thread = {
            let mut g = match self.thread.lock() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            g.take()
        };
        let mut joined = true;
        if let Some(handle) = thread {
            let start = std::time::Instant::now();
            while !handle.is_finished() && start.elapsed() < Duration::from_secs(2) {
                thread::sleep(Duration::from_millis(10));
            }
            if handle.is_finished() {
                let _ = handle.join();
            } else {
                joined = false;
                warn!("libmpv event thread did not finish within 2s; leaking handle to avoid UB");
            }
        }
        self.shut_down.store(true, Ordering::Release);
        let maybe_h = {
            let mut guard = match self.handle.write() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            guard.take()
        };
        if joined && let Some(h) = maybe_h {
            unsafe { (self.vt.terminate_destroy)(h as *mut c_void) };
        }
    }
}

impl Drop for LibMpvClient {
    fn drop(&mut self) {
        self.shutdown();
    }
}

unsafe fn error_str(vt: &MpvVtable, code: c_int) -> String {
    let ptr = unsafe { (vt.error_string)(code) };
    if ptr.is_null() {
        code.to_string()
    } else {
        unsafe { CStr::from_ptr(ptr) }.to_string_lossy().into_owned()
    }
}

unsafe fn parse_event(vt: &MpvVtable, ev: *mut MpvEvent) -> Option<MpvEventMsg> {
    let ev_ref = unsafe { &*ev };
    match ev_ref.event_id {
        event_id::COMMAND_REPLY => {
            if ev_ref.reply_userdata == 0 {
                return None;
            }
            let error = if ev_ref.error < 0 {
                Some(unsafe { error_str(vt, ev_ref.error) })
            } else {
                None
            };
            let data = if ev_ref.error >= 0 && !ev_ref.data.is_null() {
                let cmd = unsafe { &*(ev_ref.data as *const MpvEventCommand) };
                unsafe { read_node(&cmd.result) }
            } else {
                MpvValue::None
            };
            Some(MpvEventMsg::CommandReply {
                request_id: ev_ref.reply_userdata,
                error,
                data,
            })
        }
        event_id::GET_PROPERTY_REPLY => {
            if ev_ref.reply_userdata == 0 {
                return None;
            }
            let error = if ev_ref.error < 0 {
                Some(unsafe { error_str(vt, ev_ref.error) })
            } else {
                None
            };
            let data = if ev_ref.error >= 0 && !ev_ref.data.is_null() {
                let prop = unsafe { &*(ev_ref.data as *const MpvEventProperty) };
                unsafe { read_property(prop) }
            } else {
                MpvValue::None
            };
            Some(MpvEventMsg::GetPropertyReply {
                request_id: ev_ref.reply_userdata,
                error,
                data,
            })
        }
        event_id::PROPERTY_CHANGE => {
            if ev_ref.data.is_null() {
                return None;
            }
            let prop = unsafe { &*(ev_ref.data as *const MpvEventProperty) };
            let name = if prop.name.is_null() {
                String::new()
            } else {
                unsafe { CStr::from_ptr(prop.name) }
                    .to_string_lossy()
                    .into_owned()
            };
            let data = unsafe { read_property(prop) };
            Some(MpvEventMsg::PropertyChange { name, data })
        }
        event_id::START_FILE => {
            if ev_ref.data.is_null() {
                return None;
            }
            let sf = unsafe { &*(ev_ref.data as *const MpvEventStartFile) };
            Some(MpvEventMsg::StartFile {
                playlist_entry_id: sf.playlist_entry_id,
            })
        }
        event_id::END_FILE => {
            if ev_ref.data.is_null() {
                return None;
            }
            let ef = unsafe { &*(ev_ref.data as *const MpvEventEndFile) };
            let reason = match ef.reason {
                0 => "eof",
                2 => "stop",
                3 => "quit",
                4 => "error",
                5 => "redirect",
                _ => "unknown",
            }
            .to_owned();
            let file_error = if ef.reason == 4 {
                Some(unsafe { error_str(vt, ef.error) })
            } else {
                None
            };
            Some(MpvEventMsg::EndFile {
                reason,
                playlist_entry_id: ef.playlist_entry_id,
                file_error,
            })
        }
        event_id::FILE_LOADED => Some(MpvEventMsg::FileLoaded),
        _ => None,
    }
}

unsafe fn read_property(prop: &MpvEventProperty) -> MpvValue {
    if prop.data.is_null() || prop.format != format::NODE {
        return MpvValue::None;
    }
    let node = unsafe { &*(prop.data as *const MpvNode) };
    unsafe { read_node(node) }
}

unsafe fn read_node(node: &MpvNode) -> MpvValue {
    match node.format {
        format::STRING | format::OSD_STRING => {
            let ptr = unsafe { node.u.string };
            if ptr.is_null() {
                MpvValue::None
            } else {
                MpvValue::String(unsafe { CStr::from_ptr(ptr) }.to_string_lossy().into_owned())
            }
        }
        format::FLAG => MpvValue::Bool(unsafe { node.u.flag } != 0),
        format::INT64 => MpvValue::Int(unsafe { node.u.int64 }),
        format::DOUBLE => {
            let d = unsafe { node.u.double_ };
            if d.is_finite() {
                MpvValue::Double(d)
            } else {
                MpvValue::None
            }
        }
        format::NODE_ARRAY => {
            let list_ptr = unsafe { node.u.list };
            if list_ptr.is_null() {
                return MpvValue::Array(Vec::new());
            }
            let list = unsafe { &*list_ptr };
            let len = list.num.max(0) as usize;
            let mut out = Vec::with_capacity(len);
            if !list.values.is_null() {
                for i in 0..len {
                    out.push(unsafe { read_node(&*list.values.add(i)) });
                }
            }
            MpvValue::Array(out)
        }
        format::NODE_MAP => {
            let list_ptr = unsafe { node.u.list };
            if list_ptr.is_null() {
                return MpvValue::Map(BTreeMap::new());
            }
            let list = unsafe { &*list_ptr };
            let len = list.num.max(0) as usize;
            let mut map = BTreeMap::new();
            if !list.values.is_null() && !list.keys.is_null() {
                for i in 0..len {
                    let key_ptr = unsafe { *list.keys.add(i) };
                    let key = if key_ptr.is_null() {
                        String::new()
                    } else {
                        unsafe { CStr::from_ptr(key_ptr) }
                            .to_string_lossy()
                            .into_owned()
                    };
                    let val = unsafe { read_node(&*list.values.add(i)) };
                    map.insert(key, val);
                }
            }
            MpvValue::Map(map)
        }
        _ => MpvValue::None,
    }
}
