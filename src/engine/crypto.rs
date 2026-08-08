use super::*;

// Hardcoded key — 32 bytes (256-bit). Change this to your own random bytes.
// Generate with: python3 -c "import secrets; print(list(secrets.token_bytes(32)))"
const AES_KEY: &[u8; 32] = &[
    0x4A, 0x7F, 0x3C, 0x91, 0xB2, 0x5E, 0xD8, 0x0F,
    0x6A, 0xC3, 0x17, 0xEE, 0x84, 0x29, 0x5B, 0xF0,
    0x73, 0xA1, 0x9D, 0x46, 0x2C, 0x88, 0xF5, 0x1B,
    0xE7, 0x60, 0x3A, 0xD4, 0x9E, 0x52, 0xC7, 0x08,
];

pub fn encrypt_payload(plaintext: &[u8]) -> Vec<u8> {
    let key = Key::<Aes256Gcm>::from_slice(AES_KEY);
    let cipher = Aes256Gcm::new(key);

    // Random 12-byte nonce
    let mut nonce_bytes = [0u8; 12];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher.encrypt(nonce, plaintext)
        .expect("AES-GCM encryption failed");

    // Format: [12-byte nonce] + [ciphertext + 16-byte auth tag]
    let mut out = Vec::with_capacity(12 + ciphertext.len());
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ciphertext);
    out
}

pub fn decrypt_payload(data: &[u8]) -> Option<Vec<u8>> {
    if data.len() < 12 + 16 { return None; } // nonce + min auth tag

    let key = Key::<Aes256Gcm>::from_slice(AES_KEY);
    let cipher = Aes256Gcm::new(key);

    let nonce = Nonce::from_slice(&data[..12]);
    let ciphertext = &data[12..];

    cipher.decrypt(nonce, ciphertext).ok()
}
