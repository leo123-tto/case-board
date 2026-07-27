use std::fs::{self, File, OpenOptions};
use std::path::Path;

use super::paths::BridgePaths;
use super::types::{BridgeError, BridgeResult};

const OWNER_DIRECTORY_MODE: u32 = 0o700;
const OWNER_FILE_MODE: u32 = 0o600;

pub(crate) fn ensure_bridge_directories(paths: &BridgePaths) -> BridgeResult<()> {
    ensure_safe_app_data_root(paths.app_data_root())?;
    let canonical_app_data =
        fs::canonicalize(paths.app_data_root()).map_err(|source| BridgeError::Io {
            operation: "canonicalize app-data root",
            path: paths.app_data_root().to_path_buf(),
            source,
        })?;
    let namespace = paths
        .bridge_root()
        .parent()
        .ok_or(BridgeError::InvalidInput("bridge namespace has no parent"))?;
    ensure_secure_directory(namespace)?;
    ensure_secure_directory(paths.bridge_root())?;
    for path in [namespace, paths.bridge_root()] {
        let canonical = fs::canonicalize(path).map_err(|source| BridgeError::Io {
            operation: "canonicalize credential bridge directory",
            path: path.to_path_buf(),
            source,
        })?;
        if !canonical.starts_with(&canonical_app_data) {
            return Err(BridgeError::SymlinkNotAllowed {
                path: path.to_path_buf(),
            });
        }
    }
    Ok(())
}

fn ensure_safe_app_data_root(path: &Path) -> BridgeResult<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            reject_link_or_reparse(path, &metadata)?;
            if !metadata.is_dir() {
                return Err(BridgeError::NotDirectory {
                    path: path.to_path_buf(),
                });
            }
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            create_owner_only_directory(path)?;
            let metadata = fs::symlink_metadata(path).map_err(|source| BridgeError::Io {
                operation: "verify created app-data root",
                path: path.to_path_buf(),
                source,
            })?;
            reject_link_or_reparse(path, &metadata)?;
            if !metadata.is_dir() {
                return Err(BridgeError::NotDirectory {
                    path: path.to_path_buf(),
                });
            }
            Ok(())
        }
        Err(source) => Err(BridgeError::Io {
            operation: "inspect app-data root",
            path: path.to_path_buf(),
            source,
        }),
    }
}

pub(crate) fn ensure_secure_directory(path: &Path) -> BridgeResult<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            reject_link_or_reparse(path, &metadata)?;
            if !metadata.is_dir() {
                return Err(BridgeError::NotDirectory {
                    path: path.to_path_buf(),
                });
            }
            verify_owner_only(path, &metadata, OWNER_DIRECTORY_MODE)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            create_owner_only_directory(path)?;
            let metadata = fs::symlink_metadata(path).map_err(|source| BridgeError::Io {
                operation: "verify created directory",
                path: path.to_path_buf(),
                source,
            })?;
            reject_link_or_reparse(path, &metadata)?;
            verify_owner_only(path, &metadata, OWNER_DIRECTORY_MODE)
        }
        Err(source) => Err(BridgeError::Io {
            operation: "inspect directory",
            path: path.to_path_buf(),
            source,
        }),
    }
}

pub(crate) fn create_secure_file(path: &Path) -> BridgeResult<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    configure_owner_only_file_mode(&mut options);
    let file = options.open(path).map_err(|source| BridgeError::Io {
        operation: "create owner-only file",
        path: path.to_path_buf(),
        source,
    })?;

    #[cfg(windows)]
    if let Err(error) = windows_acl::apply_current_user_only_acl(path) {
        drop(file);
        let _ = fs::remove_file(path);
        return Err(error);
    }

    let metadata = fs::symlink_metadata(path).map_err(|source| BridgeError::Io {
        operation: "verify created file",
        path: path.to_path_buf(),
        source,
    })?;
    reject_link_or_reparse(path, &metadata)?;
    if !metadata.is_file() {
        return Err(BridgeError::NonRegularFile {
            path: path.to_path_buf(),
        });
    }
    verify_owner_only(path, &metadata, OWNER_FILE_MODE)?;
    Ok(file)
}

pub(crate) fn open_or_create_secure_file(path: &Path) -> BridgeResult<File> {
    match create_secure_file(path) {
        Ok(file) => Ok(file),
        Err(BridgeError::Io { source, .. })
            if source.kind() == std::io::ErrorKind::AlreadyExists =>
        {
            verify_secure_file(path)?;
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .open(path)
                .map_err(|source| BridgeError::Io {
                    operation: "open owner-only file",
                    path: path.to_path_buf(),
                    source,
                })?;
            verify_secure_file(path)?;
            Ok(file)
        }
        Err(error) => Err(error),
    }
}

pub(crate) fn verify_secure_file(path: &Path) -> BridgeResult<()> {
    let metadata = fs::symlink_metadata(path).map_err(|source| BridgeError::Io {
        operation: "inspect owner-only file",
        path: path.to_path_buf(),
        source,
    })?;
    reject_link_or_reparse(path, &metadata)?;
    if !metadata.is_file() {
        return Err(BridgeError::NonRegularFile {
            path: path.to_path_buf(),
        });
    }
    verify_owner_only(path, &metadata, OWNER_FILE_MODE)
}

fn reject_link_or_reparse(path: &Path, metadata: &fs::Metadata) -> BridgeResult<()> {
    if metadata.file_type().is_symlink() || is_windows_reparse_point(metadata) {
        return Err(BridgeError::SymlinkNotAllowed {
            path: path.to_path_buf(),
        });
    }
    Ok(())
}

#[cfg(windows)]
fn is_windows_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_windows_reparse_point(_metadata: &fs::Metadata) -> bool {
    false
}

#[cfg(unix)]
fn create_owner_only_directory(path: &Path) -> BridgeResult<()> {
    use std::os::unix::fs::DirBuilderExt;
    let mut builder = fs::DirBuilder::new();
    builder.mode(OWNER_DIRECTORY_MODE);
    builder.create(path).map_err(|source| BridgeError::Io {
        operation: "create owner-only directory",
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(windows)]
fn create_owner_only_directory(path: &Path) -> BridgeResult<()> {
    fs::create_dir(path).map_err(|source| BridgeError::Io {
        operation: "create owner-only directory",
        path: path.to_path_buf(),
        source,
    })?;
    if let Err(error) = windows_acl::apply_current_user_only_acl(path) {
        let _ = fs::remove_dir(path);
        return Err(error);
    }
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn create_owner_only_directory(path: &Path) -> BridgeResult<()> {
    fs::create_dir(path).map_err(|source| BridgeError::Io {
        operation: "create owner-only directory",
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(unix)]
fn configure_owner_only_file_mode(options: &mut OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;
    options.mode(OWNER_FILE_MODE);
}

#[cfg(not(unix))]
fn configure_owner_only_file_mode(_options: &mut OpenOptions) {}

#[cfg(unix)]
fn verify_owner_only(path: &Path, metadata: &fs::Metadata, expected_mode: u32) -> BridgeResult<()> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    let actual_mode = metadata.permissions().mode() & 0o777;
    validate_unix_security_facts(
        path,
        expected_mode,
        current_effective_uid(),
        metadata.uid(),
        actual_mode,
    )
}

#[cfg(unix)]
fn validate_unix_security_facts(
    path: &Path,
    expected_mode: u32,
    expected_uid: u32,
    actual_uid: u32,
    actual_mode: u32,
) -> BridgeResult<()> {
    if actual_mode != expected_mode {
        return Err(BridgeError::UnsafePermissions {
            path: path.to_path_buf(),
            expected: expected_mode,
            actual: actual_mode,
        });
    }

    if actual_uid != expected_uid {
        return Err(BridgeError::OwnerMismatch {
            path: path.to_path_buf(),
            expected: expected_uid,
            actual: actual_uid,
        });
    }
    Ok(())
}

#[cfg(unix)]
fn current_effective_uid() -> u32 {
    unsafe extern "C" {
        fn geteuid() -> u32;
    }
    // SAFETY: geteuid has no arguments and returns the effective uid of this process.
    unsafe { geteuid() }
}

#[cfg(windows)]
fn verify_owner_only(
    path: &Path,
    _metadata: &fs::Metadata,
    _expected_mode: u32,
) -> BridgeResult<()> {
    windows_acl::verify_current_user_only_acl(path)
}

#[cfg(not(any(unix, windows)))]
fn verify_owner_only(
    _path: &Path,
    _metadata: &fs::Metadata,
    _expected_mode: u32,
) -> BridgeResult<()> {
    Ok(())
}

#[cfg(windows)]
pub fn verify_current_user_only_acl(path: &Path) -> BridgeResult<()> {
    windows_acl::verify_current_user_only_acl(path)
}

#[cfg(windows)]
mod windows_acl {
    use std::ffi::c_void;
    use std::mem::{size_of, zeroed};
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    use std::ptr::{null, null_mut};

    use windows_sys::Win32::Foundation::{
        CloseHandle, LocalFree, GENERIC_ALL, HANDLE, WIN32_ERROR,
    };
    use windows_sys::Win32::Security::Authorization::{
        GetNamedSecurityInfoW, SetEntriesInAclW, SetNamedSecurityInfoW, EXPLICIT_ACCESS_W,
        SET_ACCESS, SE_FILE_OBJECT, TRUSTEE_IS_SID, TRUSTEE_IS_USER, TRUSTEE_W,
    };
    use windows_sys::Win32::Security::{
        AclSizeInformation, EqualSid, GetAce, GetAclInformation, GetSecurityDescriptorControl,
        GetTokenInformation, TokenUser, ACCESS_ALLOWED_ACE, ACL, ACL_SIZE_INFORMATION,
        DACL_SECURITY_INFORMATION, NO_INHERITANCE, OWNER_SECURITY_INFORMATION,
        PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID, SE_DACL_PROTECTED,
        TOKEN_QUERY, TOKEN_USER,
    };
    use windows_sys::Win32::Storage::FileSystem::FILE_ALL_ACCESS;
    use windows_sys::Win32::System::SystemServices::ACCESS_ALLOWED_ACE_TYPE;
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    use super::{BridgeError, BridgeResult};

    struct CurrentUserSid {
        token: HANDLE,
        buffer: Vec<usize>,
        sid: PSID,
    }

    impl CurrentUserSid {
        fn load(path: &Path) -> BridgeResult<Self> {
            let mut token = null_mut();
            // SAFETY: process pseudo-handle is valid; token receives a real handle.
            if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
                return Err(acl_error(path, "OpenProcessToken", last_error()));
            }

            let mut bytes = 0u32;
            // SAFETY: the first call intentionally queries the required buffer size.
            unsafe {
                GetTokenInformation(token, TokenUser, null_mut(), 0, &mut bytes);
            }
            if bytes < size_of::<TOKEN_USER>() as u32 {
                unsafe {
                    CloseHandle(token);
                }
                return Err(acl_error(path, "GetTokenInformation(size)", last_error()));
            }

            let word_size = size_of::<usize>();
            let words = (bytes as usize + word_size - 1) / word_size;
            let mut buffer = vec![0usize; words];
            // SAFETY: buffer is aligned and has at least `bytes` writable bytes.
            if unsafe {
                GetTokenInformation(
                    token,
                    TokenUser,
                    buffer.as_mut_ptr().cast(),
                    bytes,
                    &mut bytes,
                )
            } == 0
            {
                let code = last_error();
                unsafe {
                    CloseHandle(token);
                }
                return Err(acl_error(path, "GetTokenInformation", code));
            }

            // SAFETY: successful TokenUser query initialized a TOKEN_USER at buffer start.
            let sid = unsafe { (*(buffer.as_ptr().cast::<TOKEN_USER>())).User.Sid };
            if sid.is_null() {
                unsafe {
                    CloseHandle(token);
                }
                return Err(acl_error(path, "TokenUser SID", 0));
            }
            Ok(Self { token, buffer, sid })
        }

        fn sid(&self) -> PSID {
            let _keep_alive = &self.buffer;
            self.sid
        }
    }

    impl Drop for CurrentUserSid {
        fn drop(&mut self) {
            // SAFETY: token is a real handle returned by OpenProcessToken.
            unsafe {
                CloseHandle(self.token);
            }
        }
    }

    pub(super) fn apply_current_user_only_acl(path: &Path) -> BridgeResult<()> {
        let current = CurrentUserSid::load(path)?;
        let mut explicit = EXPLICIT_ACCESS_W {
            grfAccessPermissions: GENERIC_ALL,
            grfAccessMode: SET_ACCESS,
            grfInheritance: NO_INHERITANCE,
            Trustee: TRUSTEE_W {
                pMultipleTrustee: null_mut(),
                MultipleTrusteeOperation: 0,
                TrusteeForm: TRUSTEE_IS_SID,
                TrusteeType: TRUSTEE_IS_USER,
                ptstrName: current.sid().cast(),
            },
        };
        let mut acl: *mut ACL = null_mut();
        // SAFETY: explicit and SID remain alive for the call; output ACL is LocalAlloc-owned.
        let result = unsafe { SetEntriesInAclW(1, &mut explicit, null(), &mut acl) };
        if result != 0 {
            return Err(acl_error(path, "SetEntriesInAclW", result));
        }

        let mut wide = path
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>();
        // SAFETY: wide is NUL-terminated and ACL is valid until LocalFree below.
        let set_result = unsafe {
            SetNamedSecurityInfoW(
                wide.as_mut_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                null_mut(),
                null_mut(),
                acl,
                null(),
            )
        };
        // SAFETY: SetEntriesInAclW allocated ACL with LocalAlloc.
        unsafe {
            LocalFree(acl.cast());
        }
        if set_result != 0 {
            return Err(acl_error(path, "SetNamedSecurityInfoW", set_result));
        }
        verify_current_user_only_acl(path)
    }

    pub(super) fn verify_current_user_only_acl(path: &Path) -> BridgeResult<()> {
        let current = CurrentUserSid::load(path)?;
        let wide = path
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>();
        let mut owner: PSID = null_mut();
        let mut dacl: *mut ACL = null_mut();
        let mut descriptor: PSECURITY_DESCRIPTOR = null_mut();
        // SAFETY: wide is NUL-terminated; descriptor is LocalAlloc-owned on success.
        let result = unsafe {
            GetNamedSecurityInfoW(
                wide.as_ptr(),
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                &mut owner,
                null_mut(),
                &mut dacl,
                null_mut(),
                &mut descriptor,
            )
        };
        if result != 0 {
            return Err(acl_error(path, "GetNamedSecurityInfoW", result));
        }

        let verified = verify_descriptor(path, descriptor, owner, dacl, current.sid());
        // SAFETY: GetNamedSecurityInfoW allocated descriptor with LocalAlloc.
        unsafe {
            LocalFree(descriptor);
        }
        verified
    }

    fn verify_descriptor(
        path: &Path,
        descriptor: PSECURITY_DESCRIPTOR,
        owner: PSID,
        dacl: *mut ACL,
        current_sid: PSID,
    ) -> BridgeResult<()> {
        if descriptor.is_null() || owner.is_null() || dacl.is_null() {
            return Err(acl_error(path, "missing owner/protected DACL", 0));
        }
        // SAFETY: all pointers originate from a successful GetNamedSecurityInfoW call.
        if unsafe { EqualSid(owner, current_sid) } == 0 {
            return Err(acl_error(path, "owner is not current user", 0));
        }

        let mut control = 0u16;
        let mut revision = 0u32;
        // SAFETY: descriptor is valid for this call.
        if unsafe { GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) } == 0 {
            return Err(acl_error(
                path,
                "GetSecurityDescriptorControl",
                last_error(),
            ));
        }
        if control & SE_DACL_PROTECTED == 0 {
            return Err(acl_error(path, "DACL inheritance is not protected", 0));
        }

        let mut info: ACL_SIZE_INFORMATION = unsafe { zeroed() };
        // SAFETY: dacl is valid and info has the required size/alignment.
        if unsafe {
            GetAclInformation(
                dacl,
                (&mut info as *mut ACL_SIZE_INFORMATION).cast::<c_void>(),
                size_of::<ACL_SIZE_INFORMATION>() as u32,
                AclSizeInformation,
            )
        } == 0
        {
            return Err(acl_error(path, "GetAclInformation", last_error()));
        }
        if info.AceCount != 1 {
            return Err(acl_error(path, "DACL must contain exactly one ACE", 0));
        }

        let mut ace_ptr: *mut c_void = null_mut();
        // SAFETY: the ACL reports one ACE at index 0.
        if unsafe { GetAce(dacl, 0, &mut ace_ptr) } == 0 || ace_ptr.is_null() {
            return Err(acl_error(path, "GetAce", last_error()));
        }
        // SAFETY: GetAce returned an ACCESS_ALLOWED_ACE-sized entry after type check.
        let ace = unsafe { &*(ace_ptr.cast::<ACCESS_ALLOWED_ACE>()) };
        if ace.Header.AceType != ACCESS_ALLOWED_ACE_TYPE as u8 {
            return Err(acl_error(path, "sole ACE is not ACCESS_ALLOWED", 0));
        }
        if ace.Mask != GENERIC_ALL && ace.Mask != FILE_ALL_ACCESS {
            return Err(acl_error(path, "sole ACE does not grant full access", 0));
        }
        let ace_sid = (&ace.SidStart as *const u32).cast_mut().cast::<c_void>();
        // SAFETY: SidStart begins the variable-length SID within ACCESS_ALLOWED_ACE.
        if unsafe { EqualSid(ace_sid, current_sid) } == 0 {
            return Err(acl_error(path, "sole ACE is not current user", 0));
        }
        Ok(())
    }

    fn last_error() -> WIN32_ERROR {
        std::io::Error::last_os_error()
            .raw_os_error()
            .unwrap_or_default() as WIN32_ERROR
    }

    fn acl_error(path: &Path, operation: &str, code: WIN32_ERROR) -> BridgeError {
        BridgeError::WindowsAclNotCurrentUserOnly {
            path: path.to_path_buf(),
            reason: if code == 0 {
                operation.to_owned()
            } else {
                format!("{operation} failed with Win32 error {code}")
            },
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn unix_permission_contract_rejects_a_different_owner() {
        let path = Path::new("/synthetic/credential-bridge/master-key.v1");
        let error = validate_unix_security_facts(path, 0o600, 501, 502, 0o600)
            .expect_err("wrong owner must fail closed");
        assert!(matches!(
            error,
            BridgeError::OwnerMismatch {
                ref path,
                expected: 501,
                actual: 502
            } if path == Path::new("/synthetic/credential-bridge/master-key.v1")
        ));
    }
}
