const OPENAI_CREDENTIAL_TARGET: &str = "Tempo/OpenAI API key";

pub fn key_is_configured() -> bool {
    std::env::var("OPENAI_API_KEY")
        .ok()
        .is_some_and(|value| !value.trim().is_empty())
}

pub fn load_openai_key_into_environment() {
    if key_is_configured() {
        return;
    }
    if let Ok(Some(key)) = read_openai_key() {
        std::env::set_var("OPENAI_API_KEY", key);
    }
}

pub fn save_openai_key(key: &str) -> Result<(), String> {
    let key = key.trim();
    if key.len() < 20 {
        return Err("That does not look like a complete OpenAI API key".to_string());
    }
    write_openai_key(key)?;
    std::env::set_var("OPENAI_API_KEY", key);
    Ok(())
}

pub fn clear_openai_key() -> Result<(), String> {
    delete_openai_key()?;
    std::env::remove_var("OPENAI_API_KEY");
    Ok(())
}

#[cfg(windows)]
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
fn write_openai_key(key: &str) -> Result<(), String> {
    use windows::core::PWSTR;
    use windows::Win32::Security::Credentials::{
        CredWriteW, CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC,
    };

    let mut target = wide(OPENAI_CREDENTIAL_TARGET);
    let mut username = wide("Tempo");
    let mut blob = key.as_bytes().to_vec();
    let credential = CREDENTIALW {
        Type: CRED_TYPE_GENERIC,
        TargetName: PWSTR(target.as_mut_ptr()),
        CredentialBlobSize: blob.len() as u32,
        CredentialBlob: blob.as_mut_ptr(),
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        UserName: PWSTR(username.as_mut_ptr()),
        ..Default::default()
    };
    unsafe { CredWriteW(&credential, 0) }
        .map_err(|error| format!("Could not save the key in Windows Credential Manager: {error}"))
}

#[cfg(windows)]
fn read_openai_key() -> Result<Option<String>, String> {
    use std::ptr::null_mut;
    use windows::core::PCWSTR;
    use windows::Win32::Security::Credentials::{
        CredFree, CredReadW, CREDENTIALW, CRED_TYPE_GENERIC,
    };

    let target = wide(OPENAI_CREDENTIAL_TARGET);
    let mut credential: *mut CREDENTIALW = null_mut();
    let result = unsafe {
        CredReadW(
            PCWSTR(target.as_ptr()),
            CRED_TYPE_GENERIC,
            0,
            &mut credential,
        )
    };
    if result.is_err() {
        return Ok(None);
    }
    if credential.is_null() {
        return Ok(None);
    }
    let key = unsafe {
        let value = &*credential;
        let bytes = std::slice::from_raw_parts(
            value.CredentialBlob,
            value.CredentialBlobSize as usize,
        );
        String::from_utf8(bytes.to_vec())
            .map_err(|_| "Stored OpenAI key was not valid UTF-8".to_string())
    };
    unsafe { CredFree(credential.cast()) };
    key.map(Some)
}

#[cfg(windows)]
fn delete_openai_key() -> Result<(), String> {
    use windows::core::PCWSTR;
    use windows::Win32::Security::Credentials::{CredDeleteW, CRED_TYPE_GENERIC};

    let target = wide(OPENAI_CREDENTIAL_TARGET);
    let _ = unsafe { CredDeleteW(PCWSTR(target.as_ptr()), CRED_TYPE_GENERIC, 0) };
    Ok(())
}

#[cfg(not(windows))]
fn write_openai_key(_key: &str) -> Result<(), String> {
    Err("Secure in-app key storage is currently available on Windows. Set OPENAI_API_KEY in the environment on this device.".to_string())
}

#[cfg(not(windows))]
fn read_openai_key() -> Result<Option<String>, String> {
    Ok(None)
}

#[cfg(not(windows))]
fn delete_openai_key() -> Result<(), String> {
    Ok(())
}
