use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};

#[cfg(target_os = "windows")]
pub mod dpapi {
    use std::ptr;

    #[repr(C)]
    #[allow(non_snake_case)]
    struct DATA_BLOB {
        cbData: u32,
        pbData: *mut u8,
    }

    #[link(name = "crypt32")]
    extern "system" {
        fn CryptUnprotectData(
            pDataIn: *const DATA_BLOB,
            ppszDataDescr: *mut *mut u16,
            pOptionalEntropy: *const DATA_BLOB,
            pvReserved: *mut std::ffi::c_void,
            pPromptStruct: *mut std::ffi::c_void,
            dwFlags: u32,
            pDataOut: *mut DATA_BLOB,
        ) -> i32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn LocalFree(hMem: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
    }

    pub fn unprotect_data(encrypted: &[u8]) -> Result<Vec<u8>, String> {
        let in_blob = DATA_BLOB {
            cbData: encrypted.len() as u32,
            pbData: encrypted.as_ptr() as *mut u8,
        };
        let mut out_blob = DATA_BLOB {
            cbData: 0,
            pbData: ptr::null_mut(),
        };

        let success = unsafe {
            CryptUnprotectData(
                &in_blob,
                ptr::null_mut(),
                ptr::null(),
                ptr::null_mut(),
                ptr::null_mut(),
                0,
                &mut out_blob,
            )
        };

        if success == 0 {
            return Err(format!("CryptUnprotectData failed: {}", std::io::Error::last_os_error()));
        }

        let slice = unsafe {
            std::slice::from_raw_parts(out_blob.pbData, out_blob.cbData as usize)
        };
        let result = slice.to_vec();

        unsafe {
            LocalFree(out_blob.pbData as *mut std::ffi::c_void);
        }

        Ok(result)
    }
}

#[cfg(target_os = "windows")]
pub mod cred_mgr {
    use std::os::windows::ffi::OsStrExt;
    use std::ptr;

    #[repr(C)]
    struct FILETIME {
        dw_low_date_time: u32,
        dw_high_date_time: u32,
    }

    #[repr(C)]
    struct CREDENTIALW {
        flags: u32,
        cred_type: u32,
        target_name: *const u16,
        comment: *const u16,
        last_written: FILETIME,
        credential_blob_size: u32,
        credential_blob: *const u8,
        persist: u32,
        attribute_count: u32,
        attributes: *const std::ffi::c_void,
        target_alias: *const u16,
        user_name: *const u16,
    }

    #[link(name = "advapi32")]
    extern "system" {
        fn CredReadW(target_name: *const u16, type_: u32, flags: u32, credential: *mut *mut CREDENTIALW) -> i32;
        fn CredWriteW(credential: *const CREDENTIALW, flags: u32) -> i32;
        fn CredDeleteW(target_name: *const u16, type_: u32, flags: u32) -> i32;
        fn CredFree(buffer: *mut std::ffi::c_void);
    }

    pub fn read_credential(target: &str) -> Result<Vec<u8>, String> {
        let target_wide: Vec<u16> = std::ffi::OsStr::new(target)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        let mut cred_ptr: *mut CREDENTIALW = ptr::null_mut();
        let res = unsafe { CredReadW(target_wide.as_ptr(), 1, 0, &mut cred_ptr) };
        if res == 0 || cred_ptr.is_null() {
            return Err(format!("CredReadW failed for '{}': {}", target, std::io::Error::last_os_error()));
        }

        let slice = unsafe {
            std::slice::from_raw_parts((*cred_ptr).credential_blob, (*cred_ptr).credential_blob_size as usize)
        };
        let result = slice.to_vec();

        unsafe {
            CredFree(cred_ptr as *mut std::ffi::c_void);
        }

        Ok(result)
    }

    pub fn write_credential(target: &str, user: &str, blob: &[u8]) -> Result<(), String> {
        let target_wide: Vec<u16> = std::ffi::OsStr::new(target)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let user_wide: Vec<u16> = std::ffi::OsStr::new(user)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        let cred = CREDENTIALW {
            flags: 0,
            cred_type: 1, // CRED_TYPE_GENERIC
            target_name: target_wide.as_ptr(),
            comment: ptr::null(),
            last_written: FILETIME { dw_low_date_time: 0, dw_high_date_time: 0 },
            credential_blob_size: blob.len() as u32,
            credential_blob: blob.as_ptr(),
            persist: 2, // CRED_PERSIST_LOCAL_MACHINE
            attribute_count: 0,
            attributes: ptr::null(),
            target_alias: ptr::null(),
            user_name: user_wide.as_ptr(),
        };

        unsafe {
            let _ = CredDeleteW(target_wide.as_ptr(), 1, 0);
            let res = CredWriteW(&cred, 0);
            if res == 0 {
                return Err(format!("CredWriteW failed: {}", std::io::Error::last_os_error()));
            }
        }

        Ok(())
    }

    pub fn delete_credential(target: &str) -> Result<(), String> {
        let target_wide: Vec<u16> = std::ffi::OsStr::new(target)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        let res = unsafe { CredDeleteW(target_wide.as_ptr(), 1, 0) };
        if res == 0 {
            return Err(format!("CredDeleteW failed for '{}': {}", target, std::io::Error::last_os_error()));
        }
        Ok(())
    }
}

pub fn decrypt_chromium_v10(key: &[u8], ciphertext_blob: &[u8]) -> Result<Vec<u8>, String> {
    if ciphertext_blob.len() < 3 + 12 + 16 || &ciphertext_blob[..3] != b"v10" {
        return Err("Invalid chromium encrypted v10 payload format".to_string());
    }
    let nonce = Nonce::from_slice(&ciphertext_blob[3..15]);
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|e| e.to_string())?;
    let plaintext = cipher
        .decrypt(nonce, &ciphertext_blob[15..])
        .map_err(|e| format!("AES-GCM decryption failed: {}", e))?;
    Ok(plaintext)
}
