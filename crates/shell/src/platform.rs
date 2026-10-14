//! What the operating system says of the user's preferences (M20.3,
//! docs/design/media.md, "The user's preferences"), where the shell can
//! ask it: the colour scheme comes from winit's window theme, and this
//! module asks for the rest. Each platform's call is its own; where
//! there is none to make, the answer is `None`, and the shell's options
//! are the way to say it. And where the platform keeps an application's
//! data, for the profile (M28), and the key that keeps the profile's
//! (M33, D27). And the renderers' processes (M34): their launch, their
//! pipes and the job object that holds them.
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
/// resumed; it inherits its three pipes and nothing else. Elsewhere it is
/// a child process with piped streams, which ends when its input does.
pub(crate) struct ProcessLauncher {
    executable: PathBuf,
}

impl ProcessLauncher {
    /// A launcher of this very executable.
    pub(crate) fn new() -> std::io::Result<ProcessLauncher> {
        Ok(ProcessLauncher::of(std::env::current_exe()?))
    }

    /// A launcher of `executable`, the shell's.
    pub(crate) fn of(executable: PathBuf) -> ProcessLauncher {
        ProcessLauncher { executable }
    }
}

impl engine::service::Launcher for ProcessLauncher {
    fn launch(&self, number: u32) -> std::io::Result<engine::service::Launched> {
        #[cfg(windows)]
        {
            launch::launch(&self.executable, number)
        }
        #[cfg(not(windows))]
        {
            portable::launch(&self.executable, number)
        }
    }
}

#[cfg(not(windows))]
mod portable {
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

    pub(super) fn launch(executable: &Path, number: u32) -> std::io::Result<Launched> {
        let mut child = Command::new(executable)
            .arg("--renderer")
            .arg(number.to_string())
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

// The renderers' launch, through `unsafe` (M34.2).
#[cfg(windows)]
#[allow(unsafe_code)]
mod launch {
    use std::ffi::OsStr;
    use std::fs::File;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle};
    use std::path::Path;

    use engine::service::{Launched, RendererProcess};
    use windows_sys::Win32::Foundation::{
        HANDLE, HANDLE_FLAG_INHERIT, SetHandleInformation, WAIT_OBJECT_0,
    };
    use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject,
    };
    use windows_sys::Win32::System::Pipes::CreatePipe;
    use windows_sys::Win32::System::Threading::{
        CREATE_NO_WINDOW, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, CreateProcessW,
        DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT, GetExitCodeProcess,
        InitializeProcThreadAttributeList, LPPROC_THREAD_ATTRIBUTE_LIST,
        PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROCESS_INFORMATION, ResumeThread, STARTF_USESTDHANDLES,
        STARTUPINFOEXW, TerminateProcess, UpdateProcThreadAttribute, WaitForSingleObject,
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
        // SAFETY: `handle` is open and ours for the call; the call changes
        // its inheritance flag and nothing else.
        let set =
            unsafe { SetHandleInformation(raw(handle), HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT) };
        if set == 0 {
            return Err(last_error());
        }
        Ok(())
    }

    /// A job object that kills its processes when its last handle
    /// closes.
    fn job() -> std::io::Result<OwnedHandle> {
        // SAFETY: no attributes and no name: a new unnamed job with the
        // default security, its handle returned.
        let job = owned(unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) })
            .ok_or_else(last_error)?;
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
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
        Ok(job)
    }

    fn wide(text: &OsStr) -> Vec<u16> {
        text.encode_wide().chain(Some(0)).collect()
    }

    /// A renderer's process and the job that holds it: dropping the job's
    /// handle kills it.
    struct JobRenderer {
        process: OwnedHandle,
        job: OwnedHandle,
        id: u32,
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
    }

    /// An attribute list naming three handles as the only ones a process
    /// inherits; deleted when dropped.
    struct HandleList {
        /// Words, so that the list is aligned for the pointers in it.
        buffer: Vec<usize>,
        /// Boxed, so that they do not move while the list points at them.
        handles: Box<[HANDLE; 3]>,
        made: bool,
    }

    impl HandleList {
        fn new(handles: [HANDLE; 3]) -> std::io::Result<HandleList> {
            let mut size = 0usize;
            // SAFETY: a null list asks for the size one attribute needs,
            // written to `size`; the call fails by design, with the size.
            unsafe { InitializeProcThreadAttributeList(std::ptr::null_mut(), 1, 0, &mut size) };
            let mut list = HandleList {
                buffer: vec![0; size.div_ceil(size_of::<usize>()).max(1)],
                handles: Box::new(handles),
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
            // SAFETY: the list was made above; the value is the boxed array
            // of handles, of the size given, which lives (and does not
            // move) as long as the list, as the attribute requires.
            let updated = unsafe {
                UpdateProcThreadAttribute(
                    list.list(),
                    0,
                    PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                    value.cast(),
                    size_of::<[HANDLE; 3]>(),
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

    pub(super) fn launch(executable: &Path, number: u32) -> std::io::Result<Launched> {
        let (child_input, to_renderer) = pipe()?;
        let (from_renderer, child_output) = pipe()?;
        let (log, child_log) = pipe()?;
        for end in [&child_input, &child_output, &child_log] {
            inheritable(end)?;
        }
        let job = job()?;
        let application = wide(executable.as_os_str());
        let mut command = wide(OsStr::new(&format!(
            "\"{}\" --renderer {number}",
            executable.display()
        )));
        let mut list = HandleList::new([raw(&child_input), raw(&child_output), raw(&child_log)])?;
        let mut startup = STARTUPINFOEXW::default();
        startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
        startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        startup.StartupInfo.hStdInput = raw(&child_input);
        startup.StartupInfo.hStdOutput = raw(&child_output);
        startup.StartupInfo.hStdError = raw(&child_log);
        startup.lpAttributeList = list.list();
        let mut made = PROCESS_INFORMATION::default();
        // SAFETY: the application name and the command line are
        // NUL-terminated wide strings we own, the command line mutable as
        // the call requires; no process or thread attributes, the
        // environment and directory ours; the startup structure is the
        // extended one its flag says, its list naming the three
        // inheritable handles above, all alive for the call; `made` is
        // ours to fill.
        let created = unsafe {
            CreateProcessW(
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
        let process = owned(made.hProcess).ok_or_else(last_error)?;
        let thread = owned(made.hThread).ok_or_else(last_error)?;
        let mut renderer = JobRenderer {
            process,
            job,
            id: made.dwProcessId,
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
