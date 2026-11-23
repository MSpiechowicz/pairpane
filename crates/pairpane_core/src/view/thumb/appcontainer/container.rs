//! The AppContainer profile of the thumbnail helper, and the right to read and run the helper's file that its container
//! may have to be given.
#![allow(
    unsafe_code,
    reason = "the AppContainer and security descriptor calls; see each `unsafe` block"
)]
#![deny(clippy::undocumented_unsafe_blocks)]

use std::ffi::{OsStr, c_void};
use std::fs::File;
use std::io;
use std::mem::size_of;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::path::Path;
use std::ptr;

use super::step::{at, at_code, at_result};

use windows_sys::Win32::Foundation::LocalFree;
use windows_sys::Win32::Security::Authorization::{
    EXPLICIT_ACCESS_W, GRANT_ACCESS, GetSecurityInfo, NO_MULTIPLE_TRUSTEE, SE_FILE_OBJECT, SetEntriesInAclW,
    SetSecurityInfo, TRUSTEE_IS_SID, TRUSTEE_IS_UNKNOWN, TRUSTEE_W,
};
use windows_sys::Win32::Security::Isolation::{CreateAppContainerProfile, DeriveAppContainerSidFromAppContainerName};
use windows_sys::Win32::Security::{
    ACCESS_ALLOWED_ACE, ACL, ACL_SIZE_INFORMATION, AclSizeInformation, DACL_SECURITY_INFORMATION, EqualSid, FreeSid,
    GetAce, GetAclInformation, GetSecurityDescriptorControl, NO_INHERITANCE, PROTECTED_DACL_SECURITY_INFORMATION, PSID,
    SE_DACL_PROTECTED, UNPROTECTED_DACL_SECURITY_INFORMATION,
};
use windows_sys::Win32::Storage::FileSystem::{FILE_GENERIC_EXECUTE, FILE_GENERIC_READ, READ_CONTROL, WRITE_DAC};

/// What the container is given on the helper's file: to read it and to run it.
const WANTED: u32 = FILE_GENERIC_READ | FILE_GENERIC_EXECUTE;

/// The name of the container: fixed, so every run of the app (and every version) is the same container, with the same
/// SID, which is a hash of this name.
const NAME: &str = "com.mspiechowicz.pairpane.thumbnail";

/// `HRESULT_FROM_WIN32(ERROR_ALREADY_EXISTS)`: the profile was made by an earlier run.
const ALREADY_EXISTS: i32 = 0x8007_00B7_u32 as i32;

/// The text of `text` as UTF-16 with the NUL that ends it.
pub(super) fn wide(text: &OsStr) -> Vec<u16> {
    text.encode_wide().chain(std::iter::once(0)).collect()
}

/// The SID of the container. Freed when dropped.
pub(super) struct Sid(PSID);

impl Sid {
    /// The SID, for as long as `self` lives.
    pub(super) fn as_ptr(&self) -> PSID {
        self.0
    }
}

impl Drop for Sid {
    fn drop(&mut self) {
        // SAFETY: `self.0` is a SID that `CreateAppContainerProfile` or `DeriveAppContainerSidFromAppContainerName`
        // allocated for this value (both say to free it with `FreeSid`), freed here once.
        unsafe { FreeSid(self.0) };
    }
}

/// The SID of the container, made when the profile is not there yet. The profile is a small registry key and a folder in
/// the user's profile that Windows keeps for the container; it is never deleted here.
pub(super) fn container_sid() -> io::Result<Sid> {
    let name = wide(OsStr::new(NAME));
    let display = wide(OsStr::new("Pairpane thumbnails"));
    let description = wide(OsStr::new(
        "Makes the small previews of pictures, with no access to files or the network",
    ));
    let mut sid: PSID = ptr::null_mut();

    // SAFETY: the three texts are NUL-terminated and live through the call; no capabilities (NULL and a count of 0) is a
    // container with none; `sid` is a place for the SID, which is set only on success.
    let made = unsafe {
        CreateAppContainerProfile(
            name.as_ptr(),
            display.as_ptr(),
            description.as_ptr(),
            ptr::null(),
            0,
            &raw mut sid,
        )
    };
    if made >= 0 {
        return Ok(Sid(sid));
    }
    if made != ALREADY_EXISTS {
        return Err(at_result("CreateAppContainerProfile", made));
    }

    // SAFETY: the name is NUL-terminated and lives through the call; `sid` is a place for the SID, set only on success.
    let derived = unsafe { DeriveAppContainerSidFromAppContainerName(name.as_ptr(), &raw mut sid) };
    if derived < 0 {
        return Err(at_result("DeriveAppContainerSidFromAppContainerName", derived));
    }

    Ok(Sid(sid))
}

/// A block that `GetNamedSecurityInfoW` or `SetEntriesInAclW` allocated, freed with `LocalFree` when dropped.
struct Local(*mut c_void);

impl Drop for Local {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: the pointer is a block the system allocated for this value and says to free with `LocalFree`, freed
            // here once.
            unsafe { LocalFree(self.0) };
        }
    }
}

/// Whether the access list `acl` has an entry that allows the container `sid` all of `rights`.
fn allows(acl: *const ACL, sid: &Sid, rights: u32) -> bool {
    let mut size = ACL_SIZE_INFORMATION::default();

    // SAFETY: `acl` is the live access list of a descriptor the caller holds; `size` is the structure the class
    // `AclSizeInformation` fills, and its length is given.
    let known = unsafe {
        GetAclInformation(
            acl,
            (&raw mut size).cast(),
            size_of::<ACL_SIZE_INFORMATION>() as u32,
            AclSizeInformation,
        )
    };
    if known == 0 {
        return false;
    }

    for index in 0..size.AceCount {
        let mut entry: *mut c_void = ptr::null_mut();

        // SAFETY: `index` is under the number of entries the list has; `entry` is a place for a pointer into the list.
        if unsafe { GetAce(acl, index, &raw mut entry) } == 0 || entry.is_null() {
            continue;
        }
        let allowed: *const ACCESS_ALLOWED_ACE = entry.cast();

        // SAFETY: `entry` points into the live list at an entry whose header says what it is; only an "access allowed" entry
        // (type 0, which has the layout of `ACCESS_ALLOWED_ACE`: header, mask and the SID that starts at `SidStart`) is
        // read further, and its SID is compared with the container's, both live.
        let found = unsafe {
            (*allowed).Header.AceType == 0
                && (*allowed).Mask & rights == rights
                && EqualSid(ptr::addr_of!((*allowed).SidStart).cast_mut().cast(), sid.as_ptr()) != 0
        };
        if found {
            return true;
        }
    }

    false
}

/// Gives the container `sid` the right to read and run the file `path` (an entry that is its own and no one else's, for
/// that one file, not inherited), on top of what its access list says already. Nothing is written when the list has the
/// entry already. The entry is never removed: it stays as long as the file does (an update that replaces the file drops
/// it).
///
/// The list is read and written through a handle to the file, opened for reading and changing the access list and for
/// nothing else, and not by name: nothing is propagated to other objects (a write by name would make Windows walk a
/// folder's whole tree), and it is that one file's list that changes. Does nothing when the file has no access list at all
/// (everyone may then use it, and a list with one entry would shut the others out). Whether the list is protected from
/// inheritance is kept as it is (the control bits are read and passed back deliberately), so the change neither cuts the
/// file off from its folder's list nor ties it to it. Needs the right to change the access list, which the owner has;
/// without it this fails and the container cannot run the file.
///
/// The read, the merge and the write are three calls and not one atomic step: a change of the list by someone else in
/// between is lost. That is acceptable for a file that the app's own installer and updater write.
pub(super) fn allow_read_and_execute(path: &Path, sid: &Sid) -> io::Result<()> {
    let file = File::options()
        .access_mode(READ_CONTROL | WRITE_DAC)
        .open(path)
        .map_err(|error| at("open the helper's file for its access list", error))?;
    let handle = file.as_raw_handle();
    let mut old: *mut ACL = ptr::null_mut();
    let mut descriptor: *mut c_void = ptr::null_mut();

    // SAFETY: the handle is the live handle of `file`, opened with the right to read the access list; the owner, group and
    // system list are not asked for (NULL); `old` and `descriptor` are places for the file's access list, which lives
    // inside the descriptor, and for the descriptor, which is freed below through `Local`.
    let read = unsafe {
        GetSecurityInfo(
            handle,
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            ptr::null_mut(),
            &raw mut old,
            ptr::null_mut(),
            &raw mut descriptor,
        )
    };
    let _descriptor = Local(descriptor);
    if read != 0 {
        return Err(at_code("GetSecurityInfo", read));
    }
    if old.is_null() || allows(old, sid, WANTED) {
        return Ok(());
    }

    let mut control = 0_u16;
    let mut revision = 0_u32;

    // SAFETY: `descriptor` is the live security descriptor that the call above returned; `control` and `revision` are the
    // places for what it reports.
    let known = unsafe { GetSecurityDescriptorControl(descriptor, &raw mut control, &raw mut revision) };
    if known == 0 {
        return Err(at("GetSecurityDescriptorControl", io::Error::last_os_error()));
    }
    let protection = if control & SE_DACL_PROTECTED != 0 {
        PROTECTED_DACL_SECURITY_INFORMATION
    } else {
        UNPROTECTED_DACL_SECURITY_INFORMATION
    };

    let entry = EXPLICIT_ACCESS_W {
        grfAccessPermissions: WANTED,
        grfAccessMode: GRANT_ACCESS,
        grfInheritance: NO_INHERITANCE,
        Trustee: TRUSTEE_W {
            pMultipleTrustee: ptr::null_mut(),
            MultipleTrusteeOperation: NO_MULTIPLE_TRUSTEE,
            TrusteeForm: TRUSTEE_IS_SID,
            TrusteeType: TRUSTEE_IS_UNKNOWN,
            // For a trustee of the form SID this is the SID itself.
            ptstrName: sid.as_ptr().cast(),
        },
    };
    let mut new: *mut ACL = ptr::null_mut();

    // SAFETY: one entry, `entry`, whose SID lives through the call; `old` is the file's access list, alive as long as
    // `_descriptor`; `new` is a place for the new list, freed below through `Local`.
    let merged = unsafe { SetEntriesInAclW(1, &raw const entry, old, &raw mut new) };
    let _new = Local(new.cast());
    if merged != 0 {
        return Err(at_code("SetEntriesInAclW", merged));
    }

    // SAFETY: the handle is the live handle of `file`, opened with the right to change the access list; only the access
    // list is set (the owner, group and system list are NULL), to `new`, which lives until `_new` is dropped, with the
    // protection it had.
    let set = unsafe {
        SetSecurityInfo(
            handle,
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | protection,
            ptr::null_mut(),
            ptr::null_mut(),
            new,
            ptr::null(),
        )
    };
    if set != 0 {
        return Err(at_code("SetSecurityInfo", set));
    }

    Ok(())
}
