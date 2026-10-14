//! What the operating system says of the user's preferences (M20.3,
//! docs/design/media.md, "The user's preferences"), where the shell can
//! ask it: the colour scheme comes from winit's window theme, and this
//! module asks for the rest. Each platform's call is its own; where
//! there is none to make, the answer is `None`, and the shell's options
//! are the way to say it. And where the platform keeps an application's
//! data, for the profile (M28), and the key that keeps the profile's
//! (M33, D27). And the renderers' processes (M34): their launch, their
//! pipes, the job object that holds them and, on Windows, their sandbox
//! and the dumps of their crashes.
//!
//! This file is the workspace's one place that writes `unsafe` (M33.5):
//! its Windows modules allow it, each block with its `// SAFETY:`
//! argument above it.

use std::ffi::OsString;
use std::path::PathBuf;

/// The profile's directory when the command line names none (M28,
/// docs/design/storage.md): on Windows in the local application data,
/// `%LOCALAPPDATA%\SilverBrowser\Profile`; on macOS in
/// `~/Library/Application Support/SilverBrowser/Profile`; elsewhere in
/// `$XDG_DATA_HOME/silver-browser/profile`, `~/.local/share` for an
/// unset or relative `XDG_DATA_HOME`. `None` when the environment does
/// not say where.
pub(crate) fn profile_directory() -> Option<PathBuf> {
    profile_directory_in(|name| std::env::var_os(name))
}

/// [`profile_directory`] with the environment's variables read by
/// `var`.
fn profile_directory_in(var: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    let absolute = |name: &str| {
        var(name)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
    };
    #[cfg(windows)]
    {
        Some(
            absolute("LOCALAPPDATA")?
                .join("SilverBrowser")
                .join("Profile"),
        )
    }
    #[cfg(target_os = "macos")]
    {
        Some(
            absolute("HOME")?
                .join("Library/Application Support")
                .join("SilverBrowser/Profile"),
        )
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let data = match absolute("XDG_DATA_HOME") {
            Some(data) => data,
            None => absolute("HOME")?.join(".local/share"),
        };
        Some(data.join("silver-browser/profile"))
    }
}

/// What keeps the profile's data key (M33, D27): on Windows, DPAPI for
/// the user (`CryptProtectData`), which only the same user on the same
/// machine can undo; elsewhere the key is kept as it is, macOS's
/// Keychain and Linux's Secret Service not bridged (G98).
pub(crate) fn key_wrap() -> engine::KeyWrap {
    #[cfg(windows)]
    {
        engine::KeyWrap {
            name: "dpapi",
            wrap: windows::protect,
            unwrap: windows::unprotect,
        }
    }
    #[cfg(not(windows))]
    {
        engine::KeyWrap::NONE
    }
}

/// Whether the user asked the system for less motion: on Windows, the
/// client area animations turned off (`SPI_GETCLIENTAREAANIMATION`); on
/// macOS, the accessibility setting to reduce motion. `None` where the
/// shell cannot ask: on Linux the desktop portal that says it is a
/// D-Bus service the shell has no client for.
pub(crate) fn reduced_motion() -> Option<bool> {
    #[cfg(windows)]
    {
        windows::reduced_motion()
    }
    #[cfg(target_os = "macos")]
    {
        macos::reduced_motion()
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        None
    }
}

/// Starts renderers as processes of this executable, run with
/// `--renderer <number>` (M34.2, docs/design/processes.md): their
/// standard input and output are the channel, their standard error the
/// browser's log. On Windows each starts suspended, is put in a job
/// object that kills it when the browser's hold on the job closes, and is
/// resumed; it inherits its three pipes and nothing else. There it runs
/// sandboxed (M34.3, D2): with a restricted token of the browser's at
/// low integrity, in a job that admits no second process, limits its
/// memory to [`engine::service::MAX_RENDERER_MEMORY`] and closes the
/// desktop's global state to it. Elsewhere it is a child process with
/// piped streams, which ends when its input does, and has no sandbox
/// (G113). On Windows a renderer that crashes is dumped by the browser
/// into the crash folder, when the launcher has one (M34.4).
pub(crate) struct ProcessLauncher {
    executable: PathBuf,
    /// What follows the renderer's number on its command line: the
    /// sandbox's probes' (M34.3).
    arguments: Vec<OsString>,
    /// Where the dumps of crashed renderers go.
    crashes: Option<PathBuf>,
}

impl ProcessLauncher {
    /// A launcher of this very executable.
    pub(crate) fn new() -> std::io::Result<ProcessLauncher> {
        Ok(ProcessLauncher::of(std::env::current_exe()?))
    }

    /// A launcher of `executable`, the shell's.
    pub(crate) fn of(executable: PathBuf) -> ProcessLauncher {
        ProcessLauncher {
            executable,
            arguments: Vec::new(),
            crashes: None,
        }
    }

    /// The launcher, the dumps of its renderers' crashes going to
    /// `crashes`, if given.
    pub(crate) fn with_crashes(mut self, crashes: Option<PathBuf>) -> ProcessLauncher {
        self.crashes = crashes;
        self
    }

    /// The launcher, its renderers given `arguments` after their number.
    pub(crate) fn with_arguments(mut self, arguments: Vec<OsString>) -> ProcessLauncher {
        self.arguments = arguments;
        self
    }
}

/// Sets this renderer up to report its crash to the browser (M34.4)
/// through the channel whose three inherited handles the browser named
/// on its command line: the block it fills, the event it signals, and
/// the one it waits on while the browser writes the dump. An exception
/// no one handles is reported by a filter; a panic that ends the
/// renderer, by [`crashed_by_panic`], with the message the panic hook
/// kept.
pub(crate) fn report_crashes(handles: [usize; 3]) {
    #[cfg(windows)]
    launch::report_crashes(handles);
    #[cfg(not(windows))]
    let _ = handles;
}

/// Reports the panic that is ending this renderer, if it reports
/// crashes, and waits while the browser writes the dump.
pub(crate) fn crashed_by_panic() {
    #[cfg(windows)]
    launch::crashed_by_panic();
}

/// Faults, as a renderer's bug would: for the check of the dumps.
pub(crate) fn fault() {
    #[cfg(windows)]
    launch::fault();
    std::process::abort();
}

/// Whether the job this process runs in closes the desktop's global
/// state to it, as a renderer's does (M34.3): for the sandbox's probes.
pub(crate) fn desktop_closed() -> bool {
    #[cfg(windows)]
    {
        launch::desktop_closed()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

impl engine::service::Launcher for ProcessLauncher {
    fn launch(&self, number: u32) -> std::io::Result<engine::service::Launched> {
        #[cfg(windows)]
        {
            launch::launch(
                &self.executable,
                number,
                &self.arguments,
                self.crashes.as_deref(),
            )
        }
        #[cfg(not(windows))]
        {
            portable::launch(&self.executable, number, &self.arguments)
        }
    }
}

#[cfg(not(windows))]
mod portable {
    use std::ffi::OsString;
    use std::path::Path;
    use std::process::{Child, Command, Stdio};

    use engine::service::{Launched, RendererProcess};

    struct ChildRenderer(Child);

    impl RendererProcess for ChildRenderer {
        fn kill(&mut self) {
            let _ = self.0.kill();
        }

        fn id(&self) -> u32 {
            self.0.id()
        }

        fn exit_code(&mut self) -> Option<i64> {
            let status = self.0.try_wait().ok()??;
            Some(status.code().map_or(-1, i64::from))
        }
    }

    impl Drop for ChildRenderer {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    pub(super) fn launch(
        executable: &Path,
        number: u32,
        arguments: &[OsString],
    ) -> std::io::Result<Launched> {
        let mut child = Command::new(executable)
            .arg("--renderer")
            .arg(number.to_string())
            .args(arguments)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let missing = || std::io::Error::other("a pipe missing");
        let to_renderer = child.stdin.take().ok_or_else(missing)?;
        let from_renderer = child.stdout.take().ok_or_else(missing)?;
        let log = child.stderr.take().ok_or_else(missing)?;
        Ok(Launched {
            from_renderer: Box::new(from_renderer),
            to_renderer: Box::new(to_renderer),
            log: Some(Box::new(log)),
            process: Box::new(ChildRenderer(child)),
        })
    }
}

// The renderers' launch, sandbox and crashes, through `unsafe` (M34.2,
// M34.3, M34.4).
#[cfg(windows)]
#[allow(unsafe_code)]
mod launch {
    use std::ffi::{OsStr, OsString};
    use std::fs::File;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex, OnceLock, PoisonError};
    use std::time::{SystemTime, UNIX_EPOCH};

    use engine::service::{Launched, RendererProcess};
    use windows_sys::Win32::Foundation::{
        EXCEPTION_ACCESS_VIOLATION, GENERIC_ALL, HANDLE, HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE,
        SetHandleInformation, WAIT_OBJECT_0,
    };
    use windows_sys::Win32::Security::{
        ACCESS_ALLOWED_ACE, ACE_HEADER, ACL, ACL_SIZE_INFORMATION, AclSizeInformation,
        AddAccessAllowedAce, AddAce, CreateRestrictedToken, CreateWellKnownSid,
        DISABLE_MAX_PRIVILEGE, EqualSid, GetAce, GetAclInformation, GetLengthSid,
        GetTokenInformation, InitializeAcl, PSID, SECURITY_ATTRIBUTES, SECURITY_MAX_SID_SIZE,
        SID_AND_ATTRIBUTES, SetTokenInformation, TOKEN_ADJUST_DEFAULT, TOKEN_ASSIGN_PRIMARY,
        TOKEN_DEFAULT_DACL, TOKEN_DUPLICATE, TOKEN_GROUPS, TOKEN_MANDATORY_LABEL, TOKEN_QUERY,
        TokenDefaultDacl, TokenGroups, TokenIntegrityLevel, WELL_KNOWN_SID_TYPE,
        WinBuiltinUsersSid, WinLowLabelSid, WinRestrictedCodeSid, WinWorldSid,
    };
    use windows_sys::Win32::System::Diagnostics::Debug::{
        EXCEPTION_EXECUTE_HANDLER, EXCEPTION_POINTERS, MINIDUMP_EXCEPTION_INFORMATION,
        MiniDumpNormal, MiniDumpWriteDump, RaiseException, SetUnhandledExceptionFilter,
    };
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_ACTIVE_PROCESS,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOB_OBJECT_LIMIT_PROCESS_MEMORY,
        JOB_OBJECT_UILIMIT_DESKTOP, JOB_OBJECT_UILIMIT_DISPLAYSETTINGS,
        JOB_OBJECT_UILIMIT_EXITWINDOWS, JOB_OBJECT_UILIMIT_GLOBALATOMS, JOB_OBJECT_UILIMIT_HANDLES,
        JOB_OBJECT_UILIMIT_READCLIPBOARD, JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS,
        JOB_OBJECT_UILIMIT_WRITECLIPBOARD, JOBOBJECT_BASIC_UI_RESTRICTIONS,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectBasicUIRestrictions,
        JobObjectExtendedLimitInformation, QueryInformationJobObject, SetInformationJobObject,
    };
    use windows_sys::Win32::System::Memory::{
        CreateFileMappingW, FILE_MAP_READ, FILE_MAP_WRITE, MapViewOfFile, PAGE_READWRITE,
        UnmapViewOfFile,
    };
    use windows_sys::Win32::System::Pipes::CreatePipe;
    use windows_sys::Win32::System::Threading::{
        CREATE_NO_WINDOW, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, CreateEventW,
        CreateProcessAsUserW, DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT,
        GetCurrentProcess, GetCurrentThreadId, GetExitCodeProcess, INFINITE,
        InitializeProcThreadAttributeList, LPPROC_THREAD_ATTRIBUTE_LIST, OpenProcessToken,
        PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROCESS_INFORMATION, ResumeThread, STARTF_USESTDHANDLES,
        STARTUPINFOEXW, SetEvent, TerminateProcess, UpdateProcThreadAttribute,
        WaitForMultipleObjects, WaitForSingleObject,
    };

    /// A handle the call that made it returned, now ours alone; `None`
    /// for the null or invalid handle of a failure.
    fn owned(handle: HANDLE) -> Option<OwnedHandle> {
        if handle.is_null() || handle as isize == -1 {
            return None;
        }
        // SAFETY: `handle` was just returned open by the system to this
        // process, is neither null nor invalid, and nothing else holds or
        // closes it: the `OwnedHandle` is its one owner.
        Some(unsafe { OwnedHandle::from_raw_handle(handle as RawHandle) })
    }

    fn raw(handle: &OwnedHandle) -> HANDLE {
        handle.as_raw_handle() as HANDLE
    }

    fn last_error() -> std::io::Error {
        std::io::Error::last_os_error()
    }

    /// A pipe's reading and writing ends, neither inherited.
    fn pipe() -> std::io::Result<(OwnedHandle, OwnedHandle)> {
        let (mut read, mut write): (HANDLE, HANDLE) = (std::ptr::null_mut(), std::ptr::null_mut());
        let attributes = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: std::ptr::null_mut(),
            bInheritHandle: 0,
        };
        // SAFETY: the two out pointers are ours to write, the attributes
        // a whole struct of the size it says, alive for the call.
        let made = unsafe { CreatePipe(&mut read, &mut write, &attributes, 0) };
        if made == 0 {
            return Err(last_error());
        }
        let read = owned(read).ok_or_else(last_error)?;
        let write = owned(write).ok_or_else(last_error)?;
        Ok((read, write))
    }

    /// Lets `handle` be inherited by the next process made with
    /// inheritance, among the handles its list names.
    fn inheritable(handle: &OwnedHandle) -> std::io::Result<()> {
        inherited(handle, true)
    }

    /// Sets whether `handle` is inherited by processes made with
    /// inheritance.
    fn inherited(handle: &OwnedHandle, inherited: bool) -> std::io::Result<()> {
        let flag = if inherited { HANDLE_FLAG_INHERIT } else { 0 };
        // SAFETY: `handle` is open and ours for the call; the call changes
        // its inheritance flag and nothing else.
        let set = unsafe { SetHandleInformation(raw(handle), HANDLE_FLAG_INHERIT, flag) };
        if set == 0 {
            return Err(last_error());
        }
        Ok(())
    }

    /// What a renderer that crashes fills, in memory it shares with the
    /// browser (M34.4): what crashed it, its thread, the exception's
    /// pointers in its own memory, and a panic's message.
    #[repr(C)]
    struct Block {
        kind: u32,
        thread: u32,
        pointers: u64,
        length: u32,
        message: [u8; MESSAGE],
    }

    /// The longest panic message a block holds.
    const MESSAGE: usize = 4072;

    /// What crashed a renderer: an exception no one handled, or a panic.
    const EXCEPTION: u32 = 1;
    const PANIC: u32 = 2;

    /// The browser's ends of a renderer's crash channel: the block, the
    /// event the renderer signals when it crashes, and the one the browser
    /// signals once the dump is written.
    struct Channel {
        block: OwnedHandle,
        crashed: OwnedHandle,
        written: OwnedHandle,
    }

    fn channel() -> std::io::Result<Channel> {
        // SAFETY: no file: memory backed by the paging file, of a block's
        // size, unnamed, with the default security; its handle returned.
        let block = owned(unsafe {
            CreateFileMappingW(
                INVALID_HANDLE_VALUE,
                std::ptr::null(),
                PAGE_READWRITE,
                0,
                size_of::<Block>() as u32,
                std::ptr::null(),
            )
        })
        .ok_or_else(last_error)?;
        // SAFETY: auto-reset events, unsignalled, unnamed, with the
        // default security; their handles returned.
        let event = || owned(unsafe { CreateEventW(std::ptr::null(), 0, 0, std::ptr::null()) });
        let crashed = event().ok_or_else(last_error)?;
        let written = event().ok_or_else(last_error)?;
        Ok(Channel {
            block,
            crashed,
            written,
        })
    }

    /// Waits for renderer `id` to crash or end; when it crashes, writes
    /// its dump into `dir`, once, notes where in `dump`, and lets it end,
    /// ending it if it goes on.
    fn watch(
        channel: Channel,
        process: &OwnedHandle,
        id: u32,
        dir: &Path,
        dump: &Mutex<Option<PathBuf>>,
    ) {
        let handles = [raw(&channel.crashed), raw(process)];
        // SAFETY: both handles are open and ours for the wait.
        let woke = unsafe { WaitForMultipleObjects(2, handles.as_ptr(), 0, INFINITE) };
        if woke != WAIT_OBJECT_0 {
            return;
        }
        let written = write_dump(&channel, process, id, dir);
        *dump.lock().unwrap_or_else(PoisonError::into_inner) = written;
        // SAFETY: the event is open and ours.
        unsafe { SetEvent(raw(&channel.written)) };
        // SAFETY: the process handle is open and ours for the wait.
        if unsafe { WaitForSingleObject(raw(process), 10_000) } != WAIT_OBJECT_0 {
            // SAFETY: the process handle is open, ours, with every right.
            unsafe { TerminateProcess(raw(process), 1) };
        }
    }

    /// Writes renderer `id`'s dump into `dir`, with the text file of its
    /// panic's message beside it, and keeps the newest dumps; the dump's
    /// file, if one was written.
    fn write_dump(
        channel: &Channel,
        process: &OwnedHandle,
        id: u32,
        dir: &Path,
    ) -> Option<PathBuf> {
        // SAFETY: the mapping is open and ours, a block's size; the view
        // is read once below and unmapped.
        let view =
            unsafe { MapViewOfFile(raw(&channel.block), FILE_MAP_READ, 0, 0, size_of::<Block>()) };
        if view.Value.is_null() {
            return None;
        }
        // SAFETY: the view is a whole block, page-aligned; the renderer
        // filled it before it signalled and waits while it is read; any
        // bytes are a block, and a copy is read.
        let block = unsafe { view.Value.cast::<Block>().read() };
        // SAFETY: the view was mapped above and is unmapped once.
        unsafe { UnmapViewOfFile(view) };
        std::fs::create_dir_all(dir).ok()?;
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_millis());
        let path = dir.join(crate::crashes::dump_name(millis, id));
        let file = File::create(&path).ok()?;
        let exception = MINIDUMP_EXCEPTION_INFORMATION {
            ThreadId: block.thread,
            ExceptionPointers: block.pointers as usize as *mut EXCEPTION_POINTERS,
            ClientPointers: 1,
        };
        let write = |exception: *const MINIDUMP_EXCEPTION_INFORMATION| {
            // SAFETY: the process handle is open with every right, the
            // file's open for writing, both for the call; the exception's
            // information, when given, is ours and alive for the call,
            // the pointers in it the renderer's, which the call reads in
            // its memory and fails on if they lead nowhere.
            unsafe {
                MiniDumpWriteDump(
                    raw(process),
                    id,
                    file.as_raw_handle() as HANDLE,
                    MiniDumpNormal,
                    exception,
                    std::ptr::null(),
                    std::ptr::null(),
                )
            }
        };
        // The renderer's word on the exception is not trusted: without it
        // the dump still has every thread.
        let written =
            (block.kind == EXCEPTION && block.pointers != 0 && write(&raw const exception) != 0)
                || write(std::ptr::null()) != 0;
        drop(file);
        if !written {
            let _ = std::fs::remove_file(&path);
            return None;
        }
        let length = (block.length as usize).min(MESSAGE);
        if block.kind == PANIC && length > 0 {
            let _ = std::fs::write(
                path.with_extension("txt"),
                crate::crashes::message_text(&block.message[..length]),
            );
        }
        crate::crashes::prune(dir, crate::crashes::MAX_CRASH_DUMPS);
        Some(path)
    }

    /// The renderer's side of its crash channel: the block's address in
    /// it, and the two events' handles.
    struct Reporting {
        block: usize,
        crashed: usize,
        written: usize,
    }

    static REPORTING: OnceLock<Reporting> = OnceLock::new();

    /// Whether a crash was reported: one is, the first.
    static REPORTED: AtomicBool = AtomicBool::new(false);

    /// The message of the last panic, kept by the panic hook.
    static LAST_PANIC: Mutex<Option<String>> = Mutex::new(None);

    pub(super) fn report_crashes([block, crashed, written]: [usize; 3]) {
        // SAFETY: the value is the handle of the mapping the browser made
        // inheritable for this process, a block's size; a value that is no
        // such handle maps nothing, and nothing is reported.
        let view =
            unsafe { MapViewOfFile(block as HANDLE, FILE_MAP_WRITE, 0, 0, size_of::<Block>()) };
        if view.Value.is_null() {
            return;
        }
        let _ = REPORTING.set(Reporting {
            block: view.Value as usize,
            crashed,
            written,
        });
        // SAFETY: the filter is a function of the type the call takes,
        // alive as long as the process.
        unsafe { SetUnhandledExceptionFilter(Some(filter)) };
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let backtrace = std::backtrace::Backtrace::force_capture();
            *LAST_PANIC.lock().unwrap_or_else(PoisonError::into_inner) =
                Some(format!("{info}\n{backtrace}"));
            previous(info);
        }));
    }

    /// Fills the block with what crashed the renderer, of `kind`, signals
    /// the browser and waits while it writes the dump: once, the first
    /// crash's.
    fn report(kind: u32, pointers: u64, message: &[u8]) {
        let Some(reporting) = REPORTING.get() else {
            return;
        };
        if REPORTED.swap(true, Ordering::SeqCst) {
            return;
        }
        let block = reporting.block as *mut Block;
        let length = message.len().min(MESSAGE);
        // SAFETY: the block is the view mapped at the renderer's start,
        // a whole block, mapped as long as the process; this thread alone
        // writes it, once, before it signals; the browser reads it after.
        // The events are the browser's, inherited, open as long as the
        // process.
        unsafe {
            (&raw mut (*block).kind).write(kind);
            (&raw mut (*block).thread).write(GetCurrentThreadId());
            (&raw mut (*block).pointers).write(pointers);
            (&raw mut (*block).length).write(length as u32);
            (&raw mut (*block).message)
                .cast::<u8>()
                .copy_from_nonoverlapping(message.as_ptr(), length);
            SetEvent(reporting.crashed as HANDLE);
            WaitForSingleObject(reporting.written as HANDLE, 30_000);
        }
    }

    /// The renderer's filter of exceptions no one handled: the browser
    /// dumps it, then it ends.
    extern "system" fn filter(pointers: *const EXCEPTION_POINTERS) -> i32 {
        report(EXCEPTION, pointers as u64, &[]);
        EXCEPTION_EXECUTE_HANDLER
    }

    pub(super) fn crashed_by_panic() {
        let message = LAST_PANIC
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
            .unwrap_or_default();
        report(PANIC, 0, message.as_bytes());
    }

    pub(super) fn fault() {
        // SAFETY: raises an access violation with no arguments, as a bad
        // read would; the filter takes it, and the process ends.
        unsafe { RaiseException(EXCEPTION_ACCESS_VIOLATION as u32, 0, 0, std::ptr::null()) };
    }

    /// A job object that kills its processes when its last handle
    /// closes, admits one active process (so no child of the renderer's),
    /// limits a process's committed memory to `MAX_RENDERER_MEMORY`, and
    /// forbids the desktop's global state: the system's parameters, the
    /// display settings, exiting Windows, the global atoms, user objects'
    /// handles from outside the job, the clipboard and other desktops.
    fn job() -> std::io::Result<OwnedHandle> {
        // SAFETY: no attributes and no name: a new unnamed job with the
        // default security, its handle returned.
        let job = owned(unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) })
            .ok_or_else(last_error)?;
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
            | JOB_OBJECT_LIMIT_ACTIVE_PROCESS
            | JOB_OBJECT_LIMIT_PROCESS_MEMORY;
        limits.BasicLimitInformation.ActiveProcessLimit = 1;
        limits.ProcessMemoryLimit =
            usize::try_from(engine::service::MAX_RENDERER_MEMORY).unwrap_or(usize::MAX);
        // SAFETY: `limits` is the whole extended limit structure the class
        // names, of the size given, alive for the call, which reads it.
        let set = unsafe {
            SetInformationJobObject(
                raw(&job),
                JobObjectExtendedLimitInformation,
                (&raw const limits).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if set == 0 {
            return Err(last_error());
        }
        let interface = JOBOBJECT_BASIC_UI_RESTRICTIONS {
            UIRestrictionsClass: CLOSED,
        };
        // SAFETY: `interface` is the whole structure the class names, of
        // the size given, alive for the call, which reads it.
        let set = unsafe {
            SetInformationJobObject(
                raw(&job),
                JobObjectBasicUIRestrictions,
                (&raw const interface).cast(),
                size_of::<JOBOBJECT_BASIC_UI_RESTRICTIONS>() as u32,
            )
        };
        if set == 0 {
            return Err(last_error());
        }
        Ok(job)
    }

    /// What of the desktop's global state a renderer's job closes.
    const CLOSED: u32 = JOB_OBJECT_UILIMIT_DESKTOP
        | JOB_OBJECT_UILIMIT_DISPLAYSETTINGS
        | JOB_OBJECT_UILIMIT_EXITWINDOWS
        | JOB_OBJECT_UILIMIT_GLOBALATOMS
        | JOB_OBJECT_UILIMIT_HANDLES
        | JOB_OBJECT_UILIMIT_READCLIPBOARD
        | JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS
        | JOB_OBJECT_UILIMIT_WRITECLIPBOARD;

    pub(super) fn desktop_closed() -> bool {
        let mut interface = JOBOBJECT_BASIC_UI_RESTRICTIONS::default();
        // SAFETY: a null job is the one this process runs in; the
        // structure is the whole one the class names, of the size given,
        // ours to write; no length is asked back.
        let read = unsafe {
            QueryInformationJobObject(
                std::ptr::null_mut(),
                JobObjectBasicUIRestrictions,
                (&raw mut interface).cast(),
                size_of::<JOBOBJECT_BASIC_UI_RESTRICTIONS>() as u32,
                std::ptr::null_mut(),
            )
        };
        read != 0 && interface.UIRestrictionsClass & CLOSED == CLOSED
    }

    /// Of a token's groups' attributes: `SE_GROUP_INTEGRITY`, the
    /// integrity label's, and `SE_GROUP_LOGON_ID`, the logon session's.
    const GROUP_INTEGRITY: u32 = 0x0000_0020;
    const GROUP_LOGON_ID: u32 = 0xC000_0000;

    /// A well-known SID of `kind`, in a buffer of words.
    fn well_known(kind: WELL_KNOWN_SID_TYPE) -> std::io::Result<Vec<usize>> {
        let mut size = SECURITY_MAX_SID_SIZE;
        let mut sid = vec![0usize; (SECURITY_MAX_SID_SIZE as usize).div_ceil(size_of::<usize>())];
        // SAFETY: the buffer is at least `size` bytes and ours to write;
        // the kinds asked for need no domain.
        let made = unsafe {
            CreateWellKnownSid(
                kind,
                std::ptr::null_mut(),
                sid.as_mut_ptr().cast(),
                &mut size,
            )
        };
        if made == 0 {
            return Err(last_error());
        }
        Ok(sid)
    }

    /// `token`'s information of class `class`, in a buffer of words.
    fn information(
        token: &OwnedHandle,
        class: windows_sys::Win32::Security::TOKEN_INFORMATION_CLASS,
    ) -> std::io::Result<Vec<usize>> {
        let mut size = 0u32;
        // SAFETY: no buffer and no length ask for the size the class
        // needs, written to `size`; the call fails by design, with it.
        unsafe { GetTokenInformation(raw(token), class, std::ptr::null_mut(), 0, &mut size) };
        let mut buffer = vec![0usize; (size as usize).div_ceil(size_of::<usize>()).max(1)];
        // SAFETY: the buffer is at least `size` bytes, aligned for the
        // structure, and ours to write; `size` too.
        let read = unsafe {
            GetTokenInformation(
                raw(token),
                class,
                buffer.as_mut_ptr().cast(),
                size,
                &mut size,
            )
        };
        if read == 0 {
            return Err(last_error());
        }
        Ok(buffer)
    }

    /// Adds an entry granting `sid` all access to `token`'s default DACL,
    /// the one given to what a process of the token makes for itself
    /// (its threads, its sections, its events): without it, what the
    /// process makes would grant only the user, whom the restricted
    /// check does not admit, and the system's libraries fail to start.
    fn admit_by_default(token: &OwnedHandle, sid: PSID) -> std::io::Result<()> {
        let buffer = information(token, TokenDefaultDacl)?;
        // SAFETY: the buffer holds the token's default DACL as the call
        // wrote it: the structure, its pointer into the buffer or null.
        let old = unsafe { (*buffer.as_ptr().cast::<TOKEN_DEFAULT_DACL>()).DefaultDacl };
        let mut sizes = ACL_SIZE_INFORMATION::default();
        if !old.is_null() {
            // SAFETY: `old` is the token's ACL, in the buffer, alive; the
            // structure is the class's, of the size given, ours to write.
            let read = unsafe {
                GetAclInformation(
                    old,
                    (&raw mut sizes).cast(),
                    size_of::<ACL_SIZE_INFORMATION>() as u32,
                    AclSizeInformation,
                )
            };
            if read == 0 {
                return Err(last_error());
            }
        }
        // SAFETY: `sid` is a valid SID made by the caller.
        let length = size_of::<ACL>() as u32
            + sizes.AclBytesInUse
            + size_of::<ACCESS_ALLOWED_ACE>() as u32
            + unsafe { GetLengthSid(sid) };
        let mut acl = vec![0usize; (length as usize).div_ceil(size_of::<usize>())];
        let new = acl.as_mut_ptr().cast::<ACL>();
        let revision = if old.is_null() {
            windows_sys::Win32::Security::ACL_REVISION
        } else {
            // SAFETY: `old` is the token's ACL, alive in the buffer.
            u32::from(unsafe { (*old).AclRevision })
        };
        // SAFETY: the buffer is at least `length` bytes, aligned, ours.
        if unsafe { InitializeAcl(new, length, revision) } == 0 {
            return Err(last_error());
        }
        for index in 0..sizes.AceCount {
            let mut entry = std::ptr::null_mut();
            // SAFETY: `old` is a valid ACL of more than `index` entries;
            // `entry` is ours to write, then points into `old`, whose
            // entries are copied, each of the size its header says, into
            // the new ACL, which has room for all of them.
            let copied = unsafe {
                GetAce(old, index, &mut entry) != 0
                    && AddAce(
                        new,
                        revision,
                        u32::MAX,
                        entry,
                        u32::from((*entry.cast::<ACE_HEADER>()).AceSize),
                    ) != 0
            };
            if !copied {
                return Err(last_error());
            }
        }
        // SAFETY: the new ACL has room for the entry; `sid` is valid.
        if unsafe { AddAccessAllowedAce(new, revision, GENERIC_ALL, sid) } == 0 {
            return Err(last_error());
        }
        let default = TOKEN_DEFAULT_DACL { DefaultDacl: new };
        // SAFETY: the token is open with the right to change its default;
        // the structure and the ACL it points at are alive for the call,
        // which copies them.
        let set = unsafe {
            SetTokenInformation(
                raw(token),
                TokenDefaultDacl,
                (&raw const default).cast(),
                size_of::<TOKEN_DEFAULT_DACL>() as u32,
            )
        };
        if set == 0 {
            return Err(last_error());
        }
        Ok(())
    }

    /// The token a renderer runs with (M34.3, D2): the browser's own,
    /// restricted. Every privilege is removed but the one to traverse
    /// folders; every group is made deny-only but `Everyone`, `Users` and
    /// the logon session's; and `Users`, `Everyone` and `RESTRICTED` are
    /// its restricting SIDs, so that an access must be granted to one of
    /// them as well as to the user. The logon session's SID is not one:
    /// it would admit the session's desktop and named objects, which a
    /// renderer needs no more of than those grant. Its integrity is low:
    /// it can write nothing labelled above it.
    fn renderer_token() -> std::io::Result<OwnedHandle> {
        let mut own: HANDLE = std::ptr::null_mut();
        // SAFETY: the current process's pseudo-handle needs no closing;
        // `own` is ours to write.
        let opened = unsafe {
            OpenProcessToken(
                GetCurrentProcess(),
                TOKEN_DUPLICATE | TOKEN_QUERY | TOKEN_ASSIGN_PRIMARY | TOKEN_ADJUST_DEFAULT,
                &mut own,
            )
        };
        if opened == 0 {
            return Err(last_error());
        }
        let own = owned(own).ok_or_else(last_error)?;
        // The SIDs live in `sids`, to the end of the function.
        let mut sids = [
            well_known(WinBuiltinUsersSid)?,
            well_known(WinWorldSid)?,
            well_known(WinRestrictedCodeSid)?,
            well_known(WinLowLabelSid)?,
        ];
        let [users, everyone, restricted, low]: [PSID; 4] =
            sids.each_mut().map(|sid| sid.as_mut_ptr().cast());
        let buffer = information(&own, TokenGroups)?;
        let list = buffer.as_ptr().cast::<TOKEN_GROUPS>();
        // SAFETY: the buffer holds the token's groups as the call wrote
        // them: a count, then that many entries, whose SIDs point into the
        // buffer, which outlives every use of them below.
        let groups: &[SID_AND_ATTRIBUTES] = unsafe {
            std::slice::from_raw_parts(
                (&raw const (*list).Groups).cast(),
                (*list).GroupCount as usize,
            )
        };
        let mut deny = Vec::new();
        for group in groups {
            if group.Attributes & GROUP_INTEGRITY != 0 {
                continue;
            }
            if group.Attributes & GROUP_LOGON_ID == GROUP_LOGON_ID {
                continue;
            }
            // SAFETY: each SID is valid and alive: the token's, or one
            // made above.
            let kept =
                unsafe { EqualSid(group.Sid, users) != 0 || EqualSid(group.Sid, everyone) != 0 };
            if !kept {
                deny.push(SID_AND_ATTRIBUTES {
                    Sid: group.Sid,
                    Attributes: 0,
                });
            }
        }
        let restricting: Vec<SID_AND_ATTRIBUTES> = [users, everyone, restricted]
            .into_iter()
            .map(|sid| SID_AND_ATTRIBUTES {
                Sid: sid,
                Attributes: 0,
            })
            .collect();
        let mut token: HANDLE = std::ptr::null_mut();
        // SAFETY: the browser's token is open with the rights the call
        // needs; both lists are whole, of the lengths given, their SIDs
        // valid and alive for the call; `token` is ours to write.
        let made = unsafe {
            CreateRestrictedToken(
                raw(&own),
                DISABLE_MAX_PRIVILEGE,
                deny.len() as u32,
                deny.as_ptr(),
                0,
                std::ptr::null(),
                restricting.len() as u32,
                restricting.as_ptr(),
                &mut token,
            )
        };
        if made == 0 {
            return Err(last_error());
        }
        let token = owned(token).ok_or_else(last_error)?;
        admit_by_default(&token, restricted)?;
        let label = TOKEN_MANDATORY_LABEL {
            Label: SID_AND_ATTRIBUTES {
                Sid: low,
                Attributes: GROUP_INTEGRITY,
            },
        };
        // SAFETY: the new token is open with the right to change its
        // default, which covers its integrity; the label and its SID are
        // valid and alive for the call, the length counting both.
        let set = unsafe {
            SetTokenInformation(
                raw(&token),
                TokenIntegrityLevel,
                (&raw const label).cast(),
                size_of::<TOKEN_MANDATORY_LABEL>() as u32 + GetLengthSid(low),
            )
        };
        if set == 0 {
            return Err(last_error());
        }
        Ok(token)
    }

    fn wide(text: &OsStr) -> Vec<u16> {
        text.encode_wide().chain(Some(0)).collect()
    }

    /// A renderer's process and the job that holds it: dropping the job's
    /// handle kills it. And its dump, once the browser wrote one.
    struct JobRenderer {
        process: Arc<OwnedHandle>,
        job: OwnedHandle,
        id: u32,
        dump: Arc<Mutex<Option<PathBuf>>>,
    }

    impl RendererProcess for JobRenderer {
        fn kill(&mut self) {
            // SAFETY: the process handle is open, ours, with every right
            // its creation gave; ending an ended process fails harmlessly.
            unsafe { TerminateProcess(raw(&self.process), 1) };
        }

        fn id(&self) -> u32 {
            self.id
        }

        fn exit_code(&mut self) -> Option<i64> {
            // SAFETY: the process handle is open and ours; a wait of no
            // time only reads whether it is signalled.
            if unsafe { WaitForSingleObject(raw(&self.process), 0) } != WAIT_OBJECT_0 {
                return None;
            }
            let mut code = 0u32;
            // SAFETY: the handle is open and ours, `code` ours to write.
            let read = unsafe { GetExitCodeProcess(raw(&self.process), &mut code) };
            (read != 0).then_some(i64::from(code))
        }

        fn dump(&mut self) -> Option<PathBuf> {
            self.dump
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
        }
    }

    /// An attribute list naming the only handles a process inherits;
    /// deleted when dropped.
    struct HandleList {
        /// Words, so that the list is aligned for the pointers in it.
        buffer: Vec<usize>,
        /// Boxed, so that they do not move while the list points at them.
        handles: Box<[HANDLE]>,
        made: bool,
    }

    impl HandleList {
        fn new(handles: Vec<HANDLE>) -> std::io::Result<HandleList> {
            let mut size = 0usize;
            // SAFETY: a null list asks for the size one attribute needs,
            // written to `size`; the call fails by design, with the size.
            unsafe { InitializeProcThreadAttributeList(std::ptr::null_mut(), 1, 0, &mut size) };
            let mut list = HandleList {
                buffer: vec![0; size.div_ceil(size_of::<usize>()).max(1)],
                handles: handles.into_boxed_slice(),
                made: false,
            };
            // SAFETY: the buffer is at least `size` bytes, aligned for a
            // pointer, ours, and lives as long as the list.
            let made = unsafe { InitializeProcThreadAttributeList(list.list(), 1, 0, &mut size) };
            if made == 0 {
                return Err(last_error());
            }
            list.made = true;
            let value = list.handles.as_ptr();
            // SAFETY: the list was made above; the value is the boxed slice
            // of handles, of the size given, which lives (and does not
            // move) as long as the list, as the attribute requires.
            let updated = unsafe {
                UpdateProcThreadAttribute(
                    list.list(),
                    0,
                    PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                    value.cast(),
                    size_of_val(&*list.handles),
                    std::ptr::null_mut(),
                    std::ptr::null(),
                )
            };
            if updated == 0 {
                return Err(last_error());
            }
            Ok(list)
        }

        fn list(&mut self) -> LPPROC_THREAD_ATTRIBUTE_LIST {
            self.buffer.as_mut_ptr().cast()
        }
    }

    impl Drop for HandleList {
        fn drop(&mut self) {
            if self.made {
                // SAFETY: the list was initialized and is deleted once,
                // here, with nothing using it after.
                unsafe { DeleteProcThreadAttributeList(self.list()) };
            }
        }
    }

    pub(super) fn launch(
        executable: &Path,
        number: u32,
        arguments: &[OsString],
        crashes: Option<&Path>,
    ) -> std::io::Result<Launched> {
        let token = renderer_token()?;
        let (child_input, to_renderer) = pipe()?;
        let (from_renderer, child_output) = pipe()?;
        let (log, child_log) = pipe()?;
        for end in [&child_input, &child_output, &child_log] {
            inheritable(end)?;
        }
        let mut inherit = vec![raw(&child_input), raw(&child_output), raw(&child_log)];
        let channel = match crashes {
            Some(_) => Some(channel()?),
            None => None,
        };
        let job = job()?;
        let application = wide(executable.as_os_str());
        let mut line = format!("\"{}\" --renderer {number}", executable.display());
        if let Some(channel) = &channel {
            for end in [&channel.block, &channel.crashed, &channel.written] {
                inheritable(end)?;
                inherit.push(raw(end));
            }
            line.push_str(&format!(
                " --crash {} {} {}",
                raw(&channel.block) as usize,
                raw(&channel.crashed) as usize,
                raw(&channel.written) as usize
            ));
        }
        for argument in arguments {
            let quotable = argument
                .to_str()
                .filter(|text| !text.contains('"') && !text.ends_with('\\'))
                .ok_or_else(|| std::io::Error::other("an argument the line cannot quote"))?;
            line.push_str(&format!(" \"{quotable}\""));
        }
        let mut command = wide(OsStr::new(&line));
        let mut list = HandleList::new(inherit)?;
        let mut startup = STARTUPINFOEXW::default();
        startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
        startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        startup.StartupInfo.hStdInput = raw(&child_input);
        startup.StartupInfo.hStdOutput = raw(&child_output);
        startup.StartupInfo.hStdError = raw(&child_log);
        startup.lpAttributeList = list.list();
        let mut made = PROCESS_INFORMATION::default();
        // SAFETY: the token is the restricted one made above, open with
        // the rights a primary token needs; the application name and the
        // command line are NUL-terminated wide strings we own, the command
        // line mutable as the call requires; no process or thread
        // attributes, the environment and directory ours; the startup
        // structure is the extended one its flag says, its list naming the
        // three inheritable handles above, all alive for the call; `made`
        // is ours to fill.
        let created = unsafe {
            CreateProcessAsUserW(
                raw(&token),
                application.as_ptr(),
                command.as_mut_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                1,
                CREATE_SUSPENDED
                    | EXTENDED_STARTUPINFO_PRESENT
                    | CREATE_NO_WINDOW
                    | CREATE_UNICODE_ENVIRONMENT,
                std::ptr::null(),
                std::ptr::null(),
                &startup.StartupInfo,
                &mut made,
            )
        };
        drop(list);
        if created == 0 {
            return Err(last_error());
        }
        let process = Arc::new(owned(made.hProcess).ok_or_else(last_error)?);
        let thread = owned(made.hThread).ok_or_else(last_error)?;
        let mut renderer = JobRenderer {
            process,
            job,
            id: made.dwProcessId,
            dump: Arc::default(),
        };
        // SAFETY: both handles are open and ours; the process has not run
        // an instruction yet, being suspended.
        let assigned =
            unsafe { AssignProcessToJobObject(raw(&renderer.job), raw(&renderer.process)) };
        if assigned == 0 {
            let error = last_error();
            renderer.kill();
            return Err(error);
        }
        // The browser's ends of the crash channel are its own again, and a
        // thread waits for the renderer to crash.
        if let (Some(channel), Some(dir)) = (channel, crashes) {
            for end in [&channel.block, &channel.crashed, &channel.written] {
                inherited(end, false)?;
            }
            let (process, dump) = (renderer.process.clone(), renderer.dump.clone());
            let (dir, id) = (dir.to_path_buf(), renderer.id);
            let watching = std::thread::Builder::new()
                .name(format!("renderer {number}'s crash"))
                .spawn(move || watch(channel, &process, id, &dir, &dump));
            if let Err(error) = watching {
                renderer.kill();
                return Err(error);
            }
        }
        // SAFETY: the thread handle is the suspended main thread's, open
        // and ours; it is resumed once.
        if unsafe { ResumeThread(raw(&thread)) } == u32::MAX {
            let error = last_error();
            renderer.kill();
            return Err(error);
        }
        // The renderer's ends are its own now: the browser's copies close.
        drop((child_input, child_output, child_log, thread));
        Ok(Launched {
            from_renderer: Box::new(File::from(from_renderer)),
            to_renderer: Box::new(File::from(to_renderer)),
            log: Some(Box::new(File::from(log))),
            process: Box::new(renderer),
        })
    }
}

// DPAPI and the system's parameters, through `unsafe` (M33.5).
#[cfg(windows)]
#[allow(unsafe_code)]
mod windows {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SPI_GETCLIENTAREAANIMATION, SystemParametersInfoW,
    };
    use windows_sys::core::BOOL;

    /// `data` wrapped by DPAPI for the user, with no prompt; `None` when
    /// the system refuses.
    pub(super) fn protect(data: &[u8]) -> Option<Vec<u8>> {
        crypt(data, true)
    }

    /// What `protect` wrapped, unwrapped; `None` for what this user on
    /// this machine did not wrap, or that is not DPAPI's.
    pub(super) fn unprotect(data: &[u8]) -> Option<Vec<u8>> {
        crypt(data, false)
    }

    fn crypt(data: &[u8], protect: bool) -> Option<Vec<u8>> {
        let input = CRYPT_INTEGER_BLOB {
            cbData: u32::try_from(data.len()).ok()?,
            pbData: data.as_ptr().cast_mut(),
        };
        let mut output = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: std::ptr::null_mut(),
        };
        let none = std::ptr::null::<core::ffi::c_void>();
        // SAFETY: `input` points at `data`'s `cbData` bytes, alive for the
        // call, which only reads them; the description, entropy, reserved
        // and prompt pointers are null, which both calls take; `output`
        // is ours, for the call to fill.
        let done = unsafe {
            if protect {
                CryptProtectData(
                    &input,
                    none.cast(),
                    none.cast(),
                    none,
                    none.cast(),
                    CRYPTPROTECT_UI_FORBIDDEN,
                    &mut output,
                )
            } else {
                CryptUnprotectData(
                    &input,
                    std::ptr::null_mut(),
                    none.cast(),
                    none,
                    none.cast(),
                    CRYPTPROTECT_UI_FORBIDDEN,
                    &mut output,
                )
            }
        };
        if done == 0 || output.pbData.is_null() {
            return None;
        }
        // SAFETY: on success `pbData` holds the `cbData` bytes the call
        // wrote, allocated by it with `LocalAlloc`, alive until freed.
        let bytes =
            unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize) }.to_vec();
        // SAFETY: `pbData` is the call's `LocalAlloc` allocation, freed
        // once, here, and not used after.
        unsafe { LocalFree(output.pbData.cast()) };
        Some(bytes)
    }

    pub(super) fn reduced_motion() -> Option<bool> {
        let mut animations: BOOL = 1;
        // SAFETY: `SPI_GETCLIENTAREAANIMATION` takes a pointer to one
        // `BOOL`, which it writes and nothing else; `animations` is one,
        // alive and unaliased for the call.
        let read = unsafe {
            SystemParametersInfoW(
                SPI_GETCLIENTAREAANIMATION,
                0,
                (&raw mut animations).cast(),
                0,
            )
        };
        (read != 0).then_some(animations == 0)
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use objc2_app_kit::NSWorkspace;

    pub(super) fn reduced_motion() -> Option<bool> {
        let workspace = NSWorkspace::sharedWorkspace();
        Some(workspace.accessibilityDisplayShouldReduceMotion())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn dpapi_wraps_what_only_it_unwraps() {
        let key = [7u8; 32];
        let wrap = key_wrap();
        assert_eq!(wrap.name, "dpapi");
        let wrapped = (wrap.wrap)(&key).expect("DPAPI wraps it");
        assert_ne!(wrapped.as_slice(), key.as_slice());
        assert_eq!((wrap.unwrap)(&wrapped).as_deref(), Some(key.as_slice()));
        assert_eq!((wrap.unwrap)(&key), None, "not DPAPI's");
        let mut damaged = wrapped.clone();
        let last = damaged.len() - 1;
        damaged[last] ^= 1;
        assert_eq!((wrap.unwrap)(&damaged), None);
    }

    /// An environment of the variables `pairs` gives.
    fn env(pairs: &'static [(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| OsString::from(value))
        }
    }

    #[test]
    fn the_profile_lives_where_the_platform_keeps_an_applications_data() {
        assert_eq!(profile_directory_in(env(&[])), None);
        #[cfg(windows)]
        {
            assert_eq!(
                profile_directory_in(env(&[("LOCALAPPDATA", r"C:\Users\u\AppData\Local")])),
                Some(PathBuf::from(
                    r"C:\Users\u\AppData\Local\SilverBrowser\Profile"
                ))
            );
            assert_eq!(
                profile_directory_in(env(&[("LOCALAPPDATA", "relative")])),
                None
            );
        }
        #[cfg(target_os = "macos")]
        {
            assert_eq!(
                profile_directory_in(env(&[("HOME", "/Users/u")])),
                Some(PathBuf::from(
                    "/Users/u/Library/Application Support/SilverBrowser/Profile"
                ))
            );
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        {
            assert_eq!(
                profile_directory_in(env(&[("HOME", "/home/u")])),
                Some(PathBuf::from("/home/u/.local/share/silver-browser/profile"))
            );
            assert_eq!(
                profile_directory_in(env(&[("HOME", "/home/u"), ("XDG_DATA_HOME", "/data")])),
                Some(PathBuf::from("/data/silver-browser/profile"))
            );
            assert_eq!(
                profile_directory_in(env(&[("HOME", "/home/u"), ("XDG_DATA_HOME", "rel")])),
                Some(PathBuf::from("/home/u/.local/share/silver-browser/profile"))
            );
        }
    }
}
